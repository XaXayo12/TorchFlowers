use torchflower_engine::{config::Config, db::Database, validation::RealServerValidation};

#[tokio::test]
#[ignore = "requires a provisioned account and an authorized Bedrock server"]
async fn real_bds_validation_from_env_is_opt_in() {
    dotenvy::dotenv().ok();
    let config = Config::from_env().unwrap();
    let db = Database::connect(&config.database_url).await.unwrap();
    db.migrate().await.unwrap();
    RealServerValidation::new(config, db)
        .run_from_env()
        .await
        .unwrap();
}
