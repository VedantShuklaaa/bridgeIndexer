use crate::chain_adapters::registry::AdapterRegistry;
use crate::config::AppConfig;
use crate::services::read_through::ReadThrough;
use governor::{
    Quota, RateLimiter,
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
};
use reqwest::Client;
use sqlx::PgPool;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::sync::broadcast;

const BROADCAST_CAPACITY: usize = 1024;
pub type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub http_client: Client,
    pub config: AppConfig,
    pub registry: Arc<AdapterRegistry>,
    pub tx_broadcast: broadcast::Sender<String>,
    pub read_through: Arc<ReadThrough>,
    pub helius_limiter: Arc<Limiter>,
    pub wormhole_limiter: Arc<Limiter>,
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
            helius_limiter: Arc::new(RateLimiter::direct(Quota::per_second(
                NonZeroU32::new(40).unwrap(),
            ))),
            wormhole_limiter: Arc::new(RateLimiter::direct(Quota::per_second(
                NonZeroU32::new(20).unwrap(),
            ))),
        })
    }
}
