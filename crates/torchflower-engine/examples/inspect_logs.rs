use sqlx::SqlitePool;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://database/rustrock.sqlite".to_string());
    let pool = SqlitePool::connect(&db_url).await?;

    let accounts =
        sqlx::query!("SELECT id, microsoft_status, entitlement_status, last_error FROM accounts")
            .fetch_all(&pool)
            .await?;

    println!("=== ACCOUNTS ===");
    for acc in accounts {
        println!(
            "ID: {} | MS: {} | Entitlement: {} | Error: {:?}",
            acc.id.as_deref().unwrap_or("None"),
            acc.microsoft_status,
            acc.entitlement_status,
            acc.last_error
        );
    }

    let logs = sqlx::query!("SELECT level, category, step, message, metadata_json, created_at FROM logs ORDER BY created_at DESC LIMIT 30")
        .fetch_all(&pool)
        .await?;

    println!("\n=== LAST 30 LOGS ===");
    for log in logs {
        println!(
            "[{}] {} - {}/{} : {} | metadata: {}",
            log.created_at,
            log.level,
            log.category,
            log.step.unwrap_or_default(),
            log.message,
            log.metadata_json
        );
    }

    Ok(())
}
