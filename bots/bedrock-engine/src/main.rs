use std::net::SocketAddr;

use bedrock_engine::{api, config::Config, db::Database, validation::RealServerValidation};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env()?;
    let db = Database::connect(&config.database_url).await?;
    db.migrate().await?;

    match std::env::args().nth(1).as_deref() {
        Some("validate-real-server") => {
            RealServerValidation::new(config, db).run_from_env().await?;
        }
        _ => {
            let bind: SocketAddr = config.rust_engine_bind.parse()?;
            api::serve(config, db, bind).await?;
        }
    }

    Ok(())
}
