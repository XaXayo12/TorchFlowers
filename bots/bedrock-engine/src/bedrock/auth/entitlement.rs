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

const MAX_PROVISIONING_ATTEMPTS: usize = 3;

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
            xbox: XboxAuth::new(config, db.clone()),
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
                    "minecraft_multiplayer_session_start",
                    "jwt_chain_generation"
                ] }),
            )
            .await?;

        for attempt in 1..=MAX_PROVISIONING_ATTEMPTS {
            self.db.begin_entitlement_provisioning(account_id).await?;
            self.db
                .update_account_status(account_id, "entitlement_status", "pending", None)
                .await?;
            self.diagnostics
                .log_event(
                    Some(account_id),
                    None,
                    "info",
                    "auth",
                    Some("provisioning_attempt_start"),
                    "running Bedrock entitlement provisioning attempt",
                    json!({
                        "attempt": attempt,
                        "max_attempts": MAX_PROVISIONING_ATTEMPTS
                    }),
                )
                .await?;

            match self.provision_once(account_id).await {
                Ok(session) => {
                    self.diagnostics
                        .log_event(
                            Some(account_id),
                            None,
                            "info",
                            "auth",
                            Some("provisioning_complete"),
                            "completed Bedrock entitlement provisioning flow",
                            json!({
                                "attempt": attempt,
                                "max_attempts": MAX_PROVISIONING_ATTEMPTS
                            }),
                        )
                        .await?;
                    return Ok(session);
                }
                Err(error) => {
                    let message = error.to_string();
                    let retry = self
                        .db
                        .record_entitlement_provisioning_failure(account_id, &message)
                        .await?;
                    self.db
                        .update_account_status(
                            account_id,
                            "entitlement_status",
                            "failed",
                            Some(&message),
                        )
                        .await?;
                    self.diagnostics
                        .log_event(
                            Some(account_id),
                            None,
                            if attempt == MAX_PROVISIONING_ATTEMPTS {
                                "error"
                            } else {
                                "warn"
                            },
                            "auth",
                            Some("provisioning_attempt_failed"),
                            "Bedrock entitlement provisioning attempt failed",
                            json!({
                                "attempt": attempt,
                                "max_attempts": MAX_PROVISIONING_ATTEMPTS,
                                "retry_count": retry.retry_count,
                                "next_retry_at": retry.next_retry_at,
                                "error": message
                            }),
                        )
                        .await?;
                    if attempt == MAX_PROVISIONING_ATTEMPTS {
                        return Err(error);
                    }
                }
            }
        }

        Err(EngineError::Auth {
            step: "provisioning",
            message: "provisioning loop exited without result".to_string(),
        })
    }

    async fn provision_once(&self, account_id: &str) -> EngineResult<ProvisionedBedrockSession> {
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
                &xbox,
                BEDROCK_RELYING_PARTY,
                "xsts_bedrock_authorization",
            )
            .await?;
        let playfab_xsts = self
            .xsts
            .authorize(
                account_id,
                &xbox,
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
        let license_detected = match self.detect_entitlement(account_id, &minecraft_token).await {
            Ok(exists) => exists,
            Err(error) => {
                self.diagnostics
                    .log_event(
                        Some(account_id),
                        None,
                        "warn",
                        "auth",
                        Some("minecraft_entitlement_detect"),
                        "Minecraft entitlement detection failed after provisioning",
                        json!({ "error": error.to_string() }),
                    )
                    .await?;
                false
            }
        };
        self.diagnostics
            .log_event(
                Some(account_id),
                None,
                "info",
                "auth",
                Some("minecraft_entitlement_detect"),
                "recorded Minecraft license detection result",
                json!({ "license_detected": license_detected }),
            )
            .await?;
        self.db
            .update_account_status(account_id, "entitlement_status", "provisioned", None)
            .await?;

        let (signing_key, private_key_pem, public_key) = MinecraftAuth::generate_device_keypair()?;
        let legacy_auth = self
            .minecraft
            .legacy_bedrock_auth(account_id, &standard_xsts, &public_key)
            .await?;
        let bedrock_login_token = self
            .start_minecraft_multiplayer_session(account_id, &minecraft_token, &public_key)
            .await?;
        self.db
            .update_account_status(account_id, "bedrock_auth_status", "authenticated", None)
            .await?;

        let display_name = standard_xsts
            .gamertag
            .as_deref()
            .or(xbox.gamertag.as_deref())
            .unwrap_or("TorchFlowerBot");
        let xuid = standard_xsts
            .xuid
            .as_deref()
            .or(xbox.xuid.as_deref())
            .unwrap_or_default();
        let chain = MinecraftAuth::build_jwt_chain(
            legacy_auth.chain.clone(),
            signing_key,
            private_key_pem,
            public_key,
            display_name,
            xuid,
            Some(&playfab.playfab_id),
        )?;
        self.db
            .upsert_entitlement(
                account_id,
                true,
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
            bedrock_login_token,
            legacy_bedrock_token: legacy_auth.token,
            chain,
        })
    }

    async fn start_minecraft_entitlement_session(
        &self,
        account_id: &str,
        session_ticket: &str,
    ) -> EngineResult<String> {
        let body = json!({
            "user": {
                "token": session_ticket,
                "tokenType": "PlayFab"
            },
            "device": {
                "applicationType": "MinecraftPE",
                "gameVersion": "1.20.62",
                "id": Uuid::new_v4().to_string(),
                "type": "Windows10",
                "memory": "17179869184",
                "platform": "Windows10",
                "storePlatform": "uwp.store",
                "playFabTitleId": "20CA2"
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
                vec![],
                body,
            )
            .await?;
        token_from_session_start(&response).ok_or_else(|| EngineError::Auth {
            step: "minecraft_entitlement_session_start",
            message: format!("session/start response did not contain a token: {response}"),
        })
    }

    async fn start_minecraft_multiplayer_session(
        &self,
        account_id: &str,
        minecraft_services_token: &str,
        public_key: &str,
    ) -> EngineResult<String> {
        let body = json!({ "publicKey": public_key });
        let (response, _, _) = self
            .diagnostics
            .request_json::<Value>(
                &self.client,
                Some(account_id),
                "minecraft_multiplayer_session_start",
                Method::POST,
                "https://authorization.franchise.minecraft-services.net/api/v1.0/multiplayer/session/start",
                vec![
                    ("accept", "*/*".to_string()),
                    ("authorization", minecraft_services_token.to_string()),
                    ("User-Agent", "libhttpclient/1.0.0.0".to_string()),
                    ("Accept-Language", "en-US".to_string()),
                ],
                body,
            )
            .await?;

        token_from_multiplayer_session_start(&response).ok_or_else(|| EngineError::Auth {
            step: "minecraft_multiplayer_session_start",
            message: format!(
                "multiplayer/session/start response did not contain signedToken: {response}"
            ),
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
        "/result/authorizationHeader",
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
    None
}

fn token_from_multiplayer_session_start(value: &Value) -> Option<String> {
    for path in [
        "/result/signedToken",
        "/signedToken",
        "/result/token",
        "/token",
        "/result/authorizationHeader",
    ] {
        if let Some(token) = value.pointer(path).and_then(Value::as_str) {
            return Some(token.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{token_from_multiplayer_session_start, token_from_session_start};
    use serde_json::json;

    #[test]
    fn session_start_token_prefers_authorization_header() {
        let response = json!({
            "result": {
                "authorizationHeader": "MCToken services-token",
                "authorizationToken": "legacy-shape-token"
            }
        });

        assert_eq!(
            token_from_session_start(&response).as_deref(),
            Some("MCToken services-token")
        );
    }

    #[test]
    fn session_start_token_does_not_return_entire_response_object() {
        let response = json!({ "result": { "validUntil": "2026-06-02T00:00:00Z" } });

        assert!(token_from_session_start(&response).is_none());
    }

    #[test]
    fn multiplayer_session_token_reads_signed_token() {
        let response = json!({ "result": { "signedToken": "signed-login-token" } });

        assert_eq!(
            token_from_multiplayer_session_start(&response).as_deref(),
            Some("signed-login-token")
        );
    }
}
