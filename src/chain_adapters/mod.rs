use crate::domain::bridge_transfer::BridgeMessageId;
use crate::error::AppError;
use async_trait::async_trait;

pub mod evm;
pub mod registry;
pub mod wormholescan;
pub mod setup;

#[derive(Debug, Clone)]
pub struct DestinationTxInfo {
    pub tx_hash: String,
}

#[async_trait]
pub trait ChainAdapter: Send + Sync {
    fn name(&self) -> &'static str;

    async fn find_transaction(
        &self,
        message_id: &BridgeMessageId,
    ) -> Result<Option<DestinationTxInfo>, AppError>;

    /// On-chain token decimals, if this adapter can fetch them (EVM-only for now).
    /// Default: not supported.
    async fn token_decimals(&self, _token_address: &str) -> Result<Option<u8>, AppError> {
        Ok(None)
    }

    /// On-chain token symbol, if this adapter can fetch it.
    async fn token_symbol(&self, _token_address: &str) -> Result<Option<String>, AppError> {
        Ok(None)
    }
}
