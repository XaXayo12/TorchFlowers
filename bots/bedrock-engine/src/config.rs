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

#[derive(Clone, Debug)]
pub struct Config {
    pub microsoft_client_id: String,
    pub microsoft_auth_flow: MicrosoftAuthFlow,
    pub token_encryption_secret: String,
    pub database_url: String,
    pub rust_engine_bind: String,
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
            token_encryption_secret: required_env("TOKEN_ENCRYPTION_SECRET")?,
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://database/torchflower.sqlite".to_string()),
            rust_engine_bind: std::env::var("RUST_ENGINE_BIND")
                .unwrap_or_else(|_| "127.0.0.1:9080".to_string()),
        })
    }
}

fn required_env(name: &'static str) -> Result<String, EngineError> {
    let value = std::env::var(name).map_err(|_| EngineError::MissingConfig(name))?;
    if value.trim().is_empty() {
        return Err(EngineError::MissingConfig(name));
    }
    Ok(value)
}
