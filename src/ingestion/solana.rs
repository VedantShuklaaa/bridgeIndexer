use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::{
    clients::helius::get_signatures_for_address,
    config::IngestConfig,
    db::repository::{get_last_ingested_slot, set_last_ingested_slot},
    ingestion::{
        backfill::{plan_page, should_backfill},
        types::{CandidateTransaction, LogsConfig, LogsFilter, SolanaRpcRequest},
    },
    redis::producer::RedisProducer,
};

pub struct SolanaIngester {
    ws_url: String,
    bridge_program: String,
    redis: RedisProducer,
    db: PgPool,
    client: reqwest::Client,
    helius_url: String,
    config: IngestConfig,
}

impl SolanaIngester {
    pub fn new(
        ws_url: String,
        bridge_program: String,
        redis: RedisProducer,
        db: PgPool,
        client: reqwest::Client,
        helius_url: String,
        config: IngestConfig,
    ) -> Self {
        Self {
            ws_url,
            bridge_program,
            redis,
            db,
            client,
            helius_url,
            config,
        }
    }

    /// Runs (and reconnects) until `stop` is cancelled.
    pub async fn run(&self, stop: CancellationToken) -> Result<()> {
        loop {
            match self.run_connection(&stop).await {
                Ok(()) => warn!("Solana WebSocket connection closed"),
                Err(error) => error!(%error, "Solana ingestion connection failed"),
            }

            tokio::select! {
                _ = stop.cancelled() => return Ok(()),
                _ = sleep(Duration::from_secs(5)) => {}
            }
        }
    }

    async fn run_connection(&self, stop: &CancellationToken) -> Result<()> {
        let ws_host = self.ws_url.split('?').next().unwrap_or_default();
        info!(ws_host, bridge_program = %self.bridge_program, "Connecting to Solana WebSocket");

        let (ws_stream, _) = tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            result = connect_async(&self.ws_url) => {
                result.context("failed to connect to Solana WebSocket")?
            }
        };

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
            .send(Message::Text(serde_json::to_string(&request)?))
            .await?;

        info!(
            program = %self.bridge_program,
            "Subscribed to Solana bridge logs"
        );

        if self.config.backfill
            && let Err(error) = self.backfill_missed_transactions(stop).await
        {
            warn!(%error, "Solana transaction backfill failed, continuing with live ingestion");
        }

        loop {
            tokio::select! {
                _ = stop.cancelled() => {
                    info!("stop requested, closing Solana WebSocket");
                    let _ = write.send(Message::Close(None)).await;
                    return Ok(());
                }
                message = read.next() => {
                    let Some(message) = message else { break; };
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
            }
        }

        Ok(())
    }

    async fn backfill_missed_transactions(&self, stop: &CancellationToken) -> Result<()> {
        let last_ingested_slot = get_last_ingested_slot(&self.db, "solana").await?;

        if !should_backfill(last_ingested_slot) {
            info!("No ingestion checkpoint yet, skipping backfill");
            return Ok(());
        }

        info!(
            last_ingested_slot,
            max = self.config.backfill_max,
            "Starting Solana transaction backfill"
        );

        let pace = Duration::from_millis(self.config.backfill_pace_ms);
        let mut before: Option<String> = None;
        let mut published = 0usize;
        let mut skipped = 0usize;

        loop {
            if stop.is_cancelled() {
                return Ok(());
            }

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

            let remaining = self.config.backfill_max.saturating_sub(published);
            let page = plan_page(&signatures, last_ingested_slot, remaining);

            for candidate in &page.candidates {
                if stop.is_cancelled() {
                    return Ok(());
                }

                let fresh = self
                    .redis
                    .publish_transaction(candidate, self.config.dedupe_ttl_secs)
                    .await
                    .context("failed to publish backfilled transaction")?;

                if !fresh {
                    skipped += 1;
                    continue;
                }

                published += 1;

                info!(
                    tx_hash = %candidate.tx_hash,
                    slot = candidate.slot,
                    "Published backfilled transaction"
                );

                tokio::time::sleep(pace).await;
            }

            if page.done {
                if published >= self.config.backfill_max {
                    warn!(
                        max = self.config.backfill_max,
                        "Backfill stopped at INGEST_BACKFILL_MAX, older signatures were not replayed"
                    );
                }
                break;
            }

            before = signatures
                .last()
                .and_then(|value| value.get("signature").and_then(Value::as_str))
                .map(str::to_string);
        }

        info!(
            published,
            skipped, last_ingested_slot, "Solana transaction backfill completed"
        );

        Ok(())
    }

    async fn handle_message(&self, text: &str) -> Result<()> {
        let message: Value = serde_json::from_str(text).context("invalid Solana WebSocket JSON")?;
        if message.get("method").and_then(Value::as_str) != Some("logsNotification") {
            return Ok(());
        }

        if message.get("id").and_then(Value::as_u64) == Some(1) {
            if let Some(err) = message.get("error") {
                anyhow::bail!("logsSubscribe rejected: {err}");
            }

            info!(
                subscription = ?message.get("result"),
                "logsSubscribe confirmed"
            );

            return Ok(());
        }

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

        let fresh = self
            .redis
            .publish_transaction(&candidate, self.config.dedupe_ttl_secs)
            .await
            .context("failed to publish transaction to Redis")?;

        if !fresh {
            info!(tx_hash = %candidate.tx_hash, "Signature already published recently, skipping");
        }

        set_last_ingested_slot(&self.db, "solana", candidate.slot)
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
