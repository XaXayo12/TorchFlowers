use torchflower_engine::{
    config::Config,
    core::{AutomationPolicy, BlockPosition, BotSession, ServerAddress},
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
    let target = BlockPosition::new(
        std::env::var("TORCHFLOWER_EXAMPLE_BLOCK_X")?.parse()?,
        std::env::var("TORCHFLOWER_EXAMPLE_BLOCK_Y")?.parse()?,
        std::env::var("TORCHFLOWER_EXAMPLE_BLOCK_Z")?.parse()?,
    );

    let mut bot = BotSession::builder()
        .config(config)
        .database(db)
        .account(account_id)
        .server(ServerAddress::new(host.clone(), port))
        .automation_policy(AutomationPolicy::allow_for_hosts([host]))
        .build()
        .await?;

    bot.connect().await?;
    bot.place_block(target).await?;
    println!("place action queued");
    Ok(())
}
