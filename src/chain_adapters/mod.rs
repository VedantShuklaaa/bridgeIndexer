use crate::domain::bridge_transfer::BridgeMessageId;
use crate::error::AppError;
use async_trait::async_trait;

pub mod evm;
pub mod registry;
pub mod setup;
pub mod solana;
pub mod wormholescan;

#[async_trait]
pub trait ChainAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    async fn find_transaction(
        &self,
        message_id: &BridgeMessageId,
    ) -> Result<Option<DestinationTxInfo>, AppError>;

    async fn token_decimals(&self, _token_address: &str) -> Result<Option<u8>, AppError> {
        Ok(None)
    }
    
    async fn token_symbol(&self, _token_address: &str) -> Result<Option<String>, AppError> {
        Ok(None)
    }
}

pub struct DestinationTxInfo {
    pub tx_hash: String,
}
