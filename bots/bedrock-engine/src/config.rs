use std::{fmt, net::SocketAddr};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

use crate::error::EngineError;

pub const BEDROCK_PROTOCOL_LIVE_CLIENT_ID: &str = "00000000441cc96b";
pub const BEDROCK_PROTOCOL_LIVE_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";
pub const MSAL_SCOPE: &str = "XboxLive.signin offline_access";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MicrosoftAuthFlow {
    Live,
    Msal,
}

impl MicrosoftAuthFlow {
    pub fn from_env_value(value: Option<String>) -> Result<Self, EngineError> {
        match value
            .unwrap_or_else(|| "live".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "" | "live" | "bedrock-protocol" | "prismarine" => Ok(Self::Live),
            "msal" | "azure" => Ok(Self::Msal),
            other => Err(EngineError::InvalidRequest(format!(
                "unsupported MICROSOFT_AUTH_FLOW {other}; use live or msal"
            ))),
        }
    }

    pub fn device_code_url(&self) -> &'static str {
        match self {
            Self::Live => "https://login.live.com/oauth20_connect.srf",
            Self::Msal => "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode",
        }
    }

    pub fn token_url(&self) -> &'static str {
        match self {
            Self::Live => "https://login.live.com/oauth20_token.srf",
            Self::Msal => "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
        }
    }

    pub fn scope(&self) -> &'static str {
        match self {
            Self::Live => BEDROCK_PROTOCOL_LIVE_SCOPE,
            Self::Msal => MSAL_SCOPE,
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub microsoft_client_id: String,
    pub microsoft_auth_flow: MicrosoftAuthFlow,
    pub token_encryption_key: [u8; 32],
    pub database_url: String,
    pub rust_engine_bind: String,
    pub api_key: Option<String>,
    pub dev_allow_unauth_api: bool,
    pub cors_allowed_origins: Vec<String>,
    pub allowed_server_hosts: Vec<String>,
    pub dangerous_log_auth_bodies: bool,
}

impl Config {
    pub fn from_env() -> Result<Self, EngineError> {
        let microsoft_auth_flow =
            MicrosoftAuthFlow::from_env_value(std::env::var("MICROSOFT_AUTH_FLOW").ok())?;
        let microsoft_client_id = match microsoft_auth_flow {
            MicrosoftAuthFlow::Live => BEDROCK_PROTOCOL_LIVE_CLIENT_ID.to_string(),
            MicrosoftAuthFlow::Msal => match std::env::var("MICROSOFT_CLIENT_ID") {
                Ok(value) if !value.trim().is_empty() => value,
                _ => return Err(EngineError::MissingConfig("MICROSOFT_CLIENT_ID")),
            },
        };
        Ok(Self {
            microsoft_client_id,
            microsoft_auth_flow,
            token_encryption_key: token_encryption_key_from_env()?,
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://database/torchflower.sqlite".to_string()),
            rust_engine_bind: std::env::var("RUST_ENGINE_BIND")
                .unwrap_or_else(|_| "127.0.0.1:9080".to_string()),
            api_key: optional_trimmed_env("TORCHFLOWER_API_KEY"),
            dev_allow_unauth_api: bool_env("TORCHFLOWER_DEV_ALLOW_UNAUTH_API", false),
            cors_allowed_origins: list_env("TORCHFLOWER_CORS_ALLOWED_ORIGINS")
                .unwrap_or_else(default_cors_allowed_origins),
            allowed_server_hosts: list_env("TORCHFLOWER_ALLOWED_SERVER_HOSTS").unwrap_or_default(),
            dangerous_log_auth_bodies: bool_env("TORCHFLOWER_DANGEROUS_LOG_AUTH_BODIES", false),
        })
    }

    pub fn validate_api_security(&self, bind: SocketAddr) -> Result<(), EngineError> {
        if self.api_key.is_some() {
            return Ok(());
        }
        if self.dev_allow_unauth_api && bind.ip().is_loopback() {
            return Ok(());
        }
        Err(EngineError::MissingConfig("TORCHFLOWER_API_KEY"))
    }

