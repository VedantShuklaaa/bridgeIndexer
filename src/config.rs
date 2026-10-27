/// Background re-enrichment of `Detected` and `Pending` transfers.
#[derive(Clone, Debug)]
pub struct ReenrichConfig {
    pub enabled: bool,
    /// Seconds between passes.
    pub interval_secs: u64,
    /// Rows handled per pass.
    pub batch: i64,
    /// A row is left to the normal pipeline until it is this old.
    pub min_age_secs: i64,
    /// A row is not asked again before this long after its last attempt.
    pub retry_secs: i64,
    /// Rows older than this are given up on (stale-while-revalidate can still refresh them).
    pub max_age_secs: i64,
}

/// Always-on Solana ingestion (A8).
#[derive(Clone, Debug)]
pub struct IngestConfig {
    /// Kill switch. `INGEST_ENABLED=0` stops the ingester from starting at all.
    pub enabled: bool,
    /// Replay the signatures missed since the checkpoint each time a connection starts.
    pub backfill: bool,
    /// Most signatures one backfill run will publish. Bounds the Helius spend after a long
    /// outage; the checkpoint still moves forward, so the next restart does not repeat it.
    pub backfill_max: usize,
    /// Pause between backfilled signatures, to stay under the Helius plan's rate cap.
    pub backfill_pace_ms: u64,
    /// How long a published signature is remembered so a repeat (backfill overlapping the
    /// live feed, a reconnect) is not published twice. `0` disables the check.
    pub dedupe_ttl_secs: u64,
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub database_url: String,
    pub redis_url: String,
    /// Helius endpoint for the GET API (`/analyse`).
    pub helius_url: String,
    /// Helius endpoint for the live pipeline (backfill + consumer workers).
    /// Falls back to `helius_url` when `HELIUS_LIVE_URL` is unset.
    pub helius_live_url: String,
    pub wormhole_url: String,
    /// Helius requests/sec for the GET API. Helius free plan: 10 requests/sec per key.
    /// The API and the live pipeline share one key until Phase 2 gives each its own,
    /// so `helius_api_rps + helius_live_rps` must add up to the plan limit.
    pub helius_api_rps: u32,
    /// Helius requests/sec for the live pipeline (backfill + consumer workers).
    pub helius_live_rps: u32,
    /// WormholeScan requests/sec, shared by the GET API and enrichment.
    pub wormhole_rps: u32,
    /// Most upstream fetches one cold `/analyse` may run at once.
    pub upstream_permits: usize,
    pub port: u16,
    pub solana_token_bridge_program: String,
    pub solana_ws_url: String,
    pub allowed_origins: Vec<String>,
    pub ingest: IngestConfig,
    pub reenrich: ReenrichConfig,
    /// How often the in-memory chain registry reloads from Postgres.
    pub chain_registry_refresh_secs: u64,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        fn required(key: &str) -> anyhow::Result<String> {
            std::env::var(key).map_err(|_| anyhow::anyhow!("{key} not set"))
        }
        fn optional(key: &str, default: &str) -> String {
            std::env::var(key).unwrap_or_else(|_| default.to_string())
        }
        fn csv_list(key: &str) -> Vec<String> {
            std::env::var(key)
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }

        fn number<T: std::str::FromStr>(key: &str, default: T) -> T {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }

        let helius_url = required("HELIUS_URL")?;

        Ok(Self {
            database_url: required("DATABASE_URL")?,
            redis_url: required("REDIS_URL")?,
            helius_live_url: optional("HELIUS_LIVE_URL", &helius_url),
            helius_url,
            wormhole_url: optional("WORMHOLE_URL", "https://api.wormholescan.io"),
            helius_api_rps: number("HELIUS_API_RPS", 6u32).max(1),
            helius_live_rps: number("HELIUS_LIVE_RPS", 4u32).max(1),
            wormhole_rps: number("WORMHOLE_RPS", 20u32).max(1),
            upstream_permits: number("UPSTREAM_PERMITS", 16usize).max(1),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            solana_token_bridge_program: required("SOLANA_TOKEN_BRIDGE_PROGRAM")?,
            solana_ws_url: required("SOLANA_WS_URL")?,
            allowed_origins: csv_list("ALLOWED_ORIGINS"),
            ingest: IngestConfig {
                enabled: std::env::var("INGEST_ENABLED")
                    .map(|v| v != "0")
                    .unwrap_or(true),
                backfill: std::env::var("LIVE_BACKFILL")
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(true),
                backfill_max: number("INGEST_BACKFILL_MAX", 5000usize),
                backfill_pace_ms: number("INGEST_BACKFILL_PACE_MS", 150u64),
                dedupe_ttl_secs: number("INGEST_DEDUPE_TTL_SECS", 3600u64),
            },
            chain_registry_refresh_secs: number("CHAIN_REGISTRY_REFRESH_SECS", 60u64).max(5),
            reenrich: ReenrichConfig {
                enabled: std::env::var("REENRICH_ENABLED")
                    .map(|v| v != "0")
                    .unwrap_or(true),
                interval_secs: number("REENRICH_INTERVAL_SECS", 30u64).max(1),
                batch: number("REENRICH_BATCH", 50i64).max(1),
                min_age_secs: number("REENRICH_MIN_AGE_SECS", 60i64).max(0),
                retry_secs: number("REENRICH_RETRY_SECS", 120i64).max(0),
                max_age_secs: number("REENRICH_MAX_AGE_SECS", 7 * 24 * 3600i64).max(1),
            },
        })
    }
}
