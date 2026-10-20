use crate::error::AppError;
use async_trait::async_trait;

pub mod registry;
pub mod setup;
pub mod solana;

/// Per-chain lookups the pipeline cannot read from a Solana tx alone (token symbol and
/// decimals). Destination tx lookups live in `enrichment/`.
#[async_trait]
pub trait ChainAdapter: Send + Sync {
    fn name(&self) -> &'static str;

    async fn token_decimals(&self, _token_address: &str) -> Result<Option<u8>, AppError> {
        Ok(None)
    }

    async fn token_symbol(&self, _token_address: &str) -> Result<Option<String>, AppError> {
        Ok(None)
    }
}
