use anyhow::Result;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;

use crate::ingestion::solana::CandidateTransaction;

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

    pub async fn publish_transaction(&self, transaction: &CandidateTransaction) -> Result<()> {
        let mut connection = self.connection.clone();
        let slot = transaction.slot.to_string();

        let _: String = connection
            .xadd(
                BRIDGE_TX_STREAM,
                "*",
                &[
                    ("chain", transaction.chain.as_str()),
                    ("tx_hash", transaction.tx_hash.as_str()),
                    ("slot", slot.as_str()),
                ],
            )
            .await?;

        Ok(())
    }
}
