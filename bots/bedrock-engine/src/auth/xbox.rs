use reqwest::Method;
use serde::Deserialize;
use serde_json::json;

use crate::{auth::XboxIdentity, db::Database, diagnostics::Diagnostics, error::EngineResult};

#[derive(Clone)]
pub struct XboxAuth {
    client: reqwest::Client,
    diagnostics: Diagnostics,
}

impl XboxAuth {
    pub fn new(db: Database) -> Self {
        Self {
            client: reqwest::Client::new(),
            diagnostics: Diagnostics::new(db),
        }
    }

    pub async fn authenticate(
        &self,
        account_id: &str,
        microsoft_access_token: &str,
    ) -> EngineResult<XboxIdentity> {
        let body = json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={microsoft_access_token}")
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        });
        let (response, _, _) = self
            .diagnostics
            .request_json::<XboxAuthResponse>(
                &self.client,
                Some(account_id),
                "xbox_live_authentication",
                Method::POST,
                "https://user.auth.xboxlive.com/user/authenticate",
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
        Ok(XboxIdentity {
            token: response.token,
            user_hash: xui.uhs.unwrap_or_default(),
            xuid: xui.xid,
            gamertag: xui.gtg,
        })
    }
}

#[derive(Debug, Deserialize)]
struct XboxAuthResponse {
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
