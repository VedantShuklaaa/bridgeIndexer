use std::collections::HashMap;
use std::sync::Arc;

use super::ChainAdapter;

#[derive(Clone, Default)]
pub struct AdapterRegistry {
    wormholescan: HashMap<u16, Arc<dyn ChainAdapter>>,
    evm: HashMap<u16, Arc<dyn ChainAdapter>>,
    token_metadata: HashMap<u16, Arc<dyn ChainAdapter>>, // NEW — keyed by token_chain
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self {
            wormholescan: HashMap::new(),
            evm: HashMap::new(),
            token_metadata: HashMap::new(),
        }
    }

    pub fn register_wormholescan(&mut self, chain_id: u16, adapter: Arc<dyn ChainAdapter>) {
        self.wormholescan.insert(chain_id, adapter);
    }

    pub fn register_evm(&mut self, chain_id: u16, adapter: Arc<dyn ChainAdapter>) {
        self.evm.insert(chain_id, adapter);
    }

    pub fn register_token_metadata(&mut self, chain_id: u16, adapter: Arc<dyn ChainAdapter>) {
        self.token_metadata.insert(chain_id, adapter);
    }

    pub fn get_wormholescan(&self, chain_id: u16) -> Option<Arc<dyn ChainAdapter>> {
        self.wormholescan.get(&chain_id).cloned()
    }

    pub fn get_evm(&self, chain_id: u16) -> Option<Arc<dyn ChainAdapter>> {
        self.evm.get(&chain_id).cloned()
    }

    pub fn get_token_metadata(&self, chain_id: u16) -> Option<Arc<dyn ChainAdapter>> {
        self.token_metadata.get(&chain_id).cloned()
    }
}
