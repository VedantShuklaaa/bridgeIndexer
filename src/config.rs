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
    pub port: u16,
    pub solana_token_bridge_program: String,
    pub solana_ws_url: String,
    pub allowed_origins: Vec<String>,
    /// How long ingestion keeps running after the last live client leaves.
    pub live_idle_grace_secs: u64,
    /// Replay missed signatures from the checkpoint each time ingestion starts.
    pub live_backfill: bool,
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
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            solana_token_bridge_program: required("SOLANA_TOKEN_BRIDGE_PROGRAM")?,
            solana_ws_url: required("SOLANA_WS_URL")?,
            allowed_origins: csv_list("ALLOWED_ORIGINS"),
            live_idle_grace_secs: std::env::var("LIVE_IDLE_GRACE_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(20),
            live_backfill: std::env::var("LIVE_BACKFILL")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
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
