#![allow(unknown_lints)]

use std::net::SocketAddr;

pub use torchflower_engine::api::{build_router, serve};

pub struct ApiConfig {
    pub config: torchflower_engine::config::Config,
    pub database: torchflower_engine::db::Database,
    pub bind: SocketAddr,
}

pub async fn start_api_server(config: ApiConfig) -> anyhow::Result<()> {
    serve(config.config, config.database, config.bind).await
}
