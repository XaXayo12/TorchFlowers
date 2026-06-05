use std::time::Duration;

use torchflower_engine::{
    config::Config,
    core::{BotSession, ServerAddress},
    db::Database,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env()?;
    let db = Database::connect(&config.database_url).await?;
    db.migrate().await?;

    let account_id = std::env::var("BEDROCK_VALIDATE_ACCOUNT_ID")?;
    let host = std::env::var("BEDROCK_VALIDATE_SERVER_HOST")?;
    let port = std::env::var("BEDROCK_VALIDATE_SERVER_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(19132);

    let bot = BotSession::builder()
        .config(config)
        .database(db)
        .account(account_id)
        .server(ServerAddress::new(host, port))
        .build()
        .await?;

    let status = bot.validate_for(Duration::from_secs(30), false).await?;
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}
