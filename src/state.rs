use crate::chain_adapters::registry::AdapterRegistry;
use crate::chains::ChainRegistry;
use crate::clients::limiter::API_TOKEN_WAIT;
use crate::config::AppConfig;
use crate::enrichment::EnrichmentProvider;
use crate::enrichment::wormholescan::WormholeScanProvider;
use crate::protocols::registry::{ProtocolRegistry, default_registry};
use crate::redis::broadcast::BroadcastSink;
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
use std::{sync::Arc, time::Duration};
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
    /// Where workers send broadcast events. Local in `ROLE=all`, Redis pub/sub in
    /// `ROLE=stream`.
    pub broadcast: BroadcastSink,
    /// The read-through cache. Built only in the roles that serve the read API.
    pub read_through: Option<Arc<ReadThrough>>,
    pub helius_limiter: Arc<Limiter>,
    pub wormhole_limiter: Arc<Limiter>,
    pub enrichment: Arc<dyn EnrichmentProvider>,
    pub helius_wait: Duration,
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
    /// A state for unit tests: an unconnectable lazy pool and an empty chain registry,
    /// so a test that reaches the read path fails fast instead of touching a database.
    #[cfg(test)]
    pub(crate) fn test_state(config: AppConfig) -> Self {
        use crate::chain_adapters::setup::build_registry;
        use crate::chains::Snapshot;

        let client = Client::new();
        let registry = build_registry(&config, &client, config.helius_api_rps);
        let chains = ChainRegistry::new(Snapshot::build(Vec::new(), Vec::new()));
        let db = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy(&config.database_url)
            .expect("lazy pool");
        Self::new(db, config, client, registry, chains).expect("test state")
    }

    pub fn new(
        db: PgPool,
        config: AppConfig,
        http_client: Client,
        registry: AdapterRegistry,
        chains: ChainRegistry,
    ) -> anyhow::Result<Self> {
        let (tx_broadcast, _) = broadcast::channel(BROADCAST_CAPACITY);
        let read_through = Arc::new(ReadThrough::new(
            config.upstream_permits,
            Duration::from_secs(config.failed_cache_ttl_secs),
        ));
        let wormhole_rps = config.wormhole_rps;
        // The registry built the Helius adapter and its token metadata lookup around
        // one limiter; reuse it so metadata and getTransaction share the same quota.
        let helius_limiter = registry.helius_limiter().unwrap_or_else(|| {
            Arc::new(RateLimiter::direct(Quota::per_second(
                NonZeroU32::new(config.helius_api_rps).unwrap(),
            )))
        });
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
                broadcast: BroadcastSink::Local(tx_broadcast.clone()),
                tx_broadcast,
                read_through: Some(read_through),
                helius_limiter,
                wormhole_limiter,
                enrichment,
                helius_wait: API_TOKEN_WAIT,
            }),
        })
    }

    /// The slim state `ROLE=stream` runs with: no read-through cache and the
    /// registry passed in is the live-lane one (the API lane is never built).
    ///
    /// The protocol registry and the live registry stay: consumer analysis
    /// (`analyse_tx`) decodes with the former and resolves token metadata through
    /// the latter. Broadcast events go to Redis pub/sub instead of the local channel.
    pub async fn new_stream(
        db: PgPool,
        config: AppConfig,
        http_client: Client,
        registry: AdapterRegistry,
        chains: ChainRegistry,
    ) -> anyhow::Result<Self> {
        let (tx_broadcast, _) = broadcast::channel(BROADCAST_CAPACITY);
        let broadcast =
            BroadcastSink::redis(&config.redis_url, config.pubsub_channel.clone()).await?;
        let helius_limiter = registry.helius_limiter().unwrap_or_else(|| {
            Arc::new(RateLimiter::direct(Quota::per_second(
                NonZeroU32::new(config.helius_live_rps).unwrap(),
            )))
        });
        let helius_wait = config.helius_live_token_wait;
        let wormhole_limiter: Arc<Limiter> = Arc::new(RateLimiter::direct(Quota::per_second(
            NonZeroU32::new(config.wormhole_rps).unwrap(),
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
                broadcast,
                tx_broadcast,
                read_through: None,
                helius_limiter,
                wormhole_limiter,
                enrichment,
                helius_wait,
            }),
        })
    }

    /// The read-through cache. Only the api/all roles build it; the stream role has
    /// no read path, so reaching this there is a wiring bug.
    pub fn read_through(&self) -> &Arc<ReadThrough> {
        self.read_through
            .as_ref()
            .expect("read-through cache is built only in ROLE=api|all")
    }

    /// A copy of this state for the live pipeline (backfill and consumer workers):
    /// it talks to Helius through its own endpoint, registry and rate limiter, so
    /// live traffic can never eat into the GET API's quota. Everything else
    /// (database, broadcast channel, Wormholescan limiter) stays shared.
    pub fn for_live(&self, config: AppConfig, registry: AdapterRegistry) -> Self {
        // Same wiring as `new`: the live registry's metadata lookup and the live
        // pipeline share one limiter, separate from the GET API's.
        let helius_limiter = registry.helius_limiter().unwrap_or_else(|| {
            Arc::new(RateLimiter::direct(Quota::per_second(
                NonZeroU32::new(config.helius_live_rps).unwrap(),
            )))
        });
        let helius_wait = config.helius_live_token_wait;
        Self {
            inner: Arc::new(Inner {
                config: Arc::new(config),
                registry: Arc::new(registry),
                helius_limiter,
                helius_wait,
                ..(*self.inner).clone()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain_adapters::setup::build_registry;

    /// The live lane must spend the live Helius key: every caller reads
    /// `config.helius_url`, so `for_live` has to repoint it at `HELIUS_LIVE_URL`.
    #[tokio::test]
    async fn the_live_lane_uses_the_live_helius_url() {
        let config = AppConfig::test_config();
        let expected = config.helius_live_url.clone();
        assert_ne!(expected, config.helius_url);

        let base = AppState::test_state(config.clone());
        let live_config = config.for_live();
        let registry = build_registry(&live_config, &base.http_client, live_config.helius_live_rps);
        let live = base.for_live(live_config, registry);

        assert_eq!(live.config.helius_url, expected);
    }
}
