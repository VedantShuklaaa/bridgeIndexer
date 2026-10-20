use reqwest::Client;
use std::sync::Arc;

use crate::{
    chain_adapters::{registry::AdapterRegistry, solana::SolanaAdapter},
    config::AppConfig,
};

/// Wormhole chain ID of Solana. It is the only chain whose token metadata is looked up.
const SOLANA_WORMHOLE_ID: u16 = 1;

pub fn build_registry(config: &AppConfig, http_client: &Client) -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();

    let solana_adapter = Arc::new(SolanaAdapter::new(
        http_client.clone(),
        config.helius_url.clone(),
        "solana",
    ));
    registry.register_token_metadata(SOLANA_WORMHOLE_ID, solana_adapter);

    registry
}
