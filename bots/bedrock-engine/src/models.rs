use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub gamertag: Option<String>,
    pub xuid: Option<String>,
    pub microsoft_status: String,
    pub xbox_status: String,
    pub xsts_status: String,
    pub playfab_status: String,
    pub entitlement_status: String,
    pub bedrock_auth_status: String,
    pub bot_status: String,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: i64,
    pub protocol_version: i64,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bot {
    pub id: String,
    pub account_id: String,
    pub server_id: String,
    pub status: String,
    pub reconnect_enabled: bool,
    pub anti_afk_enabled: bool,
    pub current_position: Option<String>,
    pub inventory_json: serde_json::Value,
    pub capabilities_json: serde_json::Value,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: String,
    pub account_id: Option<String>,
    pub bot_id: Option<String>,
    pub level: String,
    pub category: String,
    pub step: Option<String>,
    pub request_id: Option<String>,
    pub method: Option<String>,
    pub url: Option<String>,
    pub status_code: Option<i64>,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    pub message: String,
    pub metadata_json: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilityStatus {
    pub login: bool,
    pub spawn: bool,
    pub keepalive: bool,
    pub chat: bool,
    pub forms: bool,
    pub inventory_transactions: bool,
    pub movement: bool,
    pub disconnect_handling: bool,
    pub missing_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceAuthSession {
    pub id: String,
    pub account_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: DateTime<Utc>,
    pub interval_seconds: u64,
    pub status: String,
}
