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

    registry
}
