use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use p384::{
    ecdsa::SigningKey,
    elliptic_curve::Generate,
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::{
    auth::XstsToken,
    db::Database,
    diagnostics::Diagnostics,
    error::{EngineError, EngineResult},
};

#[derive(Clone)]
pub struct MinecraftAuth {
    client: reqwest::Client,
    diagnostics: Diagnostics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BedrockJwtChain {
    pub chain: Vec<String>,
    pub skin: String,
    pub public_key_der_base64: String,
    pub private_key_pem: String,
}

#[derive(Debug, Deserialize)]
struct LegacyAuthResponse {
    chain: Vec<String>,
}

impl MinecraftAuth {
    pub fn new(db: Database) -> Self {
        Self {
            client: reqwest::Client::new(),
            diagnostics: Diagnostics::new(db),
        }
    }

    pub async fn legacy_bedrock_auth(
        &self,
        account_id: &str,
        standard_xsts: &XstsToken,
        identity_public_key: &str,
    ) -> EngineResult<Vec<String>> {
        let body = json!({ "identityPublicKey": identity_public_key });
        let authorization = format!(
            "XBL3.0 x={};{}",
            standard_xsts.user_hash, standard_xsts.token
        );
        let (response, _, _) = self
            .diagnostics
            .request_json::<LegacyAuthResponse>(
                &self.client,
                Some(account_id),
                "legacy_bedrock_authentication",
                Method::POST,
                "https://multiplayer.minecraft.net/authentication",
                vec![
                    ("Authorization", authorization),
                    ("User-Agent", "MCPE/Android".to_string()),
                    ("Client-Version", "1.21.100".to_string()),
                ],
                body,
            )
            .await?;
        Ok(response.chain)
    }

    pub fn generate_device_keypair() -> EngineResult<(SigningKey, String, String)> {
        let signing_key = SigningKey::generate();
        let private_pem = signing_key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|err| EngineError::Crypto(err.to_string()))?
            .to_string();
        let public_der = signing_key
            .verifying_key()
            .to_public_key_der()
            .map_err(|err| EngineError::Crypto(err.to_string()))?;
        let public_der_base64 = STANDARD.encode(public_der.as_bytes());
        Ok((signing_key, private_pem, public_der_base64))
    }

    pub fn build_jwt_chain(
        legacy_chain: Vec<String>,
        signing_key: SigningKey,
        private_key_pem: String,
        public_key_der_base64: String,
        display_name: &str,
        xuid: &str,
    ) -> EngineResult<BedrockJwtChain> {
        let encoding_key = EncodingKey::from_ec_pem(private_key_pem.as_bytes())
            .map_err(|err| EngineError::Crypto(err.to_string()))?;
        let now = Utc::now();
        let exp = now + Duration::hours(24);
        let identity = Uuid::new_v4().to_string();
        let client_claims = json!({
            "exp": exp.timestamp(),
            "nbf": now.timestamp(),
            "extraData": {
                "displayName": display_name,
                "identity": identity,
                "XUID": xuid,
                "titleId": "896928775"
            },
            "identityPublicKey": public_key_der_base64
        });
        let mut header = Header::new(Algorithm::ES384);
        header.x5u = Some(public_key_der_base64.clone());
        let client_jwt = encode(&header, &client_claims, &encoding_key)
            .map_err(|err| EngineError::Crypto(err.to_string()))?;

        let skin_claims = json!({
            "exp": exp.timestamp(),
            "nbf": now.timestamp(),
            "ClientRandomId": rand::random::<i64>(),
            "CurrentInputMode": 1,
            "DefaultInputMode": 1,
            "DeviceModel": "RustRock",
            "DeviceOS": 7,
            "GameVersion": "1.21.100",
            "GuiScale": 0,
            "LanguageCode": "en_US",
            "PersonaSkin": false,
            "PlatformOfflineId": "",
            "PlatformOnlineId": "",
            "PlayFabId": "",
            "SelfSignedId": Uuid::new_v4().to_string(),
            "ServerAddress": "",
            "SkinAnimationData": "",
            "SkinColor": "#0",
            "SkinData": "",
            "SkinGeometryData": "{\"geometry\":{\"default\":\"geometry.humanoid.custom\"}}",
            "SkinId": "rustrock-default",
            "SkinImageHeight": 64,
            "SkinImageWidth": 64,
            "SkinResourcePatch": "{\"geometry\":{\"default\":\"geometry.humanoid.custom\"}}",
            "ThirdPartyName": display_name,
            "ThirdPartyNameOnly": false,
            "TrustedSkin": true
        });
        let skin = encode(&header, &skin_claims, &encoding_key)
            .map_err(|err| EngineError::Crypto(err.to_string()))?;

        let mut chain = legacy_chain;
        chain.push(client_jwt);
        let _ = signing_key;
        Ok(BedrockJwtChain {
            chain,
            skin,
            public_key_der_base64,
            private_key_pem,
        })
    }

    pub fn connection_request(chain: &BedrockJwtChain) -> EngineResult<Vec<u8>> {
        Ok(serde_json::to_vec(&json!({
            "chain": chain.chain,
            "skin": chain.skin
        }))?)
    }
}