    pub fn is_server_host_allowed(&self, host: &str) -> bool {
        let host = host.trim().to_ascii_lowercase();
        !host.is_empty()
            && self.allowed_server_hosts.iter().any(|allowed| {
                let allowed = allowed.trim().to_ascii_lowercase();
                allowed == "*" || allowed == host
            })
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("microsoft_client_id", &"<redacted>")
            .field("microsoft_auth_flow", &self.microsoft_auth_flow)
            .field("token_encryption_key", &"<redacted>")
            .field("database_url", &self.database_url)
            .field("rust_engine_bind", &self.rust_engine_bind)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("dev_allow_unauth_api", &self.dev_allow_unauth_api)
            .field("cors_allowed_origins", &self.cors_allowed_origins)
            .field("allowed_server_hosts", &self.allowed_server_hosts)
            .field("dangerous_log_auth_bodies", &self.dangerous_log_auth_bodies)
            .finish()
    }
}

pub fn parse_token_encryption_key(
    key_b64: Option<&str>,
    legacy_secret: Option<&str>,
) -> Result<[u8; 32], EngineError> {
    if let Some(value) = key_b64.and_then(non_empty_trimmed) {
        let decoded = STANDARD.decode(value).map_err(|_| {
            EngineError::InvalidRequest("TOKEN_ENCRYPTION_KEY_B64 must be valid base64".to_string())
        })?;
        let key: [u8; 32] = decoded.try_into().map_err(|_| {
            EngineError::InvalidRequest(
                "TOKEN_ENCRYPTION_KEY_B64 must decode to exactly 32 bytes".to_string(),
            )
        })?;
        return Ok(key);
    }

    let legacy = legacy_secret
        .and_then(non_empty_trimmed)
        .ok_or(EngineError::MissingConfig("TOKEN_ENCRYPTION_KEY_B64"))?;
    validate_legacy_secret_strength(legacy)?;
    let mut hasher = Sha256::new();
    hasher.update(legacy.as_bytes());
    Ok(hasher.finalize().into())
}

fn token_encryption_key_from_env() -> Result<[u8; 32], EngineError> {
    parse_token_encryption_key(
        std::env::var("TOKEN_ENCRYPTION_KEY_B64").ok().as_deref(),
        std::env::var("TOKEN_ENCRYPTION_SECRET").ok().as_deref(),
    )
}

fn validate_legacy_secret_strength(value: &str) -> Result<(), EngineError> {
    let normalized = value.trim().to_ascii_lowercase();
    let weak_values = [
        "password",
        "password123",
        "secret",
        "changeme",
        "change-me",
        "example",
        "replace-with-32-plus-random-characters",
        "replace_me",
        "replace-me",
    ];
    if value.len() < 32
        || weak_values.contains(&normalized.as_str())
        || normalized.contains("replace")
        || normalized.contains("changeme")
    {
        return Err(EngineError::InvalidRequest(
            "TOKEN_ENCRYPTION_SECRET is too weak; use TOKEN_ENCRYPTION_KEY_B64 with 32 random bytes"
                .to_string(),
        ));
    }
    Ok(())
}

fn optional_trimmed_env(name: &'static str) -> Option<String> {
    std::env::var(name)
        .ok()
        .and_then(|value| non_empty_trimmed(&value).map(ToOwned::to_owned))
}

fn non_empty_trimmed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn bool_env(name: &'static str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

fn list_env(name: &'static str) -> Option<Vec<String>> {
    let values = std::env::var(name).ok()?;
    let items = values
        .split(',')
        .filter_map(non_empty_trimmed)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!items.is_empty()).then_some(items)
}

fn default_cors_allowed_origins() -> Vec<String> {
    [
        "http://127.0.0.1:3000",
        "http://localhost:3000",
        "http://127.0.0.1:5173",
        "http://localhost:5173",
    ]
    .into_iter()
    .map(ToOwned::to_owned)
    .collect()
}
