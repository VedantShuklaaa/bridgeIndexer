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

    // NEW: EVM ingestion config (Core Bridge, watched for LogMessagePublished)
    pub eth_ws_url: String,
    pub eth_core_bridge_contract: String,
    pub eth_core_bridge_deploy_block: String,

    pub bsc_ws_url: String,
    pub bsc_core_bridge_contract: String,
    pub bsc_core_bridge_deploy_block: String,

    pub polygon_ws_url: String,
    pub polygon_core_bridge_contract: String,
    pub polygon_core_bridge_deploy_block: String,

    pub avalanche_ws_url: String,
    pub avalanche_core_bridge_contract: String,
    pub avalanche_core_bridge_deploy_block: String,

    pub arbitrum_ws_url: String,
    pub arbitrum_core_bridge_contract: String,
    pub arbitrum_core_bridge_deploy_block: String,

    pub optimism_ws_url: String,
    pub optimism_core_bridge_contract: String,
    pub optimism_core_bridge_deploy_block: String,

    pub base_ws_url: String,
    pub base_core_bridge_contract: String,
    pub base_core_bridge_deploy_block: String,

    // Moonbeam
    pub moonbeam_rpc_url: String,
    pub moonbeam_token_bridge_contract: String,
    pub moonbeam_token_bridge_deploy_block: String,
    pub moonbeam_ws_url: String,
    pub moonbeam_core_bridge_contract: String,
    pub moonbeam_core_bridge_deploy_block: String,

    // Celo
    pub celo_rpc_url: String,
    pub celo_token_bridge_contract: String,
    pub celo_token_bridge_deploy_block: String,
    pub celo_ws_url: String,
    pub celo_core_bridge_contract: String,
    pub celo_core_bridge_deploy_block: String,

    // Kaia
    pub kaia_rpc_url: String,
    pub kaia_token_bridge_contract: String,
    pub kaia_token_bridge_deploy_block: String,
    pub kaia_ws_url: String,
    pub kaia_core_bridge_contract: String,
    pub kaia_core_bridge_deploy_block: String,

    // Scroll
    pub scroll_rpc_url: String,
    pub scroll_token_bridge_contract: String,
    pub scroll_token_bridge_deploy_block: String,
    pub scroll_ws_url: String,
    pub scroll_core_bridge_contract: String,
    pub scroll_core_bridge_deploy_block: String,

    // Linea
    pub linea_rpc_url: String,
    pub linea_token_bridge_contract: String,
    pub linea_token_bridge_deploy_block: String,
    pub linea_ws_url: String,
    pub linea_core_bridge_contract: String,
    pub linea_core_bridge_deploy_block: String,

    // Berachain
    pub berachain_rpc_url: String,
    pub berachain_token_bridge_contract: String,
    pub berachain_token_bridge_deploy_block: String,
    pub berachain_ws_url: String,
    pub berachain_core_bridge_contract: String,
    pub berachain_core_bridge_deploy_block: String,

    // Seievm
    pub seievm_rpc_url: String,
    pub seievm_token_bridge_contract: String,
    pub seievm_token_bridge_deploy_block: String,
    pub seievm_ws_url: String,
    pub seievm_core_bridge_contract: String,
    pub seievm_core_bridge_deploy_block: String,

    // Unichain
    pub unichain_rpc_url: String,
    pub unichain_token_bridge_contract: String,
    pub unichain_token_bridge_deploy_block: String,
    pub unichain_ws_url: String,
    pub unichain_core_bridge_contract: String,
    pub unichain_core_bridge_deploy_block: String,

    // Ink
    pub ink_rpc_url: String,
    pub ink_token_bridge_contract: String,
    pub ink_token_bridge_deploy_block: String,
    pub ink_ws_url: String,
    pub ink_core_bridge_contract: String,
    pub ink_core_bridge_deploy_block: String,

    // Sonic
    pub sonic_rpc_url: String,
    pub sonic_token_bridge_contract: String,
    pub sonic_token_bridge_deploy_block: String,
    pub sonic_ws_url: String,
    pub sonic_core_bridge_contract: String,
    pub sonic_core_bridge_deploy_block: String,

    // Non-EVM chains
    pub solana_token_bridge_program: String,
    pub near_rpc_url: String,
    pub near_token_bridge_contract: String,
    pub solana_ws_url: String,

    pub allowed_origins: Vec<String>,

    //On demand
    pub eth_ondemand_rpc_url: Option<String>,
    pub bsc_ondemand_rpc_url: Option<String>,
    pub polygon_ondemand_rpc_url: Option<String>,
    pub avalanche_ondemand_rpc_url: Option<String>,
    pub arbitrum_ondemand_rpc_url: Option<String>,
    pub optimism_ondemand_rpc_url: Option<String>,
    pub gnosis_ondemand_rpc_url: Option<String>,
    pub base_ondemand_rpc_url: Option<String>,
    pub moonbeam_ondemand_rpc_url: Option<String>,
    pub celo_ondemand_rpc_url: Option<String>,
    pub kaia_ondemand_rpc_url: Option<String>,
    pub scroll_ondemand_rpc_url: Option<String>,
    pub linea_ondemand_rpc_url: Option<String>,
    pub berachain_ondemand_rpc_url: Option<String>,
    pub seievm_ondemand_rpc_url: Option<String>,
    pub unichain_ondemand_rpc_url: Option<String>,
    pub ink_ondemand_rpc_url: Option<String>,
    pub sonic_ondemand_rpc_url: Option<String>,
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
            eth_ws_url: required("ETH_WS_URL")?,
            eth_core_bridge_contract: required("ETH_CORE_BRIDGE_CONTRACT")?,
            eth_core_bridge_deploy_block: optional("ETH_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            bsc_rpc_url: optional("BSC_RPC_URL", "https://bsc-dataseed.binance.org"),
            bsc_token_bridge_contract: required("BSC_TOKEN_BRIDGE_CONTRACT")?,
            bsc_token_bridge_deploy_block: optional("BSC_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),
            bsc_ws_url: required("BSC_WS_URL")?,
            bsc_core_bridge_contract: required("BSC_CORE_BRIDGE_CONTRACT")?,
            bsc_core_bridge_deploy_block: optional("BSC_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            polygon_rpc_url: optional("POLYGON_RPC_URL", "https://polygon-rpc.com"),
            polygon_token_bridge_contract: required("POLYGON_TOKEN_BRIDGE_CONTRACT")?,
            polygon_token_bridge_deploy_block: optional("POLYGON_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),
            polygon_ws_url: required("POLYGON_WS_URL")?,
            polygon_core_bridge_contract: required("POLYGON_CORE_BRIDGE_CONTRACT")?,
            polygon_core_bridge_deploy_block: optional("POLYGON_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            avalanche_rpc_url: optional(
                "AVALANCHE_RPC_URL",
                "https://api.avax.network/ext/bc/C/rpc",
            ),
            avalanche_token_bridge_contract: required("AVALANCHE_TOKEN_BRIDGE_CONTRACT")?,
            avalanche_token_bridge_deploy_block: optional(
                "AVALANCHE_TOKEN_BRIDGE_DEPLOY_BLOCK",
                "0x0",
            ),
            avalanche_ws_url: required("AVALANCHE_WS_URL")?,
            avalanche_core_bridge_contract: required("AVALANCHE_CORE_BRIDGE_CONTRACT")?,
            avalanche_core_bridge_deploy_block: optional(
                "AVALANCHE_CORE_BRIDGE_DEPLOY_BLOCK",
                "0x0",
            ),

            arbitrum_rpc_url: optional("ARBITRUM_RPC_URL", "https://arb1.arbitrum.io/rpc"),
            arbitrum_token_bridge_contract: required("ARBITRUM_TOKEN_BRIDGE_CONTRACT")?,
            arbitrum_token_bridge_deploy_block: optional(
                "ARBITRUM_TOKEN_BRIDGE_DEPLOY_BLOCK",
                "0x0",
            ),
            arbitrum_ws_url: required("ARBITRUM_WS_URL")?,
            arbitrum_core_bridge_contract: required("ARBITRUM_CORE_BRIDGE_CONTRACT")?,
            arbitrum_core_bridge_deploy_block: optional("ARBITRUM_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            optimism_rpc_url: optional("OPTIMISM_RPC_URL", "https://mainnet.optimism.io"),
            optimism_token_bridge_contract: required("OPTIMISM_TOKEN_BRIDGE_CONTRACT")?,
            optimism_token_bridge_deploy_block: optional(
                "OPTIMISM_TOKEN_BRIDGE_DEPLOY_BLOCK",
                "0x0",
            ),
            optimism_ws_url: required("OPTIMISM_WS_URL")?,
            optimism_core_bridge_contract: required("OPTIMISM_CORE_BRIDGE_CONTRACT")?,
            optimism_core_bridge_deploy_block: optional("OPTIMISM_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            gnosis_rpc_url: optional("GNOSIS_RPC_URL", "https://rpc.gnosischain.com"),
            gnosis_token_bridge_contract: required("GNOSIS_TOKEN_BRIDGE_CONTRACT")?,
            gnosis_token_bridge_deploy_block: optional("GNOSIS_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),

            base_rpc_url: optional("BASE_RPC_URL", "https://mainnet.base.org"),
            base_token_bridge_contract: required("BASE_TOKEN_BRIDGE_CONTRACT")?,
            base_token_bridge_deploy_block: optional("BASE_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),
            base_ws_url: required("BASE_WS_URL")?,
            base_core_bridge_contract: required("BASE_CORE_BRIDGE_CONTRACT")?,
            base_core_bridge_deploy_block: optional("BASE_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            solana_token_bridge_program: required("SOLANA_TOKEN_BRIDGE_PROGRAM")?,
            solana_ws_url: required("SOLANA_WS_URL")?,
            near_rpc_url: optional("NEAR_RPC_URL", "https://rpc.mainnet.near.org"),
            near_token_bridge_contract: required("NEAR_TOKEN_BRIDGE_CONTRACT")?,
            allowed_origins: csv_list("ALLOWED_ORIGINS"),

            moonbeam_rpc_url: optional("MOONBEAM_RPC_URL", "https://rpc.api.moonbeam.network"),
            moonbeam_token_bridge_contract: required("MOONBEAM_TOKEN_BRIDGE_CONTRACT")?,
            moonbeam_token_bridge_deploy_block: optional(
                "MOONBEAM_TOKEN_BRIDGE_DEPLOY_BLOCK",
                "0x0",
            ),
            moonbeam_ws_url: required("MOONBEAM_WS_URL")?,
            moonbeam_core_bridge_contract: required("MOONBEAM_CORE_BRIDGE_CONTRACT")?,
            moonbeam_core_bridge_deploy_block: optional("MOONBEAM_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            celo_rpc_url: optional("CELO_RPC_URL", "https://forno.celo.org"),
            celo_token_bridge_contract: required("CELO_TOKEN_BRIDGE_CONTRACT")?,
            celo_token_bridge_deploy_block: optional("CELO_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),
            celo_ws_url: required("CELO_WS_URL")?,
            celo_core_bridge_contract: required("CELO_CORE_BRIDGE_CONTRACT")?,
            celo_core_bridge_deploy_block: optional("CELO_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            kaia_rpc_url: optional("KAIA_RPC_URL", "https://public-en.node.kaia.io"),
            kaia_token_bridge_contract: required("KAIA_TOKEN_BRIDGE_CONTRACT")?,
            kaia_token_bridge_deploy_block: optional("KAIA_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),
            kaia_ws_url: required("KAIA_WS_URL")?,
            kaia_core_bridge_contract: required("KAIA_CORE_BRIDGE_CONTRACT")?,
            kaia_core_bridge_deploy_block: optional("KAIA_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            scroll_rpc_url: optional("SCROLL_RPC_URL", "https://rpc.scroll.io"),
            scroll_token_bridge_contract: required("SCROLL_TOKEN_BRIDGE_CONTRACT")?,
            scroll_token_bridge_deploy_block: optional("SCROLL_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),
            scroll_ws_url: required("SCROLL_WS_URL")?,
            scroll_core_bridge_contract: required("SCROLL_CORE_BRIDGE_CONTRACT")?,
            scroll_core_bridge_deploy_block: optional("SCROLL_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            linea_rpc_url: optional("LINEA_RPC_URL", "https://rpc.linea.build"),
            linea_token_bridge_contract: required("LINEA_TOKEN_BRIDGE_CONTRACT")?,
            linea_token_bridge_deploy_block: optional("LINEA_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),
            linea_ws_url: required("LINEA_WS_URL")?,
            linea_core_bridge_contract: required("LINEA_CORE_BRIDGE_CONTRACT")?,
            linea_core_bridge_deploy_block: optional("LINEA_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            berachain_rpc_url: optional("BERACHAIN_RPC_URL", "https://rpc.berachain.com"),
            berachain_token_bridge_contract: required("BERACHAIN_TOKEN_BRIDGE_CONTRACT")?,
            berachain_token_bridge_deploy_block: optional(
                "BERACHAIN_TOKEN_BRIDGE_DEPLOY_BLOCK",
                "0x0",
            ),
            berachain_ws_url: required("BERACHAIN_WS_URL")?,
            berachain_core_bridge_contract: required("BERACHAIN_CORE_BRIDGE_CONTRACT")?,
            berachain_core_bridge_deploy_block: optional(
                "BERACHAIN_CORE_BRIDGE_DEPLOY_BLOCK",
                "0x0",
            ),

            seievm_rpc_url: optional("SEIEVM_RPC_URL", "https://evm-rpc.sei-apis.com"),
            seievm_token_bridge_contract: required("SEIEVM_TOKEN_BRIDGE_CONTRACT")?,
            seievm_token_bridge_deploy_block: optional("SEIEVM_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),
            seievm_ws_url: required("SEIEVM_WS_URL")?,
            seievm_core_bridge_contract: required("SEIEVM_CORE_BRIDGE_CONTRACT")?,
            seievm_core_bridge_deploy_block: optional("SEIEVM_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            unichain_rpc_url: optional("UNICHAIN_RPC_URL", "https://mainnet.unichain.org"),
            unichain_token_bridge_contract: required("UNICHAIN_TOKEN_BRIDGE_CONTRACT")?,
            unichain_token_bridge_deploy_block: optional(
                "UNICHAIN_TOKEN_BRIDGE_DEPLOY_BLOCK",
                "0x0",
            ),
            unichain_ws_url: required("UNICHAIN_WS_URL")?,
            unichain_core_bridge_contract: required("UNICHAIN_CORE_BRIDGE_CONTRACT")?,
            unichain_core_bridge_deploy_block: optional("UNICHAIN_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            ink_rpc_url: optional("INK_RPC_URL", "https://rpc-gel.inkonchain.com"),
            ink_token_bridge_contract: required("INK_TOKEN_BRIDGE_CONTRACT")?,
            ink_token_bridge_deploy_block: optional("INK_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),
            ink_ws_url: required("INK_WS_URL")?,
            ink_core_bridge_contract: required("INK_CORE_BRIDGE_CONTRACT")?,
            ink_core_bridge_deploy_block: optional("INK_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            sonic_rpc_url: optional("SONIC_RPC_URL", "https://rpc.soniclabs.com"),
            sonic_token_bridge_contract: required("SONIC_TOKEN_BRIDGE_CONTRACT")?,
            sonic_token_bridge_deploy_block: optional("SONIC_TOKEN_BRIDGE_DEPLOY_BLOCK", "0x0"),
            sonic_ws_url: required("SONIC_WS_URL")?,
            sonic_core_bridge_contract: required("SONIC_CORE_BRIDGE_CONTRACT")?,
            sonic_core_bridge_deploy_block: optional("SONIC_CORE_BRIDGE_DEPLOY_BLOCK", "0x0"),

            moonbeam_ondemand_rpc_url: std::env::var("MOONBEAM_ONDEMAND_RPC_URL").ok(),
            celo_ondemand_rpc_url: std::env::var("CELO_ONDEMAND_RPC_URL").ok(),
            kaia_ondemand_rpc_url: std::env::var("KAIA_ONDEMAND_RPC_URL").ok(),
            scroll_ondemand_rpc_url: std::env::var("SCROLL_ONDEMAND_RPC_URL").ok(),
            linea_ondemand_rpc_url: std::env::var("LINEA_ONDEMAND_RPC_URL").ok(),
            berachain_ondemand_rpc_url: std::env::var("BERACHAIN_ONDEMAND_RPC_URL").ok(),
            seievm_ondemand_rpc_url: std::env::var("SEIEVM_ONDEMAND_RPC_URL").ok(),
            unichain_ondemand_rpc_url: std::env::var("UNICHAIN_ONDEMAND_RPC_URL").ok(),
            ink_ondemand_rpc_url: std::env::var("INK_ONDEMAND_RPC_URL").ok(),
            sonic_ondemand_rpc_url: std::env::var("SONIC_ONDEMAND_RPC_URL").ok(),
            eth_ondemand_rpc_url: std::env::var("ETH_ONDEMAND_RPC_URL").ok(),
            bsc_ondemand_rpc_url: std::env::var("BSC_ONDEMAND_RPC_URL").ok(),
            polygon_ondemand_rpc_url: std::env::var("POLYGON_ONDEMAND_RPC_URL").ok(),
            avalanche_ondemand_rpc_url: std::env::var("AVALANCHE_ONDEMAND_RPC_URL").ok(),
            arbitrum_ondemand_rpc_url: std::env::var("ARBITRUM_ONDEMAND_RPC_URL").ok(),
            optimism_ondemand_rpc_url: std::env::var("OPTIMISM_ONDEMAND_RPC_URL").ok(),
            gnosis_ondemand_rpc_url: std::env::var("GNOSIS_ONDEMAND_RPC_URL").ok(),
            base_ondemand_rpc_url: std::env::var("BASE_ONDEMAND_RPC_URL").ok(),
        })
    }

    pub fn rpc_url_for_chain(&self, chain: &str) -> anyhow::Result<&str> {
        Ok(match chain {
            "ethereum" => &self.eth_rpc_url,
            "bsc" => &self.bsc_rpc_url,
            "polygon" => &self.polygon_rpc_url,
            "avalanche" => &self.avalanche_rpc_url,
            "arbitrum" => &self.arbitrum_rpc_url,
            "optimism" => &self.optimism_rpc_url,
            "gnosis" => &self.gnosis_rpc_url,
            "base" => &self.base_rpc_url,
            "moonbeam" => &self.moonbeam_rpc_url,
            "celo" => &self.celo_rpc_url,
            "kaia" => &self.kaia_rpc_url,
            "scroll" => &self.scroll_rpc_url,
            "linea" => &self.linea_rpc_url,
            "berachain" => &self.berachain_rpc_url,
            "seievm" => &self.seievm_rpc_url,
            "unichain" => &self.unichain_rpc_url,
            "ink" => &self.ink_rpc_url,
            "sonic" => &self.sonic_rpc_url,
            other => anyhow::bail!("no RPC URL configured for chain: {other}"),
        })
    }

    pub fn core_bridge_contract_for_chain(&self, chain: &str) -> anyhow::Result<&str> {
        Ok(match chain {
            "ethereum" => &self.eth_core_bridge_contract,
            "bsc" => &self.bsc_core_bridge_contract,
            "polygon" => &self.polygon_core_bridge_contract,
            "avalanche" => &self.avalanche_core_bridge_contract,
            "arbitrum" => &self.arbitrum_core_bridge_contract,
            "optimism" => &self.optimism_core_bridge_contract,
            "base" => &self.base_core_bridge_contract,
            "moonbeam" => &self.moonbeam_core_bridge_contract,
            "celo" => &self.celo_core_bridge_contract,
            "kaia" => &self.kaia_core_bridge_contract,
            "scroll" => &self.scroll_core_bridge_contract,
            "linea" => &self.linea_core_bridge_contract,
            "berachain" => &self.berachain_core_bridge_contract,
            "seievm" => &self.seievm_core_bridge_contract,
            "unichain" => &self.unichain_core_bridge_contract,
            "ink" => &self.ink_core_bridge_contract,
            "sonic" => &self.sonic_core_bridge_contract,
            other => anyhow::bail!("no core bridge contract configured for chain: {other}"),
        })
    }

    pub fn ondemand_rpc_url_for_chain(&self, chain: &str) -> anyhow::Result<&str> {
        let ondemand = match chain {
            "ethereum" => self.eth_ondemand_rpc_url.as_deref(),
            "bsc" => self.bsc_ondemand_rpc_url.as_deref(),
            "polygon" => self.polygon_ondemand_rpc_url.as_deref(),
            "avalanche" => self.avalanche_ondemand_rpc_url.as_deref(),
            "arbitrum" => self.arbitrum_ondemand_rpc_url.as_deref(),
            "optimism" => self.optimism_ondemand_rpc_url.as_deref(),
            "gnosis" => self.gnosis_ondemand_rpc_url.as_deref(),
            "base" => self.base_ondemand_rpc_url.as_deref(),
            "moonbeam" => self.moonbeam_ondemand_rpc_url.as_deref(),
            "celo" => self.celo_ondemand_rpc_url.as_deref(),
            "kaia" => self.kaia_ondemand_rpc_url.as_deref(),
            "scroll" => self.scroll_ondemand_rpc_url.as_deref(),
            "linea" => self.linea_ondemand_rpc_url.as_deref(),
            "berachain" => self.berachain_ondemand_rpc_url.as_deref(),
            "seievm" => self.seievm_ondemand_rpc_url.as_deref(),
            "unichain" => self.unichain_ondemand_rpc_url.as_deref(),
            "ink" => self.ink_ondemand_rpc_url.as_deref(),
            "sonic" => self.sonic_ondemand_rpc_url.as_deref(),
            other => anyhow::bail!("no RPC URL configured for chain: {other}"),
        };
        // fall back to primary if no dedicated key
        let url = ondemand.unwrap_or(self.rpc_url_for_chain(chain)?);
        tracing::debug!(chain, url, "ondemand rpc url selected");
        Ok(ondemand.unwrap_or(self.rpc_url_for_chain(chain)?))
    }
}
