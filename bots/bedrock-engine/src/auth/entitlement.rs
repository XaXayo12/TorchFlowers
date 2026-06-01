use reqwest::Method;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    auth::{
        ProvisionedBedrockSession,
        microsoft::MicrosoftTokenResponse,
        minecraft::MinecraftAuth,
        playfab::PlayFabAuth,
        token_manager::TokenManager,
        xbox::XboxAuth,
        xsts::{BEDROCK_RELYING_PARTY, PLAYFAB_RELYING_PARTY, XstsAuth},
    },
    config::Config,
    db::{Database, NewLogEntry},
    diagnostics::Diagnostics,
    error::{EngineError, EngineResult},
};

#[derive(Clone)]
pub struct EntitlementProvisioner {
    db: Database,
    token_manager: TokenManager,
    xbox: XboxAuth,
    xsts: XstsAuth,
    playfab: PlayFabAuth,
    minecraft: MinecraftAuth,
    diagnostics: Diagnostics,
    client: reqwest::Client,
}

impl EntitlementProvisioner {
    pub fn new(config: &Config, db: Database) -> Self {
        Self {
            token_manager: TokenManager::new(config, db.clone()),
            xbox: XboxAuth::new(db.clone()),
            xsts: XstsAuth::new(db.clone()),
            playfab: PlayFabAuth::new(db.clone()),
            minecraft: MinecraftAuth::new(db.clone()),
            diagnostics: Diagnostics::new(db.clone()),
            client: reqwest::Client::new(),
            db,
        }
    }

    pub async fn save_microsoft_tokens(
        &self,
        account_id: &str,
        tokens: &MicrosoftTokenResponse,
    ) -> EngineResult<()> {
        self.token_manager
            .save_microsoft_tokens(
                account_id,
                &tokens.access_token,
                &tokens.refresh_token,
                tokens.expires_in,
            )
            .await
    }

    pub async fn provision(&self, account_id: &str) -> EngineResult<ProvisionedBedrockSession> {
        self.diagnostics
            .log_event(
                Some(account_id),
                None,
                "info",
                "auth",
                Some("provisioning_start"),
                "starting complete Bedrock entitlement provisioning flow",
                json!({ "sequence": [
                    "microsoft_oauth",
                    "xbox_live",
                    "xsts_bedrock",
                    "xsts_playfab",
                    "playfab_login",
                    "minecraft_session_start",
                    "legacy_bedrock_authentication",
                    "jwt_chain_generation"
                ] }),
            )
            .await?;

        let access_token = self
            .token_manager
            .valid_microsoft_access_token(account_id)
            .await?;
        self.db
            .update_account_status(account_id, "microsoft_status", "authenticated", None)
            .await?;

        let xbox = self.xbox.authenticate(account_id, &access_token).await?;
        self.db
            .update_account_status(account_id, "xbox_status", "authenticated", None)
            .await?;

        let standard_xsts = self
            .xsts
            .authorize(
                account_id,
                &xbox.token,
                BEDROCK_RELYING_PARTY,
                "xsts_bedrock_authorization",
            )
            .await?;
        let playfab_xsts = self
            .xsts
            .authorize(
                account_id,
                &xbox.token,
                PLAYFAB_RELYING_PARTY,
                "xsts_playfab_authorization",
            )
            .await?;
        self.db
            .update_account_status(account_id, "xsts_status", "authenticated", None)
            .await?;
        self.db
            .set_account_profile(
                account_id,
                standard_xsts
                    .gamertag
                    .as_deref()
                    .or(xbox.gamertag.as_deref()),
                standard_xsts.xuid.as_deref().or(xbox.xuid.as_deref()),
            )
            .await?;

        let playfab = self
            .playfab
            .login_with_xbox(account_id, &playfab_xsts)
            .await?;
        self.db
            .update_account_status(account_id, "playfab_status", "authenticated", None)
            .await?;

        let minecraft_token = self
            .start_minecraft_entitlement_session(account_id, &playfab.session_ticket)
            .await?;
        let entitlement_exists = self
            .detect_entitlement(account_id, &minecraft_token)
            .await
            .unwrap_or(true);
        self.db
            .update_account_status(account_id, "entitlement_status", "provisioned", None)
            .await?;

        let (signing_key, private_key_pem, public_key) = MinecraftAuth::generate_device_keypair()?;
        let legacy_chain = self
            .minecraft
            .legacy_bedrock_auth(account_id, &standard_xsts, &public_key)
            .await?;
        self.db
            .update_account_status(account_id, "bedrock_auth_status", "authenticated", None)
            .await?;

        let display_name = standard_xsts
            .gamertag
            .as_deref()
            .or(xbox.gamertag.as_deref())
            .unwrap_or("RustRockBot");
        let xuid = standard_xsts
            .xuid
            .as_deref()
            .or(xbox.xuid.as_deref())
            .unwrap_or_default();
        let chain = MinecraftAuth::build_jwt_chain(
            legacy_chain.clone(),
            signing_key,
            private_key_pem,
            public_key,
            display_name,
            xuid,
        )?;
        self.db
            .upsert_entitlement(
                account_id,
                entitlement_exists,
                Some(&playfab.playfab_id),
                Some(&self.token_manager.encrypt(&playfab.session_ticket)?),
                Some(&self.token_manager.encrypt(&minecraft_token)?),
                "provisioned",
                None,
                None,
            )
            .await?;

        Ok(ProvisionedBedrockSession {
            account_id: account_id.to_string(),
            playfab_id: playfab.playfab_id,
            playfab_session_ticket: playfab.session_ticket,
            minecraft_access_token: minecraft_token,
            legacy_bedrock_token: legacy_chain.join("."),
            chain,
        })
    }

