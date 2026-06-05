use torchflower_engine::{auth::microsoft::MicrosoftAuth, config::Config, db::Database};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env()?;
    let db = Database::connect(&config.database_url).await?;
    db.migrate().await?;

    let email = std::env::var("TORCHFLOWER_EXAMPLE_EMAIL")?;
    let session = MicrosoftAuth::new(&config, db)
        .start_device_auth(&email)
        .await?;

    println!("Open: {}", session.verification_uri);
    println!("Code: {}", session.user_code);
    println!("Poll session id: {}", session.id);
    Ok(())
}
