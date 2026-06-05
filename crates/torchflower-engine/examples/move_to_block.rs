use torchflower_engine::{
    config::Config,
    core::{BotSession, Position, ServerAddress},
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
    let x = std::env::var("TORCHFLOWER_EXAMPLE_X")?.parse::<f32>()?;
    let y = std::env::var("TORCHFLOWER_EXAMPLE_Y")?.parse::<f32>()?;
    let z = std::env::var("TORCHFLOWER_EXAMPLE_Z")?.parse::<f32>()?;

    let mut bot = BotSession::builder()
        .config(config)
        .database(db)
        .account(account_id)
        .server(ServerAddress::new(host, port))
        .build()
        .await?;

    bot.connect().await?;
    bot.move_to(Position::new(x, y, z)).await?;
    println!("movement action queued");
    Ok(())
}
