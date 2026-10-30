use super::bridge::BridgeEvent;
use super::bridge_transfer::BridgeTransfer;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
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

    /// The inverse of [`Chain::wormhole_id`] for the chains this enum knows.
    pub fn from_wormhole_id(id: u16) -> Option<Self> {
        match id {
            1 => Some(Chain::Solana),
            2 => Some(Chain::Ethereum),
            4 => Some(Chain::Bsc),
            5 => Some(Chain::Polygon),
            6 => Some(Chain::Avalanche),
            23 => Some(Chain::Arbitrum),
            24 => Some(Chain::Optimism),
            25 => Some(Chain::Gnosis),
            30 => Some(Chain::Base),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum TxStatus {
    Success,
    Failed,
}

impl TxStatus {
    /// Rebuilds the enum from the label stored in `transactions.status`.
    pub fn parse(label: &str) -> Option<Self> {
        match label {
            "Success" => Some(Self::Success),
            "Failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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
