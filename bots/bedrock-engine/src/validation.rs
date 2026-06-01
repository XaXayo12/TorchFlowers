use crate::{
    auth::entitlement::EntitlementProvisioner,
    bedrock::session::BedrockBotSession,
    config::Config,
    db::Database,
    error::{EngineError, EngineResult},
};

pub struct RealServerValidation {
    config: Config,
    db: Database,
}

impl RealServerValidation {
    pub fn new(config: Config, db: Database) -> Self {
        Self { config, db }
    }

    pub async fn run_from_env(&self) -> EngineResult<()> {
        let account_id = std::env::var("BEDROCK_VALIDATE_ACCOUNT_ID")
            .map_err(|_| EngineError::MissingConfig("BEDROCK_VALIDATE_ACCOUNT_ID"))?;
        let host = std::env::var("BEDROCK_VALIDATE_SERVER_HOST")
            .map_err(|_| EngineError::MissingConfig("BEDROCK_VALIDATE_SERVER_HOST"))?;
        let port = std::env::var("BEDROCK_VALIDATE_SERVER_PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(19132);
        let provisioned = EntitlementProvisioner::new(&self.config, self.db.clone())
            .provision(&account_id)
            .await?;
        let status = BedrockBotSession::new(self.db.clone())
            .validate_real_server(&account_id, None, &host, port, &provisioned, true)
            .await?;
        println!("{}", serde_json::to_string_pretty(&status)?);
        if !status.missing_capabilities.is_empty() {
            return Err(EngineError::Bedrock(format!(
                "missing validation capabilities: {:?}",
                status.missing_capabilities
            )));
        }
        Ok(())
    }
}
