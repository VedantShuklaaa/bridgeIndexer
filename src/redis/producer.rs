use anyhow::Result;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;

use crate::ingestion::types::CandidateTransaction;

const BRIDGE_TX_STREAM: &str = "bridge:transactions";

#[derive(Clone)]
pub struct RedisProducer {
    connection: ConnectionManager,
}

impl RedisProducer {
    pub async fn new(redis_url: &str) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;
        let connection = client.get_connection_manager().await?;

        Ok(Self { connection })
    }

    pub async fn test_connection(&self) -> Result<()> {
        let mut connection = self.connection.clone();

        let result: String = redis::cmd("PING").query_async(&mut connection).await?;

        tracing::info!(result = %result, "Redis connection successful");

        Ok(())
    }

    /// Publishes a candidate to the stream. Returns `false` when the same signature was
    /// published within `dedupe_ttl_secs` and nothing was added (`0` publishes every time).
    ///
    /// The backfill overlaps the live feed and a reconnect replays recent signatures, and
    /// every repeat costs a Helius fetch. Persistence is idempotent, so this only saves
    /// upstream calls. If the `XADD` fails the marker is removed again, so a failed publish
    /// never suppresses its own retry.
    pub async fn publish_transaction(
        &self,
        transaction: &CandidateTransaction,
        dedupe_ttl_secs: u64,
    ) -> Result<bool> {
        let mut connection = self.connection.clone();
        let slot = transaction.slot.to_string();

        let seen_key = format!("bridge:seen:{}:{}", transaction.chain, transaction.tx_hash);

        if dedupe_ttl_secs > 0 {
            let first_sighting: Option<String> = redis::cmd("SET")
                .arg(&seen_key)
                .arg(1)
                .arg("NX")
                .arg("EX")
                .arg(dedupe_ttl_secs)
                .query_async(&mut connection)
                .await?;

            if first_sighting.is_none() {
                return Ok(false);
            }
        }

        let added: redis::RedisResult<String> = connection
            .xadd(
                BRIDGE_TX_STREAM,
                "*",
                &[
                    ("chain", transaction.chain.as_str()),
                    ("tx_hash", transaction.tx_hash.as_str()),
                    ("slot", slot.as_str()),
                ],
            )
            .await;

        if let Err(error) = added {
            // Forget the signature so the retry after a reconnect is not suppressed.
            if dedupe_ttl_secs > 0 {
                let _: redis::RedisResult<i64> = connection.del(&seen_key).await;
            }
            return Err(error.into());
        }

        Ok(true)
    }
}