    async fn start_minecraft_entitlement_session(
        &self,
        account_id: &str,
        session_ticket: &str,
    ) -> EngineResult<String> {
        let body = json!({
            "PlayFabSessionTicket": session_ticket,
            "SessionTicket": session_ticket,
            "CreateAccount": true,
            "Device": {
                "ApplicationType": "MinecraftPE",
                "GameVersion": "1.21.100"
            }
        });
        let (response, _, _) = self
            .diagnostics
            .request_json::<Value>(
                &self.client,
                Some(account_id),
                "minecraft_entitlement_session_start",
                Method::POST,
                "https://authorization.franchise.minecraft-services.net/api/v1.0/session/start",
                vec![
                    ("Authorization", format!("PlayFab {session_ticket}")),
                    ("User-Agent", "MCPE/Android".to_string()),
                    ("Client-Version", "1.21.100".to_string()),
                ],
                body,
            )
            .await?;
        token_from_session_start(&response).ok_or_else(|| EngineError::Auth {
            step: "minecraft_entitlement_session_start",
            message: format!("session/start response did not contain a token: {response}"),
        })
    }

    async fn detect_entitlement(
        &self,
        account_id: &str,
        minecraft_token: &str,
    ) -> EngineResult<bool> {
        let request_id = Uuid::new_v4().to_string();
        let url = format!(
            "https://api.minecraftservices.com/entitlements/license?requestId={request_id}"
        );
        let response = self
            .client
            .get(&url)
            .bearer_auth(minecraft_token)
            .header("Content-Type", "application/json")
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        self.db
            .log(NewLogEntry {
                account_id: Some(account_id),
                bot_id: None,
                level: if status.is_success() { "info" } else { "error" },
                category: "auth_http",
                step: Some("minecraft_entitlement_detect"),
                request_id: Some(&request_id),
                method: Some("GET"),
                url: Some(&url),
                status_code: Some(status.as_u16() as i64),
                request_body: None,
                response_body: Some(&body),
                message: "checked Minecraft entitlement license",
                metadata_json: Some("{}"),
            })
            .await?;
        if !status.is_success() {
            return Err(EngineError::Auth {
                step: "minecraft_entitlement_detect",
                message: body,
            });
        }
        let value: Value = serde_json::from_str(&body)?;
        Ok(value
            .get("items")
            .and_then(|items| items.as_array())
            .is_some_and(|items| !items.is_empty()))
    }
}

fn token_from_session_start(value: &Value) -> Option<String> {
    for path in [
        "/access_token",
        "/AccessToken",
        "/token",
        "/Token",
        "/result/token",
        "/result/authorizationToken",
        "/data/token",
    ] {
        if let Some(token) = value.pointer(path).and_then(Value::as_str) {
            return Some(token.to_string());
        }
    }
    if value.is_object() {
        return Some(value.to_string());
    }
    None
}
