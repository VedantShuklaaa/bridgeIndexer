use reqwest::Client;
use std::sync::Arc;

use crate::{
    chain_adapters::{
        evm::EvmAdapter, registry::AdapterRegistry, solana::SolanaAdapter,
        wormholescan::WormholeScanAdapter,
    },
    config::AppConfig,
    domain::bridge_transfer::ChainId,
};

pub fn build_registry(config: &AppConfig, http_client: &Client) -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();

    for chain in [
        ChainId::Solana,
        ChainId::Ethereum,
        ChainId::Bsc,
        ChainId::Polygon,
        ChainId::Avalanche,
        ChainId::Near,
        ChainId::Arbitrum,
        ChainId::Optimism,
        ChainId::Gnosis,
        ChainId::Base,
    ] {
        registry.register_wormholescan(
            chain.wormhole_id(),
            Arc::new(WormholeScanAdapter::new(
                http_client.clone(),
                config.wormhole_url.clone(),
                chain_label(&chain),
            )),
        );
    }

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

    let base_adapter: Arc<EvmAdapter> = Arc::new(EvmAdapter::new(
        http_client.clone(),
        config.base_rpc_url.clone(),
        config.base_token_bridge_contract.clone(),
        config.base_token_bridge_deploy_block.clone(),
        "base",
    ));
    registry.register_evm(ChainId::Base.wormhole_id(), base_adapter.clone());
    registry.register_token_metadata(ChainId::Base.wormhole_id(), base_adapter);

    registry
}

fn chain_label(chain: &ChainId) -> &'static str {
    match chain {
        ChainId::Solana => "solana",
        ChainId::Ethereum => "ethereum",
        ChainId::Bsc => "bsc",
        ChainId::Polygon => "polygon",
        ChainId::Avalanche => "avalanche",
        ChainId::Near => "near",
        ChainId::Arbitrum => "arbitrum",
        ChainId::Optimism => "optimism",
        ChainId::Gnosis => "gnosis",
        ChainId::Base => "base",
        ChainId::Unknown(_) => "unknown",
    }
}
