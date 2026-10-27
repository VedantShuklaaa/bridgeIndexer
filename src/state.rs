use crate::chain_adapters::registry::AdapterRegistry;
use crate::chains::ChainRegistry;
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
use std::ops::Deref;
use std::sync::Arc;
use tokio::sync::broadcast;

const BROADCAST_CAPACITY: usize = 1024;

pub type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// The shared state itself. Kept behind a single `Arc` in [`AppState`] so cloning
/// the state is one refcount bump instead of one per field.
#[derive(Clone)]
pub struct Inner {
    pub db: PgPool,
    pub http_client: Client,
    pub config: Arc<AppConfig>,
    pub registry: Arc<AdapterRegistry>,
    pub chains: ChainRegistry,
    pub protocols: Arc<ProtocolRegistry>,
    pub tx_broadcast: broadcast::Sender<String>,
    pub read_through: Arc<ReadThrough>,
    pub helius_limiter: Arc<Limiter>,
    pub wormhole_limiter: Arc<Limiter>,
    pub enrichment: Arc<dyn EnrichmentProvider>,
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

impl Deref for AppState {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl AppState {
    pub fn new(
        db: PgPool,
        config: AppConfig,
        http_client: Client,
        registry: AdapterRegistry,
        chains: ChainRegistry,
    ) -> anyhow::Result<Self> {
        let (tx_broadcast, _) = broadcast::channel(BROADCAST_CAPACITY);
        let read_through = Arc::new(ReadThrough::new(config.upstream_permits));
        let api_rps = config.helius_api_rps;
        let wormhole_rps = config.wormhole_rps;
        let wormhole_limiter: Arc<Limiter> = Arc::new(RateLimiter::direct(Quota::per_second(
            NonZeroU32::new(wormhole_rps).unwrap(),
        )));
        let enrichment = Arc::new(WormholeScanProvider::new(
            http_client.clone(),
            config.wormhole_url.clone(),
            wormhole_limiter.clone(),
            chains.clone(),
        ));

        Ok(Self {
            inner: Arc::new(Inner {
                db,
                http_client,
                config: Arc::new(config),
                registry: Arc::new(registry),
                chains,
                protocols: Arc::new(default_registry()),
                tx_broadcast,
                read_through,
                helius_limiter: Arc::new(RateLimiter::direct(Quota::per_second(
                    NonZeroU32::new(api_rps).unwrap(),
                ))),
                wormhole_limiter,
                enrichment,
            }),
        })
    }

    /// A copy of this state for the live pipeline (backfill and consumer workers):
    /// it talks to Helius through its own endpoint, registry and rate limiter, so
    /// live traffic can never eat into the GET API's quota. Everything else
    /// (database, broadcast channel, Wormholescan limiter) stays shared.
    pub fn for_live(&self, config: AppConfig, registry: AdapterRegistry) -> Self {
        let live_rps = config.helius_live_rps;
        Self {
            inner: Arc::new(Inner {
                config: Arc::new(config),
                registry: Arc::new(registry),
                helius_limiter: Arc::new(RateLimiter::direct(Quota::per_second(
                    NonZeroU32::new(live_rps).unwrap(),
                ))),
                ..(*self.inner).clone()
            }),
        }
    }
}
