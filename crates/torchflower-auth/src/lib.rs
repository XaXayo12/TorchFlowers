#![allow(unknown_lints)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type AccountId = String;

pub const LIVE_CLIENT_ID: &str = "00000000402b5328";
pub const LIVE_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";
pub const BEDROCK_RELYING_PARTY: &str = "https://multiplayer.minecraft.net/";
pub const PLAYFAB_RELYING_PARTY: &str = "http://playfab.xboxlive.com/";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthFlow {
    Live,
    Msal,
    #[cfg(feature = "offline-mode")]
    Offline {
        username: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthConfig {
    pub client_id: String,
    pub flow: AuthFlow,
}

impl AuthConfig {
    pub fn device_code() -> Self {
        Self {
            client_id: LIVE_CLIENT_ID.to_string(),
            flow: AuthFlow::Live,
        }
    }

    #[cfg(feature = "offline-mode")]
    pub fn offline(username: &str) -> Self {
        Self {
            client_id: String::new(),
            flow: AuthFlow::Offline {
                username: username.to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthTokens {
    pub microsoft_access_token: Option<String>,
    pub microsoft_refresh_token: Option<String>,
    pub microsoft_expires_at: Option<DateTime<Utc>>,
    pub xbox_token: Option<String>,
    pub xsts_token: Option<String>,
    pub playfab_xsts_token: Option<String>,
    pub playfab_session_ticket: Option<String>,
    pub minecraft_access_token: Option<String>,
    pub legacy_bedrock_token: Option<String>,
    pub bedrock_chain: Vec<String>,
    pub xuid: Option<String>,
    pub gamertag: Option<String>,
    pub playfab_id: Option<String>,
}

impl AuthTokens {
    pub fn empty() -> Self {
        Self {
            microsoft_access_token: None,
            microsoft_refresh_token: None,
            microsoft_expires_at: None,
            xbox_token: None,
            xsts_token: None,
            playfab_xsts_token: None,
            playfab_session_ticket: None,
            minecraft_access_token: None,
            legacy_bedrock_token: None,
            bedrock_chain: Vec::new(),
            xuid: None,
            gamertag: None,
            playfab_id: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("offline auth is enabled only with the offline-mode feature")]
    OfflineModeDisabled,
    #[error(
        "interactive device-code authentication must be driven by the engine or an auth client"
    )]
    InteractiveFlowRequired,
    #[error("refresh token is missing")]
    MissingRefreshToken,
    #[error("HTTP auth request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("auth response parse failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub async fn authenticate(config: &AuthConfig) -> Result<AuthTokens, AuthError> {
    match &config.flow {
        AuthFlow::Live | AuthFlow::Msal => Err(AuthError::InteractiveFlowRequired),
        #[cfg(feature = "offline-mode")]
        AuthFlow::Offline { username } => {
            let mut tokens = AuthTokens::empty();
            tokens.gamertag = Some(username.clone());
            tokens.bedrock_chain = vec![format!("offline:{username}")];
            Ok(tokens)
        }
    }
}

pub async fn refresh(tokens: &AuthTokens) -> Result<AuthTokens, AuthError> {
    if tokens.microsoft_refresh_token.is_none() {
        return Err(AuthError::MissingRefreshToken);
    }
    Err(AuthError::InteractiveFlowRequired)
}

pub async fn batch_authenticate(
    accounts: &[AccountId],
    config: &AuthConfig,
    concurrency: usize,
) -> HashMap<AccountId, AuthTokens> {
    let concurrency = concurrency.max(1);
    let mut authenticated = HashMap::new();

    for chunk in accounts.chunks(concurrency) {
        let mut tasks = Vec::with_capacity(chunk.len());
        for account in chunk {
            let account = account.clone();
            let config = config.clone();
            tasks.push(tokio::spawn(async move {
                authenticate(&config).await.map(|tokens| (account, tokens))
            }));
        }

        for task in tasks {
            if let Ok(Ok((account, tokens))) = task.await {
                authenticated.insert(account, tokens);
            }
        }
    }

    authenticated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip_through_json() {
        let mut tokens = AuthTokens::empty();
        tokens.microsoft_refresh_token = Some("refresh".to_string());
        tokens.playfab_session_ticket = Some("ticket".to_string());
        tokens.bedrock_chain = vec!["chain".to_string()];

        let encoded = serde_json::to_string(&tokens).unwrap();
        let decoded: AuthTokens = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, tokens);
    }
}
