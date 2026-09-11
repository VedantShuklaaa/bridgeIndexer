use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::{
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
}

impl SolanaIngester {
    pub fn new(ws_url: String, bridge_program: String, redis: RedisProducer) -> Self {
        Self {
            ws_url,
            bridge_program,
            redis,
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

        info!(
            tx_hash = %candidate.tx_hash,
            slot = candidate.slot,
            "Published transaction to Redis"
        );

        Ok(())
    }
}
