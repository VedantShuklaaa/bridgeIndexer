use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeEvent {
    pub sequence: u64,
    pub emitter_chain: String,
    pub emitter_address: String,
    pub target_chain: Option<String>,
    pub amount: Option<String>,
}
