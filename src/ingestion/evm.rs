use anyhow::{Context, Result, anyhow};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    db::repository::{get_last_ingested_slot, set_last_ingested_slot},
    ingestion::types::CandidateTransaction,
    redis::producer::RedisProducer,
};

const MIN_BACKFILL_BLOCK_RANGE: u64 = 5;

pub struct EvmIngester {
    chain: &'static str,
    ws_url: String,
    http_client: reqwest::Client,
    rpc_url: String,
    core_bridge_contract: String,
    log_message_published_topic: String,
    deploy_block: u64,
    block_range: u64,
    redis: RedisProducer,
    db: PgPool,
    shutdown: CancellationToken,
}

impl EvmIngester {
    pub fn new(
        chain: &'static str,
        ws_url: String,
        http_client: reqwest::Client,
        rpc_url: String,
        core_bridge_contract: String,
        log_message_published_topic: String, // pass event_topic("LogMessagePublished(address,uint64,uint32,bytes,uint8)")
        deploy_block: u64,
        block_range: u64,
        redis: RedisProducer,
        db: PgPool,
        shutdown: CancellationToken,
    ) -> Self {
        Self {
            chain,
            ws_url,
            http_client,
            rpc_url,
            core_bridge_contract,
            log_message_published_topic,
            deploy_block,
            block_range,
            redis,
            db,
            shutdown,
        }
    }

    pub async fn run(&self) -> Result<()> {
        loop {
            match self.run_connection().await {
                Ok(()) => warn!(chain = self.chain, "EVM WebSocket connection closed"),
                Err(error) => error!(chain = self.chain, %error, "EVM ingestion connection failed"),
            }

            tokio::select! {
                _ = self.shutdown.cancelled() => return Ok(()),
                _ = sleep(Duration::from_secs(5)) => {}
            }
        }
    }

