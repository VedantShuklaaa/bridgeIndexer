use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

pub async fn connect(database_url: &str) -> anyhow::Result<sqlx::PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Duration::from_secs(30)) // recycle before the server does it for you
        .max_lifetime(Duration::from_secs(60 * 30))
        .test_before_acquire(true) // pings connection before handing it out
        .connect(database_url)
        .await?;

    Ok(pool)
}
