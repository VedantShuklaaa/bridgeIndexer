#[derive(Clone, Debug)]
pub struct AppConfig {
    pub database_url: String,
    pub helius_url: String,
    pub wormhole_url: String,
    pub port: u16,

    // Phase 2 — EVM destination-chain adapter config
    pub base_rpc_url: String,
    pub base_token_bridge_contract: String,
    pub base_token_bridge_deploy_block: String, // hex string, e.g. "0x1234567"
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        Ok(Self {
            database_url: std::env::var("DATABASE_URL")
                .map_err(|_| anyhow::anyhow!("DATABASE_URL not set"))?,
            helius_url: std::env::var("HELIUS_URL")
                .map_err(|_| anyhow::anyhow!("HELIUS_URL not set"))?,
            wormhole_url: std::env::var("WORMHOLE_URL")
                .unwrap_or_else(|_| "https://api.wormholescan.io".to_string()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),

            base_rpc_url: std::env::var("BASE_RPC_URL")
                .unwrap_or_else(|_| "https://mainnet.base.org".to_string()),
            base_token_bridge_contract: std::env::var("BASE_TOKEN_BRIDGE_CONTRACT")
                .map_err(|_| anyhow::anyhow!("BASE_TOKEN_BRIDGE_CONTRACT not set"))?,
            base_token_bridge_deploy_block: std::env::var("BASE_TOKEN_BRIDGE_DEPLOY_BLOCK")
                .unwrap_or_else(|_| "0x0".to_string()),
        })
    }
}
