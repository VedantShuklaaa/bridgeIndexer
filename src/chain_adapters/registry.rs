use std::collections::HashMap;
use std::sync::Arc;

use super::ChainAdapter;

/// Token metadata adapters by Wormhole chain ID.
#[derive(Clone, Default)]
pub struct AdapterRegistry {
    token_metadata: HashMap<u16, Arc<dyn ChainAdapter>>,
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
}
