use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use p256::{
    ecdsa::{signature::Signer, Signature, SigningKey},
    elliptic_curve::rand_core::OsRng,
    EncodedPoint,
};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    auth::XboxIdentity,
    config::{Config, MicrosoftAuthFlow},
    db::Database,
    diagnostics::Diagnostics,
    error::{EngineError, EngineResult},
};

const XBOX_AUTH_RELYING_PARTY: &str = "http://auth.xboxlive.com";
const USER_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const DEVICE_AUTH_URL: &str = "https://device.auth.xboxlive.com/device/authenticate";
const TITLE_AUTH_URL: &str = "https://title.auth.xboxlive.com/title/authenticate";

#[derive(Clone)]
pub struct XboxProofKey {
    signing_key: SigningKey,
    jwk: Value,
}

impl std::fmt::Debug for XboxProofKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XboxProofKey").finish_non_exhaustive()
    }
}

impl XboxProofKey {
    pub fn generate() -> Self {
        let signing_key = SigningKey::random(&mut OsRng);
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        let jwk = p256_public_jwk(&public_key);
        Self { signing_key, jwk }
    }

    pub fn jwk(&self) -> Value {
        self.jwk.clone()
    }

    pub fn signature_header(&self, url: &str, authorization_token: &str, payload: &str) -> String {
        STANDARD.encode(self.sign_request(url, authorization_token, payload))
    }

    fn sign_request(&self, url: &str, authorization_token: &str, payload: &str) -> Vec<u8> {
        let windows_timestamp = ((Utc::now().timestamp() as u64) + 11_644_473_600) * 10_000_000;
        let path = reqwest::Url::parse(url)
            .ok()
            .map(|url| url.path().to_string())
            .unwrap_or_else(|| url.to_string());

        let mut buffer = Vec::new();
        buffer.extend_from_slice(&1_i32.to_be_bytes());
        buffer.push(0);
        buffer.extend_from_slice(&windows_timestamp.to_be_bytes());
        buffer.push(0);
        buffer.extend_from_slice(b"POST");
        buffer.push(0);
        buffer.extend_from_slice(path.as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(authorization_token.as_bytes());
        buffer.push(0);
        buffer.extend_from_slice(payload.as_bytes());
        buffer.push(0);

        let signature: Signature = self.signing_key.sign(&buffer);
        let mut header = Vec::with_capacity(12 + signature.to_bytes().len());
        header.extend_from_slice(&1_i32.to_be_bytes());
        header.extend_from_slice(&windows_timestamp.to_be_bytes());
        header.extend_from_slice(&signature.to_bytes());
        header
    }
}

#[derive(Clone)]
pub struct XboxAuth {
    client: reqwest::Client,
    diagnostics: Diagnostics,
    auth_flow: MicrosoftAuthFlow,
}

impl XboxAuth {
    pub fn new(config: &Config, db: Database) -> Self {
        Self {
            client: reqwest::Client::new(),
            diagnostics: Diagnostics::new(db),
            auth_flow: config.microsoft_auth_flow.clone(),
        }
    }

