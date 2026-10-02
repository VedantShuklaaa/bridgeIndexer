use reqwest::Client;
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::broadcast;
use crate::services::read_through::ReadThrough;
use crate::chain_adapters::registry::AdapterRegistry;
use crate::config::AppConfig;

const BROADCAST_CAPACITY: usize = 1024;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub http_client: Client,
    pub config: AppConfig,
    pub registry: Arc<AdapterRegistry>,
    pub tx_broadcast: broadcast::Sender<String>,
    pub read_through: Arc<ReadThrough>,
}

impl AppState {
    pub fn new(
        db: PgPool,
        config: AppConfig,
        http_client: Client,
        registry: AdapterRegistry,
    ) -> anyhow::Result<Self> {
        let (tx_broadcast, _) = broadcast::channel(BROADCAST_CAPACITY);
        let read_through = Arc::new(ReadThrough::new(16));

        Ok(Self {
            db,
            http_client,
            config,
            registry: Arc::new(registry),
            tx_broadcast,
            read_through,
        })
    }
}
