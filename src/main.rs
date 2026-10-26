use bridge::chain_adapters::setup::build_registry;
use bridge::chains::{ChainRegistry, db::run_refresh_loop};
use bridge::config::AppConfig;
use bridge::db;
use bridge::enrichment::reenrich;
use bridge::ingestion::solana::SolanaIngester;
use bridge::redis::consumer::RedisConsumer;
use bridge::redis::producer::RedisProducer;
use bridge::routes::build_router;
use bridge::shutdown::wait_for_shutdown_signal;
use bridge::state::AppState;
use mimalloc::MiMalloc;
use reqwest::Client;
use rustls::crypto::ring;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn spawn_supervised<F, Fut>(task_name: &'static str, shutdown: CancellationToken, make_task: F)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    tracing::info!(task = task_name, "shutdown requested, stopping worker");
                    break;
                }
                result = tokio::spawn(make_task()) => {
                    match result {
                        Ok(Ok(())) => tracing::warn!(task = task_name, "worker exited cleanly, restarting"),
                        Ok(Err(error)) => tracing::error!(task = task_name, ?error, "worker failed, restarting"),
                        Err(join_error) => tracing::error!(task = task_name, ?join_error, "worker panicked, restarting"),
                    }
                }
            }

            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            }
        }
    });
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");
    tracing_subscriber::fmt::init();

    let config = AppConfig::from_env()?;
    let db = db::connection::connect(&config.database_url).await?;
    sqlx::migrate!("./migrations").run(&db).await?;

    let http_client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let registry = build_registry(&config, &http_client);
    let chains = ChainRegistry::load(&db).await?;
    let state = AppState::new(
        db.clone(),
        config.clone(),
        http_client.clone(),
        registry,
        chains.clone(),
    )?;

    // The live pipeline (backfill + consumer workers) gets its own Helius endpoint, registry
    // and rate limiter, so it can't use up the GET API's quota.
    let live_config = config.clone();

    let live_registry = build_registry(&live_config, &http_client);
    let live_state = state.for_live(live_config, live_registry);

    let redis = RedisProducer::new(&config.redis_url).await?;
    redis.test_connection().await?;

    let shutdown = CancellationToken::new();
    {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            wait_for_shutdown_signal().await;
            tracing::info!("shutdown signal received, draining workers");
            shutdown.cancel();
        });
    }

    tokio::spawn(run_refresh_loop(
        chains,
        db.clone(),
        Duration::from_secs(config.chain_registry_refresh_secs),
        shutdown.clone(),
    ));

    for worker_id in 1..=5 {
        let worker_name = format!("worker-{worker_id}");
        let redis_url = config.redis_url.clone();
        let worker_state = live_state.clone();
        let worker_shutdown = shutdown.clone();

        spawn_supervised("redis-consumer", shutdown.clone(), move || {
            let redis_url = redis_url.clone();
            let worker_state = worker_state.clone();
            let worker_name = worker_name.clone();
            let worker_shutdown = worker_shutdown.clone();

            async move {
                RedisConsumer::new(&redis_url, worker_state, worker_name, worker_shutdown)?
                    .run()
                    .await
            }
        });
    }

    {
        let redis_url = config.redis_url.clone();
        let recovery_state = live_state.clone();
        let recovery_shutdown = shutdown.clone();

        spawn_supervised("redis-recovery", shutdown.clone(), move || {
            let redis_url = redis_url.clone();
            let recovery_state = recovery_state.clone();
            let recovery_shutdown = recovery_shutdown.clone();

            async move {
                RedisConsumer::new(
                    &redis_url,
                    recovery_state,
                    "recovery-worker".to_string(),
                    recovery_shutdown,
                )?
                .run_recovery()
                .await
            }
        });
    }

    if config.reenrich.enabled {
        let reenrich_state = state.clone();
        let reenrich_shutdown = shutdown.clone();

        spawn_supervised("reenrichment", shutdown.clone(), move || {
            let reenrich_state = reenrich_state.clone();
            let reenrich_shutdown = reenrich_shutdown.clone();

            async move { reenrich::run(reenrich_state, reenrich_shutdown).await }
        });
    }

    if config.ingest.enabled {
        let solana_ingester = Arc::new(SolanaIngester::new(
            config.solana_ws_url.clone(),
            config.solana_token_bridge_program.clone(),
            redis.clone(),
            db.clone(),
            http_client.clone(),
            config.helius_live_url.clone(),
            config.ingest.clone(),
        ));

        // Always on: starts with the process and runs whether or not anyone is connected to
        // /stream, so the database is a complete record. The ingester reconnects on its own;
        // the supervisor only has to cover a panic.
        let ingest_shutdown = shutdown.clone();

        spawn_supervised("solana-ingester", shutdown.clone(), move || {
            let solana_ingester = solana_ingester.clone();
            let ingest_shutdown = ingest_shutdown.clone();

            async move { solana_ingester.run(ingest_shutdown).await }
        });
    } else {
        tracing::warn!("INGEST_ENABLED=0, Solana ingestion is disabled");
    }

    let app = build_router(state);
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown.clone().cancelled_owned())
        .await?;

    Ok(())
}
