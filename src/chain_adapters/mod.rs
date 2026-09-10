pub mod evm;
pub mod registry;
pub mod wormholescan;

use async_trait::async_trait;

use crate::domain::bridge_transfer::BridgeMessageId;
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct DestinationTxInfo {
    pub tx_hash: String,
    pub wallet: Option<String>,
}

#[async_trait]
pub trait ChainAdapter: Send + Sync {
    fn name(&self) -> &'static str;

    /// `Ok(None)` = not redeemed yet (pending), not an error.
    async fn find_transaction(
        &self,
        message_id: &BridgeMessageId,
    ) -> Result<Option<DestinationTxInfo>, AppError>;
}
