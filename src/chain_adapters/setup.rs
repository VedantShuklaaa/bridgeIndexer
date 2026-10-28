use governor::{Quota, RateLimiter};
use reqwest::Client;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::{
    chain_adapters::{
        registry::AdapterRegistry, solana::SolanaAdapter, token_metadata::TokenMetadataLookup,
    },
    clients::limiter::Limiter,
    config::AppConfig,
};

/// Wormhole chain ID of Solana. It is the only chain whose token metadata is looked up.
const SOLANA_WORMHOLE_ID: u16 = 1;

/// Builds the token metadata adapters and the Helius limiter they share with the
/// state. `helius_rps` is the plan quota for this role: the API process passes
/// `HELIUS_API_RPS`, the live pipeline passes `HELIUS_LIVE_RPS`.
pub fn build_registry(
    config: &AppConfig,
    http_client: &Client,
    helius_rps: u32,
) -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();

    let helius_limiter: Arc<Limiter> = Arc::new(RateLimiter::direct(Quota::per_second(
        NonZeroU32::new(helius_rps).unwrap(),
    )));

    let solana_adapter = Arc::new(SolanaAdapter::new(
        http_client.clone(),
        config.helius_url.clone(),
        "solana",
    ));
    registry.register_token_metadata(SOLANA_WORMHOLE_ID, solana_adapter);
    registry.register_token_metadata_lookup(Arc::new(TokenMetadataLookup::new(helius_limiter)));

    registry
}
