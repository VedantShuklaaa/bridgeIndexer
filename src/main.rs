use bridge::chain_adapters::setup::build_registry;
use bridge::config::AppConfig;
use bridge::db;
use bridge::ingestion::solana::SolanaIngester;
use bridge::redis::consumer::RedisConsumer;
use bridge::redis::producer::RedisProducer;
use bridge::routes::build_router;
use bridge::shutdown::wait_for_shutdown_signal;
use bridge::state::AppState;
use reqwest::Client;
use rustls::crypto::ring;
use std::future::Future;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

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
    let http_client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let registry = build_registry(&config, &http_client);
    let state = AppState::new(db.clone(), config.clone(), http_client.clone(), registry)?;

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

    for worker_id in 1..=5 {
        let worker_name = format!("worker-{worker_id}");
        let redis_url = config.redis_url.clone();
        let worker_state = state.clone();
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
        let recovery_state = state.clone();
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

    {
        let ws_url = config.solana_ws_url.clone();
        let bridge_program = config.solana_token_bridge_program.clone();

        let solana_ingester = SolanaIngester::new(
            ws_url,
            bridge_program,
            redis,
            db.clone(),
            http_client.clone(),
            config.helius_url.clone(),
            shutdown.clone(),
        );

        tokio::spawn(async move {
            if let Err(error) = solana_ingester.run().await {
                tracing::error!(%error, "Solana ingester stopped");
            }
        });
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
