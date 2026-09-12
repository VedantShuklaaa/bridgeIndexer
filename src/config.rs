#[derive(Clone, Debug)]
pub struct AppConfig {
    pub database_url: String,
    pub redis_url: String,
    pub helius_url: String,
    pub wormhole_url: String,
    pub port: u16,

    // EVM destination-chain adapter config
    pub eth_rpc_url: String,
    pub eth_token_bridge_contract: String,
    pub eth_token_bridge_deploy_block: String,

    pub bsc_rpc_url: String,
    pub bsc_token_bridge_contract: String,
    pub bsc_token_bridge_deploy_block: String,

    pub polygon_rpc_url: String,
    pub polygon_token_bridge_contract: String,
    pub polygon_token_bridge_deploy_block: String,

    pub avalanche_rpc_url: String,
    pub avalanche_token_bridge_contract: String,
    pub avalanche_token_bridge_deploy_block: String,

    pub arbitrum_rpc_url: String,
    pub arbitrum_token_bridge_contract: String,
    pub arbitrum_token_bridge_deploy_block: String,

    pub optimism_rpc_url: String,
    pub optimism_token_bridge_contract: String,
    pub optimism_token_bridge_deploy_block: String,

    pub gnosis_rpc_url: String,
    pub gnosis_token_bridge_contract: String,
    pub gnosis_token_bridge_deploy_block: String,

    pub base_rpc_url: String,
    pub base_token_bridge_contract: String,
    pub base_token_bridge_deploy_block: String,

    // Non-EVM chains (see section 4)
    pub solana_token_bridge_program: String,
    pub near_rpc_url: String,
    pub near_token_bridge_contract: String,

    pub solana_ws_url: String,

    pub allowed_origins: Vec<String>,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        // small local helper so we're not repeating .map_err(...) everywhere
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

        Ok(Self {
            database_url: required("DATABASE_URL")?,
            redis_url: required("REDIS_URL")?,
            helius_url: required("HELIUS_URL")?,
            wormhole_url: optional("WORMHOLE_URL", "https://api.wormholescan.io"),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),

            eth_rpc_url: optional("ETH_RPC_URL", "https://eth.llamarpc.com"),
            eth_token_bridge_contract: required("ETH_TOKEN_BRIDGE_CONTRACT")?,
            eth_token_bridge_deploy_block: optional("ETH_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),

            bsc_rpc_url: optional("BSC_RPC_URL", "https://bsc-dataseed.binance.org"),
            bsc_token_bridge_contract: required("BSC_TOKEN_BRIDGE_CONTRACT")?,
            bsc_token_bridge_deploy_block: optional("BSC_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),

            polygon_rpc_url: optional("POLYGON_RPC_URL", "https://polygon-rpc.com"),
            polygon_token_bridge_contract: required("POLYGON_TOKEN_BRIDGE_CONTRACT")?,
            polygon_token_bridge_deploy_block: optional("POLYGON_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),

            avalanche_rpc_url: optional(
                "AVALANCHE_RPC_URL",
                "https://api.avax.network/ext/bc/C/rpc",
            ),
            avalanche_token_bridge_contract: required("AVALANCHE_TOKEN_BRIDGE_CONTRACT")?,
            avalanche_token_bridge_deploy_block: optional(
                "AVALANCHE_TOKEN_BRIDGE_DEPLOY_BLOCK",
                "0x0",
            ),

            arbitrum_rpc_url: optional("ARBITRUM_RPC_URL", "https://arb1.arbitrum.io/rpc"),
            arbitrum_token_bridge_contract: required("ARBITRUM_TOKEN_BRIDGE_CONTRACT")?,
            arbitrum_token_bridge_deploy_block: optional(
                "ARBITRUM_TOKEN_BRIDGE_DEPLOY_BLOCK",
                "0x0",
            ),

            optimism_rpc_url: optional("OPTIMISM_RPC_URL", "https://mainnet.optimism.io"),
            optimism_token_bridge_contract: required("OPTIMISM_TOKEN_BRIDGE_CONTRACT")?,
            optimism_token_bridge_deploy_block: optional(
                "OPTIMISM_TOKEN_BRIDGE_DEPLOY_BLOCK",
                "0x0",
            ),

            gnosis_rpc_url: optional("GNOSIS_RPC_URL", "https://rpc.gnosischain.com"),
            gnosis_token_bridge_contract: required("GNOSIS_TOKEN_BRIDGE_CONTRACT")?,
            gnosis_token_bridge_deploy_block: optional("GNOSIS_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),

            base_rpc_url: optional("BASE_RPC_URL", "https://mainnet.base.org"),
            base_token_bridge_contract: required("BASE_TOKEN_BRIDGE_CONTRACT")?,
            base_token_bridge_deploy_block: optional("BASE_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),

            solana_token_bridge_program: required("SOLANA_TOKEN_BRIDGE_PROGRAM")?,
            solana_ws_url: required("SOLANA_WS_URL")?,
            near_rpc_url: optional("NEAR_RPC_URL", "https://rpc.mainnet.near.org"),
            near_token_bridge_contract: required("NEAR_TOKEN_BRIDGE_CONTRACT")?,
            allowed_origins: csv_list("ALLOWED_ORIGINS"),
        })
    }
}
