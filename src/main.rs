use bridge::chain_adapters::setup::build_registry;
use bridge::config::AppConfig;
use bridge::ingestion::solana::SolanaIngester;
use bridge::redis::consumer::RedisConsumer;
use bridge::redis::producer::RedisProducer;
use bridge::routes::build_router;
use bridge::state::AppState;
use reqwest::Client;
use rustls::crypto::ring;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");
    tracing_subscriber::fmt::init();

    let config = AppConfig::from_env()?;
    let db = PgPoolOptions::new()
        .max_connections(5)
        .connect_lazy(&config.database_url)?;

    let http_client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let registry = build_registry(&config, &http_client);

    let state = AppState::new(db, config.clone(), http_client.clone(), registry)?;

    let redis = RedisProducer::new(&config.redis_url)?;
    redis.test_connection().await?;

    for worker_id in 1..=5 {
        let worker_name = format!("worker-{worker_id}");

        let redis_consumer =
            RedisConsumer::new(&config.redis_url, state.clone(), worker_name.clone())?;

        tokio::spawn(async move {
            if let Err(error) = redis_consumer.run().await {
                tracing::error!(
                    consumer = %worker_name,
                    ?error,
                    "Redis consumer stopped"
                );
            }
        });
    }

    let recovery_consumer = RedisConsumer::new(
        &config.redis_url,
        state.clone(),
        "recovery-worker".to_string(),
    )?;

    tokio::spawn(async move {
        if let Err(error) = recovery_consumer.run_recovery().await {
            tracing::error!(?error, "Redis recovery worker stopped");
        }
    });

    let solana_ingester = SolanaIngester::new(
        config.solana_ws_url.clone(),
        config.solana_token_bridge_program.clone(),
        redis,
    );

    tokio::spawn(async move {
        if let Err(error) = solana_ingester.run().await {
            tracing::error!(%error, "Solana ingester stopped");
        }
    });

    let app = build_router(state);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("listening on {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}