    async fn run_connection(&self) -> Result<()> {
        info!(
            chain = self.chain,
            ws_url_debug = ?self.ws_url,
            "Connecting to EVM WebSocket"
        );

        let (ws_stream, _) = connect_async(&self.ws_url)
            .await
            .map_err(|error| anyhow!("failed to connect to EVM WebSocket: {error}"))?;

        info!(chain = self.chain, "Connected to EVM WebSocket");

        let (mut write, mut read) = ws_stream.split();

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_subscribe",
            "params": ["logs", {
                "address": self.core_bridge_contract,
                "topics": [self.log_message_published_topic],
            }]
        });

        write
            .send(Message::Text(request.to_string().into()))
            .await?;

        info!(chain = self.chain, contract = %self.core_bridge_contract, "Subscribed to EVM bridge logs");

        if let Err(error) = self.backfill_missed_transactions().await {
            warn!(chain = self.chain, %error, "EVM backfill failed, continuing with live ingestion");
        }

        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => {
                    warn!(chain = self.chain, "shutdown requested, closing EVM WebSocket");
                    let _ = write.send(Message::Close(None)).await;
                    return Ok(());
                }
                message = read.next() => {
                    let Some(message) = message else { break; };
                    match message.context("failed to read EVM WebSocket message")? {
                        Message::Text(text) => self.handle_message(&text).await?,
                        Message::Ping(payload) => write.send(Message::Pong(payload)).await?,
                        Message::Close(_) => { warn!(chain = self.chain, "EVM WebSocket sent close frame"); break; }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_message(&self, text: &str) -> Result<()> {
        let message: Value = serde_json::from_str(text).context("invalid EVM WebSocket JSON")?;
        let Some(log) = message.get("params").and_then(|p| p.get("result")) else {
            return Ok(());
        };
        self.handle_log(log).await
    }

    async fn handle_log(&self, log: &Value) -> Result<()> {
        if log.get("removed").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(()); // reorged out
        }

        let Some(tx_hash) = log.get("transactionHash").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(block) = log
            .get("blockNumber")
            .and_then(Value::as_str)
            .and_then(|h| u64::from_str_radix(h.trim_start_matches("0x"), 16).ok())
        else {
            return Ok(());
        };

        let candidate = CandidateTransaction {
            chain: self.chain.to_string(),
            tx_hash: tx_hash.to_string(),
            slot: block,
        };

        info!(chain = self.chain, tx_hash = %candidate.tx_hash, block, "Detected EVM bridge transaction");

        self.redis
            .publish_transaction(&candidate)
            .await
            .context("failed to publish transaction to Redis")?;
        set_last_ingested_slot(&self.db, self.chain, block)
            .await
            .context("failed to update EVM ingestion checkpoint")?;

        Ok(())
    }

    async fn latest_block(&self) -> Result<u64> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "eth_blockNumber", "params": [] });
        let resp = self
            .http_client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await?;
        let payload: Value = resp.json().await?;
        let hex = payload
            .get("result")
            .and_then(Value::as_str)
            .context("no result from eth_blockNumber")?;
        Ok(u64::from_str_radix(hex.trim_start_matches("0x"), 16)?)
    }

    async fn get_logs(&self, from: u64, to: u64) -> Result<Vec<Value>> {
        let body = json!({
            "jsonrpc": "2.0", "id": 1, "method": "eth_getLogs",
            "params": [{
                "address": self.core_bridge_contract,
                "topics": [self.log_message_published_topic],
                "fromBlock": format!("0x{:x}", from),
                "toBlock": format!("0x{:x}", to),
            }]
        });

        let resp = self
            .http_client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await?;
        let payload: Value = resp.json().await?;

        if let Some(err) = payload.get("error") {
            anyhow::bail!("eth_getLogs error: {err}");
        }

        Ok(payload
            .get("result")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    async fn backfill_missed_transactions(&self) -> Result<()> {
        let last_ingested = get_last_ingested_slot(&self.db, self.chain).await?;
        let latest = self.latest_block().await?;
        let start = last_ingested.max(self.deploy_block);

        if start >= latest {
            info!(chain = self.chain, "No EVM blocks to backfill");
            return Ok(());
        }

        info!(
            chain = self.chain,
            from = start,
            to = latest,
            "Starting EVM transaction backfill"
        );

        let mut from = start + 1;
        let mut window = self.block_range;
        let mut recovered = 0usize;

        while from <= latest {
            let to = (from + window - 1).min(latest);

            match self.get_logs(from, to).await {
                Ok(logs) => {
                    for log in &logs {
                        if log.get("removed").and_then(Value::as_bool).unwrap_or(false) {
                            continue;
                        }
                        let Some(tx_hash) = log.get("transactionHash").and_then(Value::as_str)
                        else {
                            continue;
                        };
                        let Some(block) = log
                            .get("blockNumber")
                            .and_then(Value::as_str)
                            .and_then(|h| u64::from_str_radix(h.trim_start_matches("0x"), 16).ok())
                        else {
                            continue;
                        };

                        let candidate = CandidateTransaction {
                            chain: self.chain.to_string(),
                            tx_hash: tx_hash.to_string(),
                            slot: block,
                        };

                        self.redis
                            .publish_transaction(&candidate)
                            .await
                            .context("failed to publish backfilled EVM transaction")?;
                        recovered += 1;
                        info!(
                            chain = self.chain,
                            tx_hash, block, "Published backfilled EVM transaction"
                        );
                    }

                    set_last_ingested_slot(&self.db, self.chain, to).await?;
                    from = to + 1;
                }
                Err(error) if window > MIN_BACKFILL_BLOCK_RANGE => {
                    warn!(chain = self.chain, %error, "range rejected during backfill, shrinking window");
                    window = (window / 4).max(MIN_BACKFILL_BLOCK_RANGE);
                }
                Err(error) => return Err(error),
            }
        }

        info!(
            chain = self.chain,
            recovered, "EVM transaction backfill completed"
        );
        Ok(())
    }
}
