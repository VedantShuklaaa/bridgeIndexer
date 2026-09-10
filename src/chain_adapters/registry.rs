use std::collections::HashMap;
use std::sync::Arc;

use super::ChainAdapter;

#[derive(Clone, Default)]
pub struct AdapterRegistry {
    adapters: HashMap<u16, Arc<dyn ChainAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
        }
    }

    pub fn register(&mut self, wormhole_chain_id: u16, adapter: Arc<dyn ChainAdapter>) {
        self.adapters.insert(wormhole_chain_id, adapter);
    }

    pub fn get(&self, wormhole_chain_id: u16) -> Option<Arc<dyn ChainAdapter>> {
        self.adapters.get(&wormhole_chain_id).cloned()
    }
}
