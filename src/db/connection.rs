use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

pub async fn connect(database_url: &str) -> anyhow::Result<sqlx::PgPool> {
    let max_connections = std::env::var("DB_POOL_MAX")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(20)
        .max(1);
    let pool = PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(max_connections)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(30)) // recycle before the server does it for you
        .max_lifetime(Duration::from_secs(60 * 30))
        .test_before_acquire(true) // pings connection before handing it out
        .connect(database_url)
        .await?;

    Ok(pool)
}
