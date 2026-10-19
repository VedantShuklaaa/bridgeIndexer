use crate::chain_adapters::registry::AdapterRegistry;
use crate::config::AppConfig;
use crate::enrichment::EnrichmentProvider;
use crate::enrichment::wormholescan::WormholeScanProvider;
use crate::protocols::registry::{ProtocolRegistry, default_registry};
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
use tokio::sync::{broadcast, watch};

const BROADCAST_CAPACITY: usize = 1024;
pub type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    pub http_client: Client,
    pub config: AppConfig,
    pub registry: Arc<AdapterRegistry>,
    pub protocols: Arc<ProtocolRegistry>,
    pub tx_broadcast: broadcast::Sender<String>,
    /// Number of connected `/stream` clients. Live ingestion only runs while this is above zero.
    pub live_clients: Arc<watch::Sender<usize>>,
    pub read_through: Arc<ReadThrough>,
    pub helius_limiter: Arc<Limiter>,
    pub wormhole_limiter: Arc<Limiter>,
    pub enrichment: Arc<dyn EnrichmentProvider>,
}

impl AppState {
    pub fn new(
        db: PgPool,
        config: AppConfig,
        http_client: Client,
        registry: AdapterRegistry,
    ) -> anyhow::Result<Self> {
        let (tx_broadcast, _) = broadcast::channel(BROADCAST_CAPACITY);
        let (live_clients, _) = watch::channel(0usize);
        let read_through = Arc::new(ReadThrough::new(16));
        let wormhole_limiter: Arc<Limiter> = Arc::new(RateLimiter::direct(Quota::per_second(
            NonZeroU32::new(20).unwrap(),
        )));
        let enrichment = Arc::new(WormholeScanProvider::new(
            http_client.clone(),
            config.wormhole_url.clone(),
            wormhole_limiter.clone(),
        ));

        Ok(Self {
            db,
            http_client,
            config,
            registry: Arc::new(registry),
            protocols: Arc::new(default_registry()),
            tx_broadcast,
            live_clients: Arc::new(live_clients),
            read_through,
            helius_limiter: Arc::new(RateLimiter::direct(Quota::per_second(
                NonZeroU32::new(40).unwrap(),
            ))),
            wormhole_limiter,
            enrichment,
        })
    }

    /// A copy of this state for the live pipeline (backfill and consumer workers):
    /// it talks to Helius through its own endpoint, registry and rate limiter, so
    /// live traffic can never eat into the GET API's quota. Everything else
    /// (database, broadcast channel, Wormholescan limiter) stays shared.
    pub fn for_live(&self, config: AppConfig, registry: AdapterRegistry) -> Self {
        Self {
            config,
            registry: Arc::new(registry),
            helius_limiter: Arc::new(RateLimiter::direct(Quota::per_second(
                NonZeroU32::new(40).unwrap(),
            ))),
            ..self.clone()
        }
    }
}
