use anyhow::{Context, Result};
use futures_util::{SinkExt, Stream, StreamExt};
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{self, Message},
};
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

/// How long the server has to acknowledge `logsSubscribe`. Data messages are not expected
/// before the acknowledgement, so silence past this means the connection is dead.
const SUBSCRIBE_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// The reply to our `logsSubscribe` request, as far as it concerns us.
#[derive(Debug, PartialEq, Eq)]
enum SubscribeReply {
    /// The server accepted the subscription. The payload is the subscription id, kept
    /// optional because only the absence of an error is contractually guaranteed.
    Confirmed { subscription: Option<u64> },
    /// The server rejected the subscription; the string is the JSON-RPC error payload.
    Rejected(String),
    /// Not a reply to this request (a notification, or a response to another id).
    Unrelated,
}

fn parse_subscribe_reply(text: &str, expected_id: u64) -> Result<SubscribeReply> {
    let message: Value = serde_json::from_str(text).context("invalid Solana WebSocket JSON")?;

    if message.get("id").and_then(Value::as_u64) != Some(expected_id) {
        return Ok(SubscribeReply::Unrelated);
    }

    if let Some(error) = message.get("error") {
        return Ok(SubscribeReply::Rejected(error.to_string()));
    }

    match message.get("result") {
        Some(result) if !result.is_null() => Ok(SubscribeReply::Confirmed {
            subscription: result.as_u64(),
        }),
        _ => Ok(SubscribeReply::Rejected(
            "logsSubscribe reply has no result".to_owned(),
        )),
    }
}

/// Waits for the server to accept (or reject) `logsSubscribe` before any message is handled.
///
/// Frames that are not the reply are ignored: a subscription is not active until it is
/// acknowledged, so there is nothing to process yet. A rejection, a closed socket and a
/// timeout are all connection failures, which the caller turns into a reconnect.
async fn wait_for_subscribe_reply<S>(
    frames: &mut S,
    expected_id: u64,
    timeout: Duration,
) -> Result<Option<u64>>
where
    S: Stream<Item = std::result::Result<Message, tungstenite::Error>> + Unpin,
{
    let reply = tokio::time::timeout(timeout, async {
        loop {
            let frame = match frames.next().await {
                Some(frame) => frame.context("failed to read logsSubscribe reply")?,
                None => anyhow::bail!("Solana WebSocket ended before acknowledging logsSubscribe"),
            };

            match frame {
                Message::Text(text) => match parse_subscribe_reply(&text, expected_id)? {
                    SubscribeReply::Confirmed { subscription } => {
                        return Ok::<Option<u64>, anyhow::Error>(subscription);
                    }
                    SubscribeReply::Rejected(reason) => {
                        anyhow::bail!("logsSubscribe rejected: {reason}")
                    }
                    SubscribeReply::Unrelated => {}
                },
                Message::Close(frame) => anyhow::bail!(
                    "Solana WebSocket closed before acknowledging logsSubscribe: {frame:?}"
                ),
                _ => {}
            }
        }
    })
    .await;

    match reply {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!(
            "timed out after {timeout:?} waiting for logsSubscribe reply"
        )),
    }
}

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

        let subscription =
            match wait_for_subscribe_reply(&mut read, request.id, SUBSCRIBE_REPLY_TIMEOUT).await {
                Ok(subscription) => subscription,
                Err(error) => {
                    error!(
                        %error,
                        program = %self.bridge_program,
                        "logsSubscribe failed, closing the connection"
                    );
                    return Err(error);
                }
            };

        info!(
            program = %self.bridge_program,
            subscription = ?subscription,
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
        // The `logsSubscribe` reply is consumed before this loop starts, so anything that is
        // not a notification here (keepalive frames, stray replies) is ignored.
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    const ID: u64 = 1;

    fn frame(text: &str) -> Message {
        Message::Text(text.to_owned())
    }

    #[test]
    fn accepts_a_confirmed_subscription() {
        let reply = parse_subscribe_reply(r#"{"jsonrpc":"2.0","result":23784,"id":1}"#, ID);

        assert_eq!(
            reply.unwrap(),
            SubscribeReply::Confirmed {
                subscription: Some(23784)
            }
        );
    }

    #[test]
    fn accepts_a_confirmation_without_a_numeric_subscription_id() {
        let reply = parse_subscribe_reply(r#"{"jsonrpc":"2.0","result":true,"id":1}"#, ID);

        assert_eq!(
            reply.unwrap(),
            SubscribeReply::Confirmed { subscription: None }
        );
    }

    #[test]
    fn surfaces_a_json_rpc_error_object() {
        let reply = parse_subscribe_reply(
            r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid params"},"id":1}"#,
            ID,
        );

        match reply.unwrap() {
            SubscribeReply::Rejected(reason) => assert!(reason.contains("Invalid params")),
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_reply_without_a_result() {
        let reply = parse_subscribe_reply(r#"{"jsonrpc":"2.0","id":1}"#, ID);

        match reply.unwrap() {
            SubscribeReply::Rejected(reason) => assert!(reason.contains("no result")),
            other => panic!("expected rejection, got {other:?}"),
        }
    }

    #[test]
    fn ignores_a_reply_with_a_different_id() {
        let reply = parse_subscribe_reply(r#"{"jsonrpc":"2.0","result":23784,"id":2}"#, ID);

        assert_eq!(reply.unwrap(), SubscribeReply::Unrelated);
    }

    #[test]
    fn ignores_a_reply_with_a_non_numeric_id() {
        let reply = parse_subscribe_reply(r#"{"jsonrpc":"2.0","result":23784,"id":"1"}"#, ID);

        assert_eq!(reply.unwrap(), SubscribeReply::Unrelated);
    }

    #[test]
    fn ignores_a_reply_with_a_missing_id() {
        let reply = parse_subscribe_reply(r#"{"jsonrpc":"2.0","result":23784}"#, ID);

        assert_eq!(reply.unwrap(), SubscribeReply::Unrelated);
    }

    #[test]
    fn ignores_a_logs_notification() {
        let reply = parse_subscribe_reply(
            r#"{"jsonrpc":"2.0","method":"logsNotification","params":{"result":{"context":{"slot":1},"value":{"signature":"sig","err":null}}}}"#,
            ID,
        );

        assert_eq!(reply.unwrap(), SubscribeReply::Unrelated);
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(parse_subscribe_reply("not json", ID).is_err());
    }

    #[tokio::test]
    async fn waits_until_a_matching_confirmation_arrives() {
        let mut frames = stream::iter(vec![
            Ok::<_, tungstenite::Error>(frame(r#"{"jsonrpc":"2.0","result":1,"id":2}"#)),
            Ok::<_, tungstenite::Error>(frame(r#"{"jsonrpc":"2.0","result":23784,"id":1}"#)),
        ]);

        let subscription = wait_for_subscribe_reply(&mut frames, ID, Duration::from_secs(1)).await;

        assert_eq!(subscription.unwrap(), Some(23784));
    }

    #[tokio::test]
    async fn a_rejection_fails_the_wait() {
        let mut frames = stream::iter(vec![Ok::<_, tungstenite::Error>(frame(
            r#"{"jsonrpc":"2.0","error":{"code":-32602,"message":"Invalid params"},"id":1}"#,
        ))]);

        let error = wait_for_subscribe_reply(&mut frames, ID, Duration::from_secs(1))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("logsSubscribe rejected"));
    }

    #[tokio::test]
    async fn a_missing_reply_times_out() {
        let mut frames = stream::pending();

        let error = wait_for_subscribe_reply(&mut frames, ID, Duration::from_millis(20))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("timed out"));
    }
}
