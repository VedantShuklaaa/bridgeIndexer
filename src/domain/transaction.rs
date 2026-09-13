use super::bridge::BridgeEvent;
use super::bridge_transfer::BridgeTransfer;
use serde::Serialize;

#[derive(Debug, Serialize, Clone, Copy)]
pub enum Chain {
    Solana,
    Ethereum,
    Bsc,
    Polygon,
    Avalanche,
    Arbitrum,
    Optimism,
    Gnosis,
    Base,
}

impl Chain {
    pub fn wormhole_id(&self) -> u16 {
        match self {
            Chain::Solana => 1,
            Chain::Ethereum => 2,
            Chain::Bsc => 4,
            Chain::Polygon => 5,
            Chain::Avalanche => 6,
            Chain::Arbitrum => 23,
            Chain::Optimism => 24,
            Chain::Gnosis => 25,
            Chain::Base => 30,
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "solana" => Some(Chain::Solana),
            "ethereum" => Some(Chain::Ethereum),
            "bsc" => Some(Chain::Bsc),
            "polygon" => Some(Chain::Polygon),
            "avalanche" => Some(Chain::Avalanche),
            "arbitrum" => Some(Chain::Arbitrum),
            "optimism" => Some(Chain::Optimism),
            "gnosis" => Some(Chain::Gnosis),
            "base" => Some(Chain::Base),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
pub enum TxStatus {
    Success,
    Failed,
}

#[derive(Debug, Serialize)]
pub struct NormalisedTransaction {
    pub hash: String,
    pub chain: Chain,
    pub status: TxStatus,
    pub slot: u64, // block number for EVM chains, slot for Solana
    pub timestamp: Option<i64>,
    pub fee_lamports: u64, // wei-denominated fee for EVM chains
    pub signer: Option<String>,
    pub bridge_event: Option<BridgeEvent>,
    pub bridge_transfer: Option<BridgeTransfer>,
}
