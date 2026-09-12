use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::domain::bridge_transfer::BridgeMessageId;
use crate::error::AppError;

use super::{ChainAdapter, DestinationTxInfo};

pub struct WormholeScanAdapter {
    client: Client,
    base_url: String,
    name: &'static str,
}

impl WormholeScanAdapter {
    pub fn new(client: Client, base_url: String, name: &'static str) -> Self {
        Self {
            client,
            base_url,
            name,
        }
    }
}

#[async_trait]
impl ChainAdapter for WormholeScanAdapter {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn find_transaction(
        &self,
        message_id: &BridgeMessageId,
    ) -> Result<Option<DestinationTxInfo>, AppError> {
        let url = format!(
            "{}/api/v1/global-tx/{}/{}/{}",
            self.base_url,
            message_id.emitter_chain,
            message_id.emitter_address,
            message_id.sequence
        );

        let resp = self.client.get(&url).send().await?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            return Err(AppError::UpstreamProvider {
                provider: self.name,
                message: format!("status {}", resp.status()),
            });
        }

        let payload: Value = resp.json().await?;
        let tx_hash = payload
            .get("destinationTx")
            .and_then(|d| d.get("txHash"))
            .and_then(Value::as_str);

        Ok(tx_hash.map(|h| DestinationTxInfo {
            tx_hash: h.to_string(),
        }))
    }
}
