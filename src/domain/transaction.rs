use serde::Serialize;
use super::bridge::BridgeEvent;
use super::bridge_transfer::BridgeTransfer;

#[derive(Debug, Serialize)]
pub enum Chain {
    Solana,
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
    pub slot: u64,
    pub timestamp: Option<i64>,
    pub fee_lamports: u64,
    pub signer: Option<String>,
    pub bridge_event: Option<BridgeEvent>,
    pub bridge_transfer: Option<BridgeTransfer>, // new — Phase 2 result
}