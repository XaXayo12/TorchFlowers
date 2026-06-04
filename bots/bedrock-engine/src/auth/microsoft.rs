use chrono::{Duration, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::{
    config::{Config, MicrosoftAuthFlow},
    db::Database,
    diagnostics::Diagnostics,
    error::{EngineError, EngineResult},
    models::DeviceAuthSession,
};

#[derive(Clone)]
pub struct MicrosoftAuth {
    client: reqwest::Client,
    db: Database,
    diagnostics: Diagnostics,
    client_id: String,
    auth_flow: MicrosoftAuthFlow,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: i64,
    pub interval: Option<i64>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MicrosoftTokenResponse {
    pub token_type: String,
    pub scope: String,
    pub expires_in: i64,
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MicrosoftTokenError {
    pub error: String,
    pub error_description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TokenPollResponse {
    Success(MicrosoftTokenResponse),
    Pending(MicrosoftTokenError),
}

impl MicrosoftAuth {
    pub fn new(config: &Config, db: Database) -> Self {
        Self {
            client: reqwest::Client::new(),
            diagnostics: Diagnostics::new(db.clone()),
            db,
            client_id: config.microsoft_client_id.clone(),
            auth_flow: config.microsoft_auth_flow.clone(),
        }
    }

    pub async fn start_device_auth(&self, email: &str) -> EngineResult<DeviceAuthSession> {
        let account_id = self.db.upsert_account_email(email).await?;
        let (device, _, _) = self
            .diagnostics
            .request_form_json::<DeviceCodeResponse>(
                &self.client,
                Some(&account_id),
                "microsoft_device_code",
                self.auth_flow.device_code_url(),
                self.device_code_form(),
            )
            .await?;
        let expires_at = Utc::now() + Duration::seconds(device.expires_in.max(60));
        let session_id = self
            .db
            .create_auth_session(
                &account_id,
                &device.device_code,
                &device.user_code,
                &device.verification_uri,
                &expires_at.to_rfc3339(),
                device.interval.unwrap_or(5).max(1),
            )
            .await?;
        self.diagnostics
            .log_event(
                Some(&account_id),
                None,
                "info",
                "auth",
                Some("microsoft_device_code"),
                "created Microsoft device-code authentication session",
                serde_json::json!({ "session_id": session_id }),
            )
            .await?;
        Ok(DeviceAuthSession {
            id: session_id,
            account_id,
            user_code: device.user_code,
            verification_uri: device.verification_uri,
            expires_at,
            interval_seconds: device.interval.unwrap_or(5) as u64,
            status: "pending".to_string(),
        })
    }

    pub async fn poll_device_auth(
        &self,
        session_id: &str,
    ) -> EngineResult<Option<MicrosoftTokenResponse>> {
        let (session, device_code) = self.db.get_auth_session_secret(session_id).await?;
        if session.expires_at <= Utc::now() {
            self.db
                .update_auth_session_status(session_id, "expired", Some("device code expired"))
                .await?;
            return Err(EngineError::Auth {
                step: "microsoft_device_token",
                message: "device code expired".to_string(),
            });
        }
        let (poll, _, status) = self
            .diagnostics
            .request_form_json::<TokenPollResponse>(
                &self.client,
                Some(&session.account_id),
                "microsoft_device_token",
                &self.poll_token_url(),
                self.poll_token_form(device_code),
            )
            .await?;
        match poll {
            TokenPollResponse::Success(tokens) if status == StatusCode::OK => {
                self.db
                    .update_auth_session_status(session_id, "authenticated", None)
                    .await?;
                Ok(Some(tokens))
            }
            TokenPollResponse::Pending(error)
                if matches!(error.error.as_str(), "authorization_pending" | "slow_down") =>
            {
                self.db
                    .update_auth_session_status(
                        session_id,
                        "pending",
                        error.error_description.as_deref(),
                    )
                    .await?;
                Ok(None)
            }
            TokenPollResponse::Pending(error) => {
                self.db
                    .update_auth_session_status(
                        session_id,
                        "failed",
                        error.error_description.as_deref(),
                    )
                    .await?;
                Err(EngineError::Auth {
                    step: "microsoft_device_token",
                    message: error
                        .error_description
                        .unwrap_or_else(|| error.error.to_string()),
                })
            }
            TokenPollResponse::Success(tokens) => Ok(Some(tokens)),
        }
    }

    fn device_code_form(&self) -> Vec<(&'static str, String)> {
        let mut form = vec![
            ("client_id", self.client_id.clone()),
            ("scope", self.auth_flow.scope().to_string()),
        ];
        if self.auth_flow == MicrosoftAuthFlow::Live {
            form.push(("response_type", "device_code".to_string()));
        }
        form
    }

    fn poll_token_form(&self, device_code: String) -> Vec<(&'static str, String)> {
        vec![
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:device_code".to_string(),
            ),
            ("client_id", self.client_id.clone()),
            ("device_code", device_code),
        ]
    }

    fn poll_token_url(&self) -> String {
        match self.auth_flow {
            MicrosoftAuthFlow::Live => {
                format!(
                    "{}?client_id={}",
                    self.auth_flow.token_url(),
                    self.client_id
                )
            }
            MicrosoftAuthFlow::Msal => self.auth_flow.token_url().to_string(),
        }
    }
}
