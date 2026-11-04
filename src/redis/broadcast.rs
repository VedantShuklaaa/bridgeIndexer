//! How a worker's broadcast event reaches WebSocket clients.
//!
//! In `ROLE=all` the workers and the API share one process, so the event goes
//! straight into the local `tokio::sync::broadcast` channel. In the split roles the
//! workers run in the stream process and the WebSocket clients in the api process,
//! so the event crosses through Redis pub/sub: the stream role publishes and the
//! api role's forwarder feeds its local channel.

use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::StreamExt;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// The producers' view of the broadcast. Consumers call `publish` without knowing
/// whether they are in the single process or the split deployment.
#[derive(Clone)]
pub enum BroadcastSink {
    /// Same process as the WebSocket server.
    Local(broadcast::Sender<String>),
    /// The WebSocket server is in another process, behind Redis pub/sub.
    Redis {
        // Boxed so the enum stays small next to the local variant.
        connection: Box<ConnectionManager>,
        channel: String,
    },
}

impl BroadcastSink {
    pub async fn redis(redis_url: &str, channel: String) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;
        let connection = client
            .get_connection_manager()
            .await
            .context("failed to connect to Redis for broadcast pub/sub")?;
        Ok(Self::Redis {
            connection: Box::new(connection),
            channel,
        })
    }

    /// Delivers one already-serialised transaction. A publish failure is logged:
    /// the transaction is already persisted, and a missed live event must not undo
    /// that or hold the stream message unacknowledged.
    pub async fn publish(&self, payload: String) {
        match self {
            Self::Local(tx) => {
                let _ = tx.send(payload);
            }
            Self::Redis {
                connection,
                channel,
            } => {
                let mut connection = connection.clone();
                let result: redis::RedisResult<i64> = connection.publish(channel, payload).await;
                if let Err(error) = result {
                    tracing::warn!(%error, channel, "failed to publish broadcast event");
                }
            }
        }
    }
}

/// Subscribes to the pub/sub channel and forwards every message into the local
/// broadcast channel until shutdown. Reconnects with exponential backoff, so the
/// api process can start before Redis or survive a Redis restart.
pub async fn forward(
    redis_url: String,
    channel: String,
    tx: broadcast::Sender<String>,
    shutdown: CancellationToken,
) -> Result<()> {
    let mut backoff = INITIAL_BACKOFF;

    loop {
        if shutdown.is_cancelled() {
            return Ok(());
        }

        match subscribe_and_forward(&redis_url, &channel, &tx, &shutdown, &mut backoff).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                tracing::warn!(
                    %error,
                    channel,
                    retry_in_ms = backoff.as_millis(),
                    "broadcast forwarder disconnected, reconnecting"
                );
            }
        }

        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// One subscription lifetime. Returns `Ok(())` only when shutdown was requested;
/// any connection or stream failure returns `Err` so the caller reconnects.
async fn subscribe_and_forward(
    redis_url: &str,
    channel: &str,
    tx: &broadcast::Sender<String>,
    shutdown: &CancellationToken,
    backoff: &mut Duration,
) -> Result<()> {
    let client = redis::Client::open(redis_url)?;
    let mut pubsub = client
        .get_async_pubsub()
        .await
        .context("failed to open Redis pub/sub connection")?;
    pubsub
        .subscribe(channel)
        .await
        .with_context(|| format!("failed to subscribe to {channel}"))?;
    tracing::info!(channel, "broadcast forwarder subscribed");
    // A working subscription resets the next failure's wait.
    *backoff = INITIAL_BACKOFF;

    let mut messages = pubsub.on_message();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            message = messages.next() => {
                let Some(message) = message else {
                    anyhow::bail!("Redis pub/sub stream ended");
                };
                let payload: String = message.get_payload()?;
                // No WebSocket client is a normal state: the send has no receivers.
                let _ = tx.send(payload);
            }
        }
    }
}
