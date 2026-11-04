use governor::{Quota, RateLimiter};
use reqwest::Client;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::{
    chain_adapters::{
        registry::AdapterRegistry, solana::SolanaAdapter, token_metadata::TokenMetadataLookup,
    },
    clients::limiter::Limiter,
    config::{AppConfig, Lane},
};

/// Wormhole chain ID of Solana. It is the only chain whose token metadata is looked up.
const SOLANA_WORMHOLE_ID: u16 = 1;

/// state, for one provider lane. The lane picks the endpoint (`HELIUS_API_URL` or
/// `HELIUS_STREAM_URL`) and the quota (`HELIUS_API_RPS` or `HELIUS_LIVE_RPS`), so a
/// lane's adapters and limiter can never point at the other lane's key.
pub fn build_registry(config: &AppConfig, http_client: &Client, lane: Lane) -> AdapterRegistry {
    let mut registry = AdapterRegistry::new();

    let helius_limiter: Arc<Limiter> = Arc::new(RateLimiter::direct(Quota::per_second(
        NonZeroU32::new(config.helius_rps(lane)).unwrap(),
    )));

    let solana_adapter = Arc::new(SolanaAdapter::new(
        http_client.clone(),
        config.helius_url(lane).to_string(),
        "solana",
    ));
    registry.register_token_metadata(SOLANA_WORMHOLE_ID, solana_adapter);
    registry.register_token_metadata_lookup(Arc::new(TokenMetadataLookup::new(helius_limiter)));

    registry
}
