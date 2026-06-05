use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{
    auth::{XboxIdentity, XstsToken},
    db::Database,
    diagnostics::Diagnostics,
    error::EngineResult,
};

pub const BEDROCK_RELYING_PARTY: &str = "https://multiplayer.minecraft.net/";
pub const PLAYFAB_RELYING_PARTY: &str = "http://playfab.xboxlive.com/";
const XSTS_AUTHORIZE_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";

#[derive(Clone)]
pub struct XstsAuth {
    client: reqwest::Client,
    diagnostics: Diagnostics,
}

impl XstsAuth {
    pub fn new(db: Database) -> Self {
        Self {
            client: reqwest::Client::new(),
            diagnostics: Diagnostics::new(db),
        }
    }

    pub async fn authorize(
        &self,
        account_id: &str,
        xbox: &XboxIdentity,
        relying_party: &str,
        step: &'static str,
    ) -> EngineResult<XstsToken> {
        let mut properties = Map::new();
        properties.insert("SandboxId".to_string(), json!("RETAIL"));
        properties.insert("UserTokens".to_string(), json!([xbox.token]));
        if let Some(device_token) = &xbox.device_token {
            properties.insert("DeviceToken".to_string(), json!(device_token));
        }
        if let Some(title_token) = &xbox.title_token {
            properties.insert("TitleToken".to_string(), json!(title_token));
        }
        if let Some(proof_key) = &xbox.proof_key {
            properties.insert("ProofKey".to_string(), proof_key.jwk());
        }

        let body = json!({
            "Properties": Value::Object(properties),
            "RelyingParty": relying_party,
            "TokenType": "JWT"
        });
        let body_text = body.to_string();
        let mut headers = vec![
            (
                "Cache-Control",
                "no-store, must-revalidate, no-cache".to_string(),
            ),
            ("x-xbl-contract-version", "1".to_string()),
        ];
        if let Some(proof_key) = &xbox.proof_key {
            headers.push((
                "Signature",
                proof_key.signature_header(XSTS_AUTHORIZE_URL, "", &body_text),
            ));
        }
        let (response, _, _) = self
            .diagnostics
            .request_json_text::<XstsAuthResponse>(
                &self.client,
                Some(account_id),
                step,
                Method::POST,
                XSTS_AUTHORIZE_URL,
                headers,
                body_text,
            )
            .await?;
        let xui = response
            .display_claims
            .xui
            .first()
            .cloned()
            .unwrap_or_default();
        Ok(XstsToken {
            token: response.token,
            user_hash: xui.uhs.unwrap_or_default(),
            xuid: xui.xid,
            gamertag: xui.gtg,
            relying_party: relying_party.to_string(),
        })
    }
}

#[derive(Debug, Deserialize)]
struct XstsAuthResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Debug, Deserialize)]
struct DisplayClaims {
    xui: Vec<Xui>,
}

#[derive(Debug, Deserialize, Clone, Default)]
struct Xui {
    uhs: Option<String>,
    xid: Option<String>,
    gtg: Option<String>,
}
