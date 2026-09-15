use async_trait::async_trait;
use reqwest::Client;
use serde_json::{Value, json};
use sha3::{Digest, Keccak256};

use crate::domain::bridge_transfer::BridgeMessageId;
use crate::error::AppError;

use super::{ChainAdapter, DestinationTxInfo};

const DEFAULT_BLOCK_RANGE: u64 = 2000;
const MIN_BLOCK_RANGE: u64 = 5;
const MAX_CHUNKS: u64 = 2000; // raised since ranges can now shrink a lot on free tiers

pub struct EvmAdapter {
    client: Client,
    rpc_url: String,
    token_bridge_contract: String,
    deploy_block: u64,
    name: &'static str,
    block_range: u64,
}

impl EvmAdapter {
    pub fn new(
        client: Client,
        rpc_url: String,
        token_bridge_contract: String,
        from_block_hex: String,
        name: &'static str,
    ) -> Self {
        Self::with_block_range(
            client,
            rpc_url,
            token_bridge_contract,
            from_block_hex,
            name,
            DEFAULT_BLOCK_RANGE,
        )
    }

    /// Use this for chains/providers with a tighter eth_getLogs range limit
    /// (e.g. Base on Alchemy's free tier only allows 10 blocks per call).
    pub fn with_block_range(
        client: Client,
        rpc_url: String,
        token_bridge_contract: String,
        from_block_hex: String,
        name: &'static str,
        block_range: u64,
    ) -> Self {
        let deploy_block =
            u64::from_str_radix(from_block_hex.trim_start_matches("0x"), 16).unwrap_or(0);

        Self {
            client,
            rpc_url,
            token_bridge_contract,
            deploy_block,
            name,
            block_range: block_range.max(1),
        }
    }

    async fn latest_block(&self) -> Result<u64, AppError> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "eth_blockNumber", "params": [] });
        let resp = self.client.post(&self.rpc_url).json(&body).send().await?;
        let payload: Value = resp.json().await?;

        let hex = payload
            .get("result")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::UpstreamProvider {
                provider: self.name.to_string(),
                message: "no result from eth_blockNumber".into(),
            })?;

        u64::from_str_radix(hex.trim_start_matches("0x"), 16)
            .map_err(|e| AppError::Normalisation(format!("bad block number: {e}")))
    }

    async fn get_logs_in_range(
        &self,
        from: u64,
        to: u64,
        topics: &[String; 4],
    ) -> Result<Vec<Value>, AppError> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getLogs",
            "params": [{
                "address": self.token_bridge_contract,
                "topics": topics,
                "fromBlock": format!("0x{:x}", from),
                "toBlock": format!("0x{:x}", to),
            }]
        });

        let resp = self.client.post(&self.rpc_url).json(&body).send().await?;
        let payload: Value = resp.json().await?;

        if let Some(err) = payload.get("error") {
            return Err(AppError::UpstreamProvider {
                provider: self.name.to_string(),
                message: err.to_string(),
            });
        }

        Ok(payload
            .get("result")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Fetches logs for [from, to], automatically shrinking the range and
    /// retrying if the provider rejects it as too wide (e.g. free-tier caps).
    async fn get_logs_resilient(
        &self,
        from: u64,
        to: u64,
        topics: &[String; 4],
    ) -> Result<Vec<Value>, AppError> {
        let mut window = to - from + 1;
        let cursor_to = to;

        loop {
            let cursor_from = cursor_to.saturating_sub(window - 1).max(from);

            match self.get_logs_in_range(cursor_from, cursor_to, topics).await {
                Ok(logs) => {
                    if cursor_from == from {
                        return Ok(logs);
                    }
                    // shrunk below the original request but succeeded on this
                    // sub-window; caller's outer loop handles stepping further back
                    return Ok(logs);
                }
                Err(AppError::UpstreamProvider { message, .. })
                    if window > MIN_BLOCK_RANGE
                        && (message.contains("-32600")
                            || message.to_lowercase().contains("block range")) =>
                {
                    window = (window / 4).max(MIN_BLOCK_RANGE);
                    tracing::warn!(
                        provider = self.name,
                        new_window = window,
                        "eth_getLogs range rejected, shrinking and retrying"
                    );
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

pub fn event_topic(signature: &str) -> String {
    format!("0x{}", hex::encode(Keccak256::digest(signature.as_bytes())))
}

fn pad_u16(v: u16) -> String {
    let mut buf = [0u8; 32];
    buf[30..32].copy_from_slice(&v.to_be_bytes());
    format!("0x{}", hex::encode(buf))
}

fn pad_u64(v: u64) -> String {
    let mut buf = [0u8; 32];
    buf[24..32].copy_from_slice(&v.to_be_bytes());
    format!("0x{}", hex::encode(buf))
}

fn decode_decimals(hex_result: &str) -> Option<u8> {
    let bytes = hex::decode(hex_result.trim_start_matches("0x")).ok()?;
    bytes.last().copied()
}

fn decode_symbol(hex_result: &str) -> Option<String> {
    let bytes = hex::decode(hex_result.trim_start_matches("0x")).ok()?;

    if bytes.len() == 32 {
        return String::from_utf8(bytes.iter().copied().take_while(|b| *b != 0).collect())
            .ok()
            .filter(|s| !s.is_empty());
    }

    if bytes.len() >= 64 {
        let offset = u64::from_be_bytes(bytes[0..32].get(24..32)?.try_into().ok()?) as usize;

        if offset + 32 > bytes.len() {
            return None;
        }

        let len =
            u64::from_be_bytes(bytes[offset..offset + 32].get(24..32)?.try_into().ok()?) as usize;

        let start = offset + 32;
        let end = start.checked_add(len)?;

        let data = bytes.get(start..end)?;

        return String::from_utf8(data.to_vec())
            .ok()
            .filter(|s| !s.is_empty());
    }

    None
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
        let topics = [
            event_topic("TransferRedeemed(uint16,bytes32,uint64)"),
            pad_u16(message_id.emitter_chain),
            format!("0x{}", message_id.emitter_address),
            pad_u64(message_id.sequence),
        ];

        let latest = self.latest_block().await?;
        let mut to = latest;

        for _ in 0..MAX_CHUNKS {
            let from = to
                .saturating_sub(self.block_range.saturating_sub(1))
                .max(self.deploy_block);
            let logs = self.get_logs_resilient(from, to, &topics).await?;

            if !logs.is_empty() {
                let tx_hash = logs[0]
                    .get("transactionHash")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                return Ok(Some(DestinationTxInfo { tx_hash }));
            }

            if from <= self.deploy_block {
                break;
            }
            to = from.saturating_sub(1);
        }

        Ok(None)
    }

    async fn token_decimals(&self, token_address: &str) -> Result<Option<u8>, AppError> {
        let body = json!({
            "jsonrpc": "2.0", "id": 1, "method": "eth_call",
            "params": [{ "to": token_address, "data": "0x313ce567" }, "latest"]
        });
        let resp = self.client.post(&self.rpc_url).json(&body).send().await?;
        let payload: Value = resp.json().await?;
        Ok(payload
            .get("result")
            .and_then(Value::as_str)
            .and_then(decode_decimals))
    }

    async fn token_symbol(&self, token_address: &str) -> Result<Option<String>, AppError> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_call",
            "params": [{
                "to": token_address,
                "data": "0x95d89b41"
            }, "latest"]
        });

        let resp = self.client.post(&self.rpc_url).json(&body).send().await?;

        let payload: Value = resp.json().await?;

        Ok(payload
            .get("result")
            .and_then(Value::as_str)
            .and_then(decode_symbol))
    }
}
