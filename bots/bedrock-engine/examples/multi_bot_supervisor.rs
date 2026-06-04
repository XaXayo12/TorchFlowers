use torchflower_engine::{bot::supervisor::BotSupervisor, config::Config, db::Database};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env()?;
    let db = Database::connect(&config.database_url).await?;
    db.migrate().await?;

    let bot_id = std::env::var("TORCHFLOWER_EXAMPLE_BOT_ID")?;
    let supervisor = BotSupervisor::new(config, db);
    supervisor.start(&bot_id).await?;
    println!("bot supervisor started {bot_id}");
    Ok(())
}
