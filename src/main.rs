use std::sync::Arc;

use bridge::chain_adapters::registry::AdapterRegistry;
use bridge::chain_adapters::wormholescan::WormholeScanAdapter;
use bridge::config::AppConfig;
use bridge::routes::build_router;
use bridge::state::AppState;
use reqwest::Client;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = AppConfig::from_env()?;
    let db = PgPoolOptions::new()
        .max_connections(5)
        .connect_lazy(&config.database_url)?;

    let http_client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let mut registry = AdapterRegistry::new();
    registry.register(
        30,
        Arc::new(WormholeScanAdapter::new(
            http_client.clone(),
            config.wormhole_url.clone(),
            "base",
        )),
    );

    let state = AppState::new(db, config.clone(), registry)?;
    let app = build_router(state);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("listening on {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}
