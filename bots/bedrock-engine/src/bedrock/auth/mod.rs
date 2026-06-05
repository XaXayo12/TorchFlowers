pub mod entitlement;
pub mod microsoft;
pub mod minecraft;
pub mod playfab;
pub mod token_manager;
pub mod xbox;
pub mod xsts;

use serde::{Deserialize, Serialize};

pub use xbox::XboxProofKey;

#[derive(Debug, Clone)]
pub struct XboxIdentity {
    pub token: String,
    pub device_token: Option<String>,
    pub title_token: Option<String>,
    pub proof_key: Option<XboxProofKey>,
    pub user_hash: String,
    pub xuid: Option<String>,
    pub gamertag: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XstsToken {
    pub token: String,
    pub user_hash: String,
    pub xuid: Option<String>,
    pub gamertag: Option<String>,
    pub relying_party: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionedBedrockSession {
    pub account_id: String,
    pub playfab_id: String,
    pub playfab_session_ticket: String,
    pub minecraft_access_token: String,
    pub bedrock_login_token: String,
    pub legacy_bedrock_token: String,
    pub chain: crate::auth::minecraft::BedrockJwtChain,
}
