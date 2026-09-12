use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::{
    clients::helius::get_signatures_for_address,
    db::repository::{get_last_ingested_slot, set_last_ingested_slot},
    ingestion::types::{LogsConfig, LogsFilter, SolanaRpcRequest},
    redis::producer::RedisProducer,
};

#[derive(Debug, Clone)]
pub struct CandidateTransaction {
    pub chain: String,
    pub tx_hash: String,
    pub slot: u64,
}

pub struct SolanaIngester {
    ws_url: String,
    bridge_program: String,
    redis: RedisProducer,
    db: PgPool,
    client: reqwest::Client,
    helius_url: String,
}

impl SolanaIngester {
    pub fn new(
        ws_url: String,
        bridge_program: String,
        redis: RedisProducer,
        db: PgPool,
        client: reqwest::Client,
        helius_url: String,
    ) -> Self {
        Self {
            ws_url,
            bridge_program,
            redis,
            db,
            client,
            helius_url,
        }
    }

    pub async fn run(&self) -> Result<()> {
        loop {
            match self.run_connection().await {
                Ok(()) => {
                    warn!("Solana WebSocket connection closed");
                }
                Err(error) => {
                    error!(%error, "Solana ingestion connection failed");
                }
            }

            info!("Reconnecting to Solana WebSocket in 5 seconds...");
            sleep(Duration::from_secs(5)).await;
        }
    }

    async fn run_connection(&self) -> Result<()> {
        info!(
            ws_url = %self.ws_url,
            bridge_program = %self.bridge_program,
            "Connecting to Solana WebSocket"
        );

        let (ws_stream, _) = connect_async(&self.ws_url)
            .await
            .context("failed to connect to Solana WebSocket")?;

        info!("Connected to Solana WebSocket");

        let (mut write, mut read) = ws_stream.split();

        let request = SolanaRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "logsSubscribe",
            params: (
                LogsFilter {
                    mentions: vec![self.bridge_program.clone()],
                },
                LogsConfig {
                    commitment: "confirmed",
                },
            ),
        };

        write
            .send(Message::Text(serde_json::to_string(&request)?.into()))
            .await?;

        info!(
            program = %self.bridge_program,
            "Subscribed to Solana bridge logs"
        );

        self.backfill_missed_transactions().await?;

        while let Some(message) = read.next().await {
            let message = message.context("failed to read Solana WebSocket message")?;

            match message {
                Message::Text(text) => {
                    self.handle_message(&text).await?;
                }

                Message::Ping(payload) => {
                    info!("Received Solana WebSocket ping");
                    write.send(Message::Pong(payload)).await?;
                }

                Message::Close(_) => {
                    warn!("Solana WebSocket sent close frame");
                    break;
                }

                _ => {}
            }
        }

        Ok(())
    }

    async fn backfill_missed_transactions(&self) -> Result<()> {
        let last_ingested_slot = get_last_ingested_slot(&self.db).await?;

        info!(last_ingested_slot, "Starting Solana transaction backfill");

        let mut before: Option<String> = None;
        let mut recovered = 0usize;

        loop {
            let signatures = get_signatures_for_address(
                &self.client,
                &self.helius_url,
                &self.bridge_program,
                before.clone(),
            )
            .await?;

            if signatures.is_empty() {
                break;
            }

            let mut reached_checkpoint = false;

            for signature_info in &signatures {
                let slot = match signature_info.get("slot").and_then(Value::as_u64) {
                    Some(slot) => slot,
                    None => continue,
                };

                if slot <= last_ingested_slot {
                    reached_checkpoint = true;
                    break;
                }

                let err = signature_info.get("err");

                if err.is_some_and(|value| !value.is_null()) {
                    continue;
                }

                let signature = match signature_info.get("signature").and_then(Value::as_str) {
                    Some(signature) => signature,
                    None => continue,
                };

                let candidate = CandidateTransaction {
                    chain: "solana".to_string(),
                    tx_hash: signature.to_string(),
                    slot,
                };

                self.redis
                    .publish_transaction(&candidate)
                    .await
                    .context("failed to publish backfilled transaction")?;

                recovered += 1;

                info!(
                    tx_hash = %candidate.tx_hash,
                    slot = candidate.slot,
                    "Published backfilled transaction"
                );
            }

            if reached_checkpoint {
                break;
            }

            before = signatures
                .last()
                .and_then(|value| value.get("signature").and_then(Value::as_str))
                .map(str::to_string);
        }

        info!(
            recovered,
            last_ingested_slot, "Solana transaction backfill completed"
        );

        Ok(())
    }

    async fn handle_message(&self, text: &str) -> Result<()> {
        let message: Value = serde_json::from_str(text).context("invalid Solana WebSocket JSON")?;
        if message.get("method").and_then(Value::as_str) != Some("logsNotification") {
            return Ok(());
        }

        let params = match message.get("params") {
            Some(params) => params,
            None => return Ok(()),
        };

        let result = match params.get("result") {
            Some(result) => result,
            None => return Ok(()),
        };

        let context = match result.get("context") {
            Some(context) => context,
            None => return Ok(()),
        };

        let slot = context
            .get("slot")
            .and_then(Value::as_u64)
            .context("missing slot in Solana log notification")?;

        let value = match result.get("value") {
            Some(value) => value,
            None => return Ok(()),
        };

        let signature = match value.get("signature").and_then(Value::as_str) {
            Some(signature) => signature,
            None => return Ok(()),
        };

        let err = value.get("err");

        if err.is_some_and(|value| !value.is_null()) {
            return Ok(());
        }

        let candidate = CandidateTransaction {
            chain: "solana".into(),
            tx_hash: signature.to_owned(),
            slot,
        };

        info!(
            chain = candidate.chain,
            tx_hash = %candidate.tx_hash,
            slot = candidate.slot,
            "Detected Solana transaction involving bridge program"
        );

        self.redis
            .publish_transaction(&candidate)
            .await
            .context("failed to publish transaction to Redis")?;

        set_last_ingested_slot(&self.db, candidate.slot)
            .await
            .context("failed to update Solana ingestion checkpoint")?;

        info!(
            tx_hash = %candidate.tx_hash,
            slot = candidate.slot,
            "Published transaction to Redis and updated ingestion checkpoint"
        );

        Ok(())
    }
}
