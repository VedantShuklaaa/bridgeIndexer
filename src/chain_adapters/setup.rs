use reqwest::Client;
use std::sync::Arc;

use crate::{
    chain_adapters::{evm::EvmAdapter, registry::AdapterRegistry, solana::SolanaAdapter},
    config::AppConfig,
    domain::bridge_transfer::ChainId,
};

pub fn build_registry(config: &AppConfig, http_client: &Client) -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();

    let solana_adapter: Arc<SolanaAdapter> = Arc::new(SolanaAdapter::new(
        http_client.clone(),
        config.helius_url.clone(),
        "solana",
    ));
    registry.register_token_metadata(ChainId::Solana.wormhole_id(), solana_adapter);

    let eth_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.eth_rpc_url.clone(),
        config.eth_token_bridge_contract.clone(),
        config.eth_token_bridge_deploy_block.clone(),
        "ethereum",
    ));
    registry.register_evm(ChainId::Ethereum.wormhole_id(), eth_adapter.clone());
    registry.register_token_metadata(ChainId::Ethereum.wormhole_id(), eth_adapter);

    let bsc_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.bsc_rpc_url.clone(),
        config.bsc_token_bridge_contract.clone(),
        config.bsc_token_bridge_deploy_block.clone(),
        "bsc",
    ));
    registry.register_evm(ChainId::Bsc.wormhole_id(), bsc_adapter.clone());
    registry.register_token_metadata(ChainId::Bsc.wormhole_id(), bsc_adapter);

    let base_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::with_block_range(
        http_client.clone(),
        config.base_rpc_url.clone(),
        config.base_token_bridge_contract.clone(),
        config.base_token_bridge_deploy_block.clone(),
        "base",
        10, // Alchemy free tier caps eth_getLogs at 10 blocks for Base
    ));
    registry.register_evm(ChainId::Base.wormhole_id(), base_adapter.clone());
    registry.register_token_metadata(ChainId::Base.wormhole_id(), base_adapter);

    let polygon_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.polygon_rpc_url.clone(),
        config.polygon_token_bridge_contract.clone(),
        config.polygon_token_bridge_deploy_block.clone(),
        "polygon",
    ));
    registry.register_evm(ChainId::Polygon.wormhole_id(), polygon_adapter.clone());
    registry.register_token_metadata(ChainId::Polygon.wormhole_id(), polygon_adapter);

    let avalanche_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.avalanche_rpc_url.clone(),
        config.avalanche_token_bridge_contract.clone(),
        config.avalanche_token_bridge_deploy_block.clone(),
        "avalanche",
    ));
    registry.register_evm(ChainId::Avalanche.wormhole_id(), avalanche_adapter.clone());
    registry.register_token_metadata(ChainId::Avalanche.wormhole_id(), avalanche_adapter);

    let arbitrum_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.arbitrum_rpc_url.clone(),
        config.arbitrum_token_bridge_contract.clone(),
        config.arbitrum_token_bridge_deploy_block.clone(),
        "arbitrum",
    ));
    registry.register_evm(ChainId::Arbitrum.wormhole_id(), arbitrum_adapter.clone());
    registry.register_token_metadata(ChainId::Arbitrum.wormhole_id(), arbitrum_adapter);

    let optimism_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.optimism_rpc_url.clone(),
        config.optimism_token_bridge_contract.clone(),
        config.optimism_token_bridge_deploy_block.clone(),
        "optimism",
    ));
    registry.register_evm(ChainId::Optimism.wormhole_id(), optimism_adapter.clone());
    registry.register_token_metadata(ChainId::Optimism.wormhole_id(), optimism_adapter);

    let gnosis_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.gnosis_rpc_url.clone(),
        config.gnosis_token_bridge_contract.clone(),
        config.gnosis_token_bridge_deploy_block.clone(),
        "gnosis",
    ));
    registry.register_evm(ChainId::Gnosis.wormhole_id(), gnosis_adapter.clone());
    registry.register_token_metadata(ChainId::Gnosis.wormhole_id(), gnosis_adapter);

    // in build_registry — add after existing chains:
    let moonbeam_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.moonbeam_rpc_url.clone(),
        config.moonbeam_token_bridge_contract.clone(),
        config.moonbeam_token_bridge_deploy_block.clone(),
        "moonbeam",
    ));
    registry.register_evm(ChainId::Moonbeam.wormhole_id(), moonbeam_adapter.clone());
    registry.register_token_metadata(ChainId::Moonbeam.wormhole_id(), moonbeam_adapter);

    let celo_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.celo_rpc_url.clone(),
        config.celo_token_bridge_contract.clone(),
        config.celo_token_bridge_deploy_block.clone(),
        "celo",
    ));
    registry.register_evm(ChainId::Celo.wormhole_id(), celo_adapter.clone());
    registry.register_token_metadata(ChainId::Celo.wormhole_id(), celo_adapter);

    let kaia_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.kaia_rpc_url.clone(),
        config.kaia_token_bridge_contract.clone(),
        config.kaia_token_bridge_deploy_block.clone(),
        "kaia",
    ));
    registry.register_evm(ChainId::Kaia.wormhole_id(), kaia_adapter.clone());
    registry.register_token_metadata(ChainId::Kaia.wormhole_id(), kaia_adapter);

    let scroll_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.scroll_rpc_url.clone(),
        config.scroll_token_bridge_contract.clone(),
        config.scroll_token_bridge_deploy_block.clone(),
        "scroll",
    ));
    registry.register_evm(ChainId::Scroll.wormhole_id(), scroll_adapter.clone());
    registry.register_token_metadata(ChainId::Scroll.wormhole_id(), scroll_adapter);

    let linea_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.linea_rpc_url.clone(),
        config.linea_token_bridge_contract.clone(),
        config.linea_token_bridge_deploy_block.clone(),
        "linea",
    ));
    registry.register_evm(ChainId::Linea.wormhole_id(), linea_adapter.clone());
    registry.register_token_metadata(ChainId::Linea.wormhole_id(), linea_adapter);

    let berachain_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.berachain_rpc_url.clone(),
        config.berachain_token_bridge_contract.clone(),
        config.berachain_token_bridge_deploy_block.clone(),
        "berachain",
    ));
    registry.register_evm(ChainId::Berachain.wormhole_id(), berachain_adapter.clone());
    registry.register_token_metadata(ChainId::Berachain.wormhole_id(), berachain_adapter);

    let seievm_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.seievm_rpc_url.clone(),
        config.seievm_token_bridge_contract.clone(),
        config.seievm_token_bridge_deploy_block.clone(),
        "seievm",
    ));
    registry.register_evm(ChainId::Seievm.wormhole_id(), seievm_adapter.clone());
    registry.register_token_metadata(ChainId::Seievm.wormhole_id(), seievm_adapter);

    let unichain_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.unichain_rpc_url.clone(),
        config.unichain_token_bridge_contract.clone(),
        config.unichain_token_bridge_deploy_block.clone(),
        "unichain",
    ));
    registry.register_evm(ChainId::Unichain.wormhole_id(), unichain_adapter.clone());
    registry.register_token_metadata(ChainId::Unichain.wormhole_id(), unichain_adapter);

    let ink_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.ink_rpc_url.clone(),
        config.ink_token_bridge_contract.clone(),
        config.ink_token_bridge_deploy_block.clone(),
        "ink",
    ));
    registry.register_evm(ChainId::Ink.wormhole_id(), ink_adapter.clone());
    registry.register_token_metadata(ChainId::Ink.wormhole_id(), ink_adapter);

    let sonic_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.sonic_rpc_url.clone(),
        config.sonic_token_bridge_contract.clone(),
        config.sonic_token_bridge_deploy_block.clone(),
        "sonic",
    ));
    registry.register_evm(ChainId::Sonic.wormhole_id(), sonic_adapter.clone());
    registry.register_token_metadata(ChainId::Sonic.wormhole_id(), sonic_adapter);

    registry
}

