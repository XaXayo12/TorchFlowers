use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, OsRng, rand_core::RngCore},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    config::Config,
    db::Database,
    diagnostics::encode_form,
    error::{EngineError, EngineResult},
};

#[derive(Clone)]
pub struct TokenManager {
    db: Database,
    client: reqwest::Client,
    client_id: String,
    auth_flow: crate::config::MicrosoftAuthFlow,
    cipher: Aes256Gcm,
}

#[derive(Debug, Clone)]
pub struct StoredMicrosoftTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: Option<DateTime<Utc>>,
}

impl TokenManager {
    pub fn new(config: &Config, db: Database) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(config.token_encryption_secret.as_bytes());
        let key = hasher.finalize();
        let cipher = Aes256Gcm::new_from_slice(&key).expect("sha256 key length is always valid");
        Self {
            db,
            client: reqwest::Client::new(),
            client_id: config.microsoft_client_id.clone(),
            auth_flow: config.microsoft_auth_flow.clone(),
            cipher,
        }
    }

    pub fn encrypt(&self, plaintext: &str) -> EngineResult<String> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|err| EngineError::Crypto(err.to_string()))?;
        Ok(format!(
            "v1:{}:{}",
            URL_SAFE_NO_PAD.encode(nonce_bytes),
            URL_SAFE_NO_PAD.encode(ciphertext)
        ))
    }

    pub fn decrypt(&self, encoded: &str) -> EngineResult<String> {
        let mut parts = encoded.split(':');
        match (parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some("v1"), Some(nonce), Some(ciphertext), None) => {
                let nonce = URL_SAFE_NO_PAD
                    .decode(nonce)
                    .map_err(|err| EngineError::Crypto(err.to_string()))?;
                let ciphertext = URL_SAFE_NO_PAD
                    .decode(ciphertext)
                    .map_err(|err| EngineError::Crypto(err.to_string()))?;
                let plaintext = self
                    .cipher
                    .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
                    .map_err(|err| EngineError::Crypto(err.to_string()))?;
                String::from_utf8(plaintext).map_err(|err| EngineError::Crypto(err.to_string()))
            }
            _ => Err(EngineError::Crypto(
                "unsupported ciphertext envelope".to_string(),
            )),
        }
    }

    pub async fn load_microsoft_tokens(
        &self,
        account_id: &str,
    ) -> EngineResult<StoredMicrosoftTokens> {
        let (access, refresh, expires_at) = self.db.token_ciphertexts(account_id).await?;
        let expires_at = expires_at
            .and_then(|v| DateTime::parse_from_rfc3339(&v).ok())
            .map(|v| v.with_timezone(&Utc));
        Ok(StoredMicrosoftTokens {
            access_token: self.decrypt(&access)?,
            refresh_token: self.decrypt(&refresh)?,
            expires_at,
        })
    }

    pub async fn save_microsoft_tokens(
        &self,
        account_id: &str,
        access_token: &str,
        refresh_token: &str,
        expires_in: i64,
    ) -> EngineResult<()> {
        let expires_at = Utc::now() + Duration::seconds(expires_in.max(60) - 30);
        self.db
            .save_tokens(
                account_id,
                &self.encrypt(access_token)?,
                &self.encrypt(refresh_token)?,
                &expires_at.to_rfc3339(),
            )
            .await
    }

    pub async fn valid_microsoft_access_token(&self, account_id: &str) -> EngineResult<String> {
        let tokens = self.load_microsoft_tokens(account_id).await?;
        if tokens
            .expires_at
            .is_some_and(|expires| expires > Utc::now() + Duration::seconds(60))
        {
            return Ok(tokens.access_token);
        }
        let refreshed = self.refresh_microsoft(&tokens.refresh_token).await?;
        self.save_microsoft_tokens(
            account_id,
            &refreshed.access_token,
            refreshed
                .refresh_token
                .as_deref()
                .unwrap_or(tokens.refresh_token.as_str()),
            refreshed.expires_in,
        )
        .await?;
        Ok(refreshed.access_token)
    }

    async fn refresh_microsoft(&self, refresh_token: &str) -> EngineResult<RefreshResponse> {
        let form = vec![
            ("client_id", self.client_id.clone()),
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.to_string()),
            ("scope", self.auth_flow.scope().to_string()),
        ];
        let response = self
            .client
            .post(self.auth_flow.token_url())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(encode_form(&form))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(EngineError::Auth {
                step: "microsoft_refresh",
                message: body,
            });
        }
        Ok(serde_json::from_str(&body)?)
    }
}

#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: i64,
}
