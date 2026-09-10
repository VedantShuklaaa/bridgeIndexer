use bridge::config::AppConfig;
use bridge::routes::build_router;
use bridge::state::AppState;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = AppConfig::from_env()?;
    let db = PgPoolOptions::new()
        .max_connections(5)
        .connect_lazy(&config.database_url)?;
    let state = AppState::new(db, config.clone())?;

    let app = build_router(state);
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("listening on {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}
