use std::collections::HashMap;
use std::sync::Arc;

use super::ChainAdapter;
use super::token_metadata::TokenMetadataLookup;
use crate::clients::limiter::Limiter;

/// Token metadata adapters by Wormhole chain ID, plus the cache and limiter their
/// lookups share. The registry owns the Helius API limiter so the same instance
/// pays for `getTransaction` and for token metadata.
#[derive(Clone, Default)]
pub struct AdapterRegistry {
    token_metadata: HashMap<u16, Arc<dyn ChainAdapter>>,
    token_metadata_lookup: Option<Arc<TokenMetadataLookup>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_token_metadata(&mut self, chain_id: u16, adapter: Arc<dyn ChainAdapter>) {
        self.token_metadata.insert(chain_id, adapter);
    }

    pub fn get_token_metadata(&self, chain_id: u16) -> Option<Arc<dyn ChainAdapter>> {
        self.token_metadata.get(&chain_id).cloned()
    }

    pub fn register_token_metadata_lookup(&mut self, lookup: Arc<TokenMetadataLookup>) {
        self.token_metadata_lookup = Some(lookup);
    }

    pub fn token_metadata_lookup(&self) -> Option<Arc<TokenMetadataLookup>> {
        self.token_metadata_lookup.clone()
    }

    /// The Helius API limiter the metadata lookup was built with, if any.
    pub fn helius_limiter(&self) -> Option<Arc<Limiter>> {
        self.token_metadata_lookup
            .as_ref()
            .map(|lookup| lookup.limiter())
    }
}
