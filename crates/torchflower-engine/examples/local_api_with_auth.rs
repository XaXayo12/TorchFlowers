use std::net::SocketAddr;

use torchflower_engine::{api, config::Config, db::Database};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let config = Config::from_env()?;
    let db = Database::connect(&config.database_url).await?;
    db.migrate().await?;
    let bind: SocketAddr = config.rust_engine_bind.parse()?;
    api::serve(config, db, bind).await
}
