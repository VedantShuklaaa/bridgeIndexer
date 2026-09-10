use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};

use crate::domain::bridge_transfer::BridgeMessageId;
use crate::error::AppError;

use super::{ChainAdapter, DestinationTxInfo};

/// Direct-RPC adapter for EVM chains. Looks for the Wormhole Token Bridge's
/// TransferRedeemed event log, filtered by (emitterChainId, sequence).
///
/// SCAFFOLD — not verified against a live response. The event topic hash
/// and topic-decoding logic below need confirming against a real
/// eth_getLogs result before this is trustworthy.
pub struct EvmAdapter {
    client: Client,
    rpc_url: String,
    token_bridge_contract: String,
    name: &'static str,
}

impl EvmAdapter {
    pub fn new(
        client: Client,
        rpc_url: String,
        token_bridge_contract: String,
        name: &'static str,
    ) -> Self {
        Self {
            client,
            rpc_url,
            token_bridge_contract,
            name,
        }
    }
}

#[async_trait]
impl ChainAdapter for EvmAdapter {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn find_transaction(
        &self,
        message_id: &BridgeMessageId,
    ) -> Result<Option<DestinationTxInfo>, AppError> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getLogs",
            "params": [{
                "address": self.token_bridge_contract,
                "fromBlock": "earliest",
                "toBlock": "latest",
            }]
        });

        let resp = self.client.post(&self.rpc_url).json(&body).send().await?;
        let payload: Value = resp.json().await?;
        let logs = payload.get("result").and_then(Value::as_array);

        // TODO: decode each log's topics for (emitterChainId, sequence) ==
        // message_id, once the TransferRedeemed topic encoding is confirmed.
        // fromBlock: earliest will be too slow on a real contract — needs
        // a reasonable lower bound (recent block range) before this is usable.
        let _ = message_id;

        match logs {
            Some(arr) if !arr.is_empty() => Ok(Some(DestinationTxInfo {
                tx_hash: arr[0]
                    .get("transactionHash")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                wallet: None,
            })),
            _ => Ok(None),
        }
    }
}