    pub async fn authenticate(
        &self,
        account_id: &str,
        microsoft_access_token: &str,
    ) -> EngineResult<XboxIdentity> {
        let proof_key = XboxProofKey::generate();
        let rps_prefix = match self.auth_flow {
            MicrosoftAuthFlow::Live => "t=",
            MicrosoftAuthFlow::Msal => "d=",
        };
        let body = json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("{rps_prefix}{microsoft_access_token}")
            },
            "RelyingParty": XBOX_AUTH_RELYING_PARTY,
            "TokenType": "JWT"
        });
        let body_text = body.to_string();
        let signature = proof_key.signature_header(USER_AUTH_URL, "", &body_text);
        let (response, _, _) = self
            .diagnostics
            .request_json_text::<XboxAuthResponse>(
                &self.client,
                Some(account_id),
                "xbox_live_authentication",
                Method::POST,
                USER_AUTH_URL,
                vec![
                    (
                        "Cache-Control",
                        "no-store, must-revalidate, no-cache".to_string(),
                    ),
                    ("Signature", signature),
                    ("accept", "application/json".to_string()),
                    ("x-xbl-contract-version", "2".to_string()),
                ],
                body_text,
            )
            .await?;

        let device_token = self.device_token(account_id, &proof_key).await?;
        let title_token = if self.auth_flow == MicrosoftAuthFlow::Live {
            match self
                .title_token(
                    account_id,
                    &proof_key,
                    microsoft_access_token,
                    &device_token,
                )
                .await
            {
                Ok(token) => Some(token),
                Err(error) => {
                    let message = format!(
                        "{}. Re-run device-code auth for this account so the Microsoft token is issued to the bedrock-protocol Live title id.",
                        error
                    );
                    self.diagnostics
                        .log_event(
                            Some(account_id),
                            None,
                            "error",
                            "auth",
                            Some("xbox_title_authentication"),
                            "Xbox title authentication failed; Bedrock login would produce titleId=null",
                            json!({ "error": message }),
                        )
                        .await?;
                    return Err(EngineError::Auth {
                        step: "xbox_title_authentication",
                        message,
                    });
                }
            }
        } else {
            None
        };

        let xui = response
            .display_claims
            .xui
            .first()
            .cloned()
            .unwrap_or_default();
        Ok(XboxIdentity {
            token: response.token,
            device_token: Some(device_token),
            title_token,
            proof_key: Some(proof_key),
            user_hash: xui.uhs.unwrap_or_default(),
            xuid: xui.xid,
            gamertag: xui.gtg,
        })
    }

    async fn device_token(
        &self,
        account_id: &str,
        proof_key: &XboxProofKey,
    ) -> EngineResult<String> {
        let body = json!({
            "Properties": {
                "AuthMethod": "ProofOfPossession",
                "Id": format!("{{{}}}", Uuid::new_v4()),
                "DeviceType": "Nintendo",
                "SerialNumber": format!("{{{}}}", Uuid::new_v4()),
                "Version": "0.0.0",
                "ProofKey": proof_key.jwk()
            },
            "RelyingParty": XBOX_AUTH_RELYING_PARTY,
            "TokenType": "JWT"
        });
        let body_text = body.to_string();
        let signature = proof_key.signature_header(DEVICE_AUTH_URL, "", &body_text);
        let (response, _, _) = self
            .diagnostics
            .request_json_text::<XboxTokenOnlyResponse>(
                &self.client,
                Some(account_id),
                "xbox_device_authentication",
                Method::POST,
                DEVICE_AUTH_URL,
                vec![
                    (
                        "Cache-Control",
                        "no-store, must-revalidate, no-cache".to_string(),
                    ),
                    ("Signature", signature),
                    ("x-xbl-contract-version", "1".to_string()),
                ],
                body_text,
            )
            .await?;
        Ok(response.token)
    }

    async fn title_token(
        &self,
        account_id: &str,
        proof_key: &XboxProofKey,
        microsoft_access_token: &str,
        device_token: &str,
    ) -> EngineResult<String> {
        let body = json!({
            "Properties": {
                "AuthMethod": "RPS",
                "DeviceToken": device_token,
                "RpsTicket": format!("t={microsoft_access_token}"),
                "SiteName": "user.auth.xboxlive.com",
                "ProofKey": proof_key.jwk()
            },
            "RelyingParty": XBOX_AUTH_RELYING_PARTY,
            "TokenType": "JWT"
        });
        let body_text = body.to_string();
        let signature = proof_key.signature_header(TITLE_AUTH_URL, "", &body_text);
        let (response, _, _) = self
            .diagnostics
            .request_json_text::<XboxTokenOnlyResponse>(
                &self.client,
                Some(account_id),
                "xbox_title_authentication",
                Method::POST,
                TITLE_AUTH_URL,
                vec![
                    (
                        "Cache-Control",
                        "no-store, must-revalidate, no-cache".to_string(),
                    ),
                    ("Signature", signature),
                    ("x-xbl-contract-version", "1".to_string()),
                ],
                body_text,
            )
            .await?;
        Ok(response.token)
    }
}

#[derive(Debug, Deserialize)]
struct XboxAuthResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims", default)]
    display_claims: DisplayClaims,
}

#[derive(Debug, Deserialize)]
struct XboxTokenOnlyResponse {
    #[serde(rename = "Token")]
    token: String,
}

#[derive(Debug, Deserialize, Default)]
struct DisplayClaims {
    #[serde(default)]
    xui: Vec<Xui>,
}

#[derive(Debug, Deserialize, Clone, Default)]
struct Xui {
    uhs: Option<String>,
    xid: Option<String>,
    gtg: Option<String>,
}

fn p256_public_jwk(public_key: &EncodedPoint) -> Value {
    let x = public_key.x().expect("P-256 public key has x coordinate");
    let y = public_key.y().expect("P-256 public key has y coordinate");
    json!({
        "kty": "EC",
        "crv": "P-256",
        "alg": "ES256",
        "use": "sig",
        "x": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(x),
        "y": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(y)
    })
}