pub fn build_ondemand_registry(config: &AppConfig, http_client: &Client) -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();

    // Solana token metadata — same key, no separate on-demand concept
    let solana_adapter: Arc<SolanaAdapter> = Arc::new(SolanaAdapter::new(
        http_client.clone(),
        config.helius_url.clone(),
        "solana",
    ));
    registry.register_token_metadata(ChainId::Solana.wormhole_id(), solana_adapter);

    macro_rules! evm {
        ($chain:ident, $ChainId:ident, $rpc:expr, $contract:expr, $deploy:expr) => {{
            let adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
                http_client.clone(),
                $rpc,
                $contract,
                $deploy,
                stringify!($chain),
            ));
            registry.register_evm(ChainId::$ChainId.wormhole_id(), adapter.clone());
            registry.register_token_metadata(ChainId::$ChainId.wormhole_id(), adapter);
        }};
    }

    // For each chain: fall back to primary if no on-demand URL set
    let eth_url = config
        .eth_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.eth_rpc_url.clone());
    evm!(
        ethereum,
        Ethereum,
        eth_url,
        config.eth_token_bridge_contract.clone(),
        config.eth_token_bridge_deploy_block.clone()
    );

    let bsc_url = config
        .bsc_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.bsc_rpc_url.clone());
    evm!(
        bsc,
        Bsc,
        bsc_url,
        config.bsc_token_bridge_contract.clone(),
        config.bsc_token_bridge_deploy_block.clone()
    );

    let polygon_url = config
        .polygon_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.polygon_rpc_url.clone());
    evm!(
        polygon,
        Polygon,
        polygon_url,
        config.polygon_token_bridge_contract.clone(),
        config.polygon_token_bridge_deploy_block.clone()
    );

    let avalanche_url = config
        .avalanche_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.avalanche_rpc_url.clone());
    evm!(
        avalanche,
        Avalanche,
        avalanche_url,
        config.avalanche_token_bridge_contract.clone(),
        config.avalanche_token_bridge_deploy_block.clone()
    );

    let arbitrum_url = config
        .arbitrum_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.arbitrum_rpc_url.clone());
    evm!(
        arbitrum,
        Arbitrum,
        arbitrum_url,
        config.arbitrum_token_bridge_contract.clone(),
        config.arbitrum_token_bridge_deploy_block.clone()
    );

    let optimism_url = config
        .optimism_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.optimism_rpc_url.clone());
    evm!(
        optimism,
        Optimism,
        optimism_url,
        config.optimism_token_bridge_contract.clone(),
        config.optimism_token_bridge_deploy_block.clone()
    );

    let gnosis_url = config
        .gnosis_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.gnosis_rpc_url.clone());
    evm!(
        gnosis,
        Gnosis,
        gnosis_url,
        config.gnosis_token_bridge_contract.clone(),
        config.gnosis_token_bridge_deploy_block.clone()
    );

    // Base — keep the 10-block cap, just swap the RPC key
    let base_url = config
        .base_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.base_rpc_url.clone());
    let base_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::with_block_range(
        http_client.clone(),
        base_url,
        config.base_token_bridge_contract.clone(),
        config.base_token_bridge_deploy_block.clone(),
        "base",
        2000,
    ));
    registry.register_evm(ChainId::Base.wormhole_id(), base_adapter.clone());
    registry.register_token_metadata(ChainId::Base.wormhole_id(), base_adapter);

    let moonbeam_url = config
        .moonbeam_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.moonbeam_rpc_url.clone());
    let moonbeam_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        moonbeam_url,
        config.moonbeam_token_bridge_contract.clone(),
        config.moonbeam_token_bridge_deploy_block.clone(),
        "moonbeam",
    ));
    registry.register_evm(ChainId::Moonbeam.wormhole_id(), moonbeam_adapter.clone());
    registry.register_token_metadata(ChainId::Moonbeam.wormhole_id(), moonbeam_adapter);

    let celo_url = config
        .celo_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.celo_rpc_url.clone());
    let celo_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        celo_url,
        config.celo_token_bridge_contract.clone(),
        config.celo_token_bridge_deploy_block.clone(),
        "celo",
    ));
    registry.register_evm(ChainId::Celo.wormhole_id(), celo_adapter.clone());
    registry.register_token_metadata(ChainId::Celo.wormhole_id(), celo_adapter);

    let kaia_url = config
        .kaia_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.kaia_rpc_url.clone());
    let kaia_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        kaia_url,
        config.kaia_token_bridge_contract.clone(),
        config.kaia_token_bridge_deploy_block.clone(),
        "kaia",
    ));
    registry.register_evm(ChainId::Kaia.wormhole_id(), kaia_adapter.clone());
    registry.register_token_metadata(ChainId::Kaia.wormhole_id(), kaia_adapter);

    let scroll_url = config
        .scroll_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.scroll_rpc_url.clone());
    let scroll_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        scroll_url,
        config.scroll_token_bridge_contract.clone(),
        config.scroll_token_bridge_deploy_block.clone(),
        "scroll",
    ));
    registry.register_evm(ChainId::Scroll.wormhole_id(), scroll_adapter.clone());
    registry.register_token_metadata(ChainId::Scroll.wormhole_id(), scroll_adapter);

    let linea_url = config
        .linea_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.linea_rpc_url.clone());
    let linea_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        linea_url,
        config.linea_token_bridge_contract.clone(),
        config.linea_token_bridge_deploy_block.clone(),
        "linea",
    ));
    registry.register_evm(ChainId::Linea.wormhole_id(), linea_adapter.clone());
    registry.register_token_metadata(ChainId::Linea.wormhole_id(), linea_adapter);

    let berachain_url = config
        .berachain_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.berachain_rpc_url.clone());
    let berachain_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        berachain_url,
        config.berachain_token_bridge_contract.clone(),
        config.berachain_token_bridge_deploy_block.clone(),
        "berachain",
    ));
    registry.register_evm(ChainId::Berachain.wormhole_id(), berachain_adapter.clone());
    registry.register_token_metadata(ChainId::Berachain.wormhole_id(), berachain_adapter);

    let seievm_url = config
        .seievm_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.seievm_rpc_url.clone());
    let seievm_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        seievm_url,
        config.seievm_token_bridge_contract.clone(),
        config.seievm_token_bridge_deploy_block.clone(),
        "seievm",
    ));
    registry.register_evm(ChainId::Seievm.wormhole_id(), seievm_adapter.clone());
    registry.register_token_metadata(ChainId::Seievm.wormhole_id(), seievm_adapter);

    let unichain_url = config
        .unichain_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.unichain_rpc_url.clone());
    let unichain_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        unichain_url,
        config.unichain_token_bridge_contract.clone(),
        config.unichain_token_bridge_deploy_block.clone(),
        "unichain",
    ));
    registry.register_evm(ChainId::Unichain.wormhole_id(), unichain_adapter.clone());
    registry.register_token_metadata(ChainId::Unichain.wormhole_id(), unichain_adapter);

    let ink_url = config
        .ink_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.ink_rpc_url.clone());
    let ink_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        ink_url,
        config.ink_token_bridge_contract.clone(),
        config.ink_token_bridge_deploy_block.clone(),
        "ink",
    ));
    registry.register_evm(ChainId::Ink.wormhole_id(), ink_adapter.clone());
    registry.register_token_metadata(ChainId::Ink.wormhole_id(), ink_adapter);

    let sonic_url = config
        .sonic_ondemand_rpc_url
        .clone()
        .unwrap_or_else(|| config.sonic_rpc_url.clone());
    let sonic_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        sonic_url,
        config.sonic_token_bridge_contract.clone(),
        config.sonic_token_bridge_deploy_block.clone(),
        "sonic",
    ));
    registry.register_evm(ChainId::Sonic.wormhole_id(), sonic_adapter.clone());
    registry.register_token_metadata(ChainId::Sonic.wormhole_id(), sonic_adapter);

    registry
}
