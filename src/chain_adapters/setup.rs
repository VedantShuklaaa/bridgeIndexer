use reqwest::Client;
use std::sync::Arc;

use crate::{
    chain_adapters::{
        registry::AdapterRegistry, solana::SolanaAdapter, wormholescan::WormholeScanAdapter,
    },
    config::AppConfig,
    domain::bridge_transfer::ChainId,
};

pub fn build_registry(config: &AppConfig, http_client: &Client) -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();

    // Destination ("X") lookups go through WormholeScan for any chain, so the
    // X leg stays data-driven even though only Solana is ingested.
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

    let solana_adapter = Arc::new(SolanaAdapter::new(
        http_client.clone(),
        config.helius_url.clone(),
        "solana",
    ));
    registry.register_token_metadata(ChainId::Solana.wormhole_id(), solana_adapter);

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
