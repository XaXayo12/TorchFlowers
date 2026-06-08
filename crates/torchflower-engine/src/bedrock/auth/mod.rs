#[cfg(feature = "full-engine")]
pub mod entitlement;
#[cfg(feature = "full-engine")]
pub mod microsoft;
pub mod minecraft;
#[cfg(feature = "full-engine")]
pub mod playfab;
#[cfg(feature = "full-engine")]
pub mod token_manager;
#[cfg(feature = "full-engine")]
pub mod xbox;
#[cfg(feature = "full-engine")]
pub mod xsts;

use serde::{Deserialize, Serialize};

#[cfg(feature = "full-engine")]
pub use xbox::XboxProofKey;

#[derive(Debug, Clone)]
pub struct XboxIdentity {
    pub token: String,
    pub device_token: Option<String>,
    pub title_token: Option<String>,
    #[cfg(feature = "full-engine")]
    pub proof_key: Option<XboxProofKey>,
    #[cfg(not(feature = "full-engine"))]
    pub proof_key: Option<()>,
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
