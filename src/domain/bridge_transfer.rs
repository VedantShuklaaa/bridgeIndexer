use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
pub enum ChainId {
    Solana,
    Ethereum,
    Base,
    Unknown(u16),
}

impl ChainId {
    pub fn from_wormhole_id(id: u16) -> Self {
        match id {
            1 => ChainId::Solana,
            2 => ChainId::Ethereum,
            30 => ChainId::Base,
            other => ChainId::Unknown(other),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum BridgeStatus {
    Detected,
    Pending,
    Completed,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeMessageId {
    pub emitter_chain: u16,
    pub emitter_address: String,
    pub sequence: u64,
}

#[derive(Debug, Serialize)]
pub struct BridgeTransfer {
    pub source_chain: ChainId,
    pub source_tx_hash: String,
    pub source_wallet: Option<String>,
    pub destination_chain: ChainId,
    pub destination_wallet: Option<String>,
    pub destination_tx_hash: Option<String>,
    pub token: Option<String>,
    pub amount: Option<String>,
    pub message_id: BridgeMessageId,
    pub status: BridgeStatus,
}
