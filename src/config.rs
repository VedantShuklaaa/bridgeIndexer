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
        })
    }
}
