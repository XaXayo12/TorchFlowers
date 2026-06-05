use crate::{
    config::Config,
    core::{AutomationPolicy, BotSession, ServerAddress},
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
        let duration = std::env::var("BEDROCK_VALIDATE_DURATION_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(300)
            .clamp(5, 900);
        let mut policy = AutomationPolicy::allow_for_hosts([host.clone()]);
        policy.allow_gameplay_actions = true;
        let bot = BotSession::builder()
            .config(self.config.clone())
            .database(self.db.clone())
            .account(account_id.clone())
            .server(ServerAddress::new(host.clone(), port))
            .automation_policy(policy)
            .build()
            .await
            .map_err(|err| EngineError::InvalidRequest(err.to_string()))?;
        let status = bot
            .validate_for(std::time::Duration::from_secs(duration), true)
            .await
            .map_err(|err| EngineError::Bedrock(err.to_string()))?;
        println!("{}", serde_json::to_string_pretty(&status)?);
        if !status.success {
            return Err(EngineError::Bedrock(format!(
                "missing validation capabilities: {:?}",
                status.missing_capabilities
            )));
        }
        Ok(())
    }
}
