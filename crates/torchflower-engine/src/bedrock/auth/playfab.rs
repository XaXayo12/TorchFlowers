use reqwest::Method;
use serde::Deserialize;
use serde_json::json;

use crate::{auth::XstsToken, db::Database, diagnostics::Diagnostics, error::EngineResult};

#[derive(Clone)]
pub struct PlayFabAuth {
    client: reqwest::Client,
    diagnostics: Diagnostics,
}

#[derive(Debug, Clone)]
pub struct PlayFabSession {
    pub playfab_id: String,
    pub session_ticket: String,
}

impl PlayFabAuth {
    pub fn new(db: Database) -> Self {
        Self {
            client: reqwest::Client::new(),
            diagnostics: Diagnostics::new(db),
        }
    }

    pub async fn login_with_xbox(
        &self,
        account_id: &str,
        playfab_xsts: &XstsToken,
    ) -> EngineResult<PlayFabSession> {
        let xbox_token = format!("XBL3.0 x={};{}", playfab_xsts.user_hash, playfab_xsts.token);
        let body = json!({
            "CreateAccount": true,
            "TitleId": "20CA2",
            "XboxToken": xbox_token,
            "InfoRequestParameters": {
                "GetUserAccountInfo": true,
                "GetPlayerProfile": true
            }
        });
        let (response, _, _) = self
            .diagnostics
            .request_json::<PlayFabResponse>(
                &self.client,
                Some(account_id),
                "playfab_login_with_xbox",
                Method::POST,
                "https://20ca2.playfabapi.com/Client/LoginWithXbox",
                vec![("X-PlayFabSDK", "TorchFlowerEngine/0.1.0".to_string())],
                body,
            )
            .await?;
        Ok(PlayFabSession {
            playfab_id: response.data.playfab_id,
            session_ticket: response.data.session_ticket,
        })
    }
}

#[derive(Debug, Deserialize)]
struct PlayFabResponse {
    data: PlayFabData,
}

#[derive(Debug, Deserialize)]
struct PlayFabData {
    #[serde(rename = "PlayFabId")]
    playfab_id: String,
    #[serde(rename = "SessionTicket")]
    session_ticket: String,
}
