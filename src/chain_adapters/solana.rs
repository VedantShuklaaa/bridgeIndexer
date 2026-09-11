use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};

use crate::domain::bridge_transfer::BridgeMessageId;
use crate::error::AppError;

use super::{ChainAdapter, DestinationTxInfo};

pub struct SolanaAdapter {
    client: Client,
    helius_url: String,
    name: &'static str,
}

impl SolanaAdapter {
    pub fn new(client: Client, helius_url: String, name: &'static str) -> Self {
        Self {
            client,
            helius_url,
            name,
        }
    }

    async fn get_asset(&self, mint: &str) -> Result<Value, AppError> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": "token-metadata",
            "method": "getAsset",
            "params": {
                "id": mint,
                "displayOptions": { "showFungible": true }
            }
        });

        let resp = self
            .client
            .post(&self.helius_url)
            .json(&body)
            .send()
            .await?;
        let payload: Value = resp.json().await?;

        if let Some(err) = payload.get("error") {
            return Err(AppError::UpstreamProvider {
                provider: self.name,
                message: err.to_string(),
            });
        }

        Ok(payload.get("result").cloned().unwrap_or(Value::Null))
    }
}

#[async_trait]
impl ChainAdapter for SolanaAdapter {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn find_transaction(
        &self,
        _message_id: &BridgeMessageId,
    ) -> Result<Option<DestinationTxInfo>, AppError> {
        Ok(None)
    }

    async fn token_decimals(&self, token_address: &str) -> Result<Option<u8>, AppError> {
        let asset = self.get_asset(token_address).await?;
        Ok(asset
            .get("token_info")
            .and_then(|t| t.get("decimals"))
            .and_then(Value::as_u64)
            .map(|d| d as u8))
    }

    async fn token_symbol(&self, token_address: &str) -> Result<Option<String>, AppError> {
        let asset = self.get_asset(token_address).await?;

        let symbol = asset
            .get("token_info")
            .and_then(|t| t.get("symbol"))
            .and_then(Value::as_str)
            .or_else(|| {
                asset
                    .get("content")
                    .and_then(|c| c.get("metadata"))
                    .and_then(|m| m.get("symbol"))
                    .and_then(Value::as_str)
            })
            .map(str::to_string)
            .filter(|s| !s.is_empty());

        Ok(symbol)
    }
}
