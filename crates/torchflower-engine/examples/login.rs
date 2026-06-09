use torchflower_engine::{
    auth::{entitlement::EntitlementProvisioner, microsoft::MicrosoftAuth},
    config::Config,
    db::Database,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env()?;
    let db = Database::connect(&config.database_url).await?;
    db.migrate().await?;

    let email = std::env::var("TORCHFLOWER_EXAMPLE_EMAIL")?;
    let microsoft = MicrosoftAuth::new(&config, db.clone());
    let session = microsoft.start_device_auth(&email).await?;

    println!();
    println!("=== Microsoft Device Login ===");
    println!("1. Open:  {}", session.verification_uri);
    println!("2. Enter: {}", session.user_code);
    println!("   (session id: {})", session.id);
    println!();
    println!("Waiting for you to complete login in the browser...");

    let tokens = loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        match microsoft.poll_device_auth(&session.id).await? {
            Some(tokens) => break tokens,
            None => print!("."),
        }
        // flush stdout so the dots appear without newlines
        use std::io::Write;
        std::io::stdout().flush().ok();
    };

    println!();
    println!("Microsoft login successful — provisioning Bedrock session...");

    let provisioner = EntitlementProvisioner::new(&config, db);
    provisioner
        .save_microsoft_tokens(&session.account_id, &tokens)
        .await?;
    let bedrock_session = provisioner.provision(&session.account_id).await?;

    println!();
    println!("=== Account provisioned ===");
    println!("Account ID : {}", bedrock_session.account_id);
    println!("PlayFab ID : {}", bedrock_session.playfab_id);
    println!();
    println!("The account is now stored in the database and ready for use.");
    println!("You can export a session JSON for the lite-bot with:");
    println!("  GET /api/accounts/{}/session", bedrock_session.account_id);

    Ok(())
}
