use reqwest::Method;
use serde::Deserialize;
use serde_json::json;

use crate::{auth::XstsToken, db::Database, diagnostics::Diagnostics, error::EngineResult};

pub const BEDROCK_RELYING_PARTY: &str = "https://multiplayer.minecraft.net/";
pub const PLAYFAB_RELYING_PARTY: &str = "http://playfab.xboxlive.com/";

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
        xbox_user_token: &str,
        relying_party: &str,
        step: &'static str,
    ) -> EngineResult<XstsToken> {
        let body = json!({
            "Properties": {
                "SandboxId": "RETAIL",
                "UserTokens": [xbox_user_token]
            },
            "RelyingParty": relying_party,
            "TokenType": "JWT"
        });
        let (response, _, _) = self
            .diagnostics
            .request_json::<XstsAuthResponse>(
                &self.client,
                Some(account_id),
                step,
                Method::POST,
                "https://xsts.auth.xboxlive.com/xsts/authorize",
                vec![("x-xbl-contract-version", "1".to_string())],
                body,
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
