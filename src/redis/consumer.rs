use crate::error::AppError;
use crate::state::AppState;
use crate::{domain::transaction::NormalisedTransaction, services::analyzer::analyse_tx};
use anyhow::{Context, Result};
use redis::Value;
use std::time::Duration;
use tracing::info;

use crate::ingestion::solana::CandidateTransaction;

const BRIDGE_TX_STREAM: &str = "bridge:transactions";
const CONSUMER_GROUP: &str = "bridge-workers";
const CONSUMER_NAME: &str = "worker-1";

pub struct RedisConsumer {
    client: redis::Client,
    state: AppState,
}

impl RedisConsumer {
    pub fn new(redis_url: &str, state: AppState) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;

        Ok(Self { client, state })
    }

    pub async fn run(&self) -> Result<()> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .context("failed to connect to Redis")?;

        self.create_consumer_group(&mut connection).await?;

        info!(
            stream = BRIDGE_TX_STREAM,
            group = CONSUMER_GROUP,
            consumer = CONSUMER_NAME,
            "Redis consumer started"
        );

        loop {
            let response: Value = redis::cmd("XREADGROUP")
                .arg("GROUP")
                .arg(CONSUMER_GROUP)
                .arg(CONSUMER_NAME)
                .arg("COUNT")
                .arg(10)
                .arg("BLOCK")
                .arg(5000)
                .arg("STREAMS")
                .arg(BRIDGE_TX_STREAM)
                .arg(">")
                .query_async(&mut connection)
                .await?;

            self.process_messages(response).await?;
        }
    }

    async fn create_consumer_group(
        &self,
        connection: &mut redis::aio::MultiplexedConnection,
    ) -> Result<()> {
        let result: redis::RedisResult<String> = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(BRIDGE_TX_STREAM)
            .arg(CONSUMER_GROUP)
            .arg("0")
            .query_async(connection)
            .await;

        match result {
            Ok(_) => {
                info!(
                    stream = BRIDGE_TX_STREAM,
                    group = CONSUMER_GROUP,
                    "Created Redis consumer group"
                );
            }

            Err(error) if error.to_string().contains("BUSYGROUP") => {
                info!(
                    group = CONSUMER_GROUP,
                    "Redis consumer group already exists"
                );
            }

            Err(error) => {
                return Err(error.into());
            }
        }

        Ok(())
    }

    async fn analyse_with_retry(&self, tx_hash: &str) -> Result<NormalisedTransaction, AppError> {
        const MAX_ATTEMPTS: u32 = 5;

        for attempt in 1..=MAX_ATTEMPTS {
            match analyse_tx(&self.state, tx_hash).await {
                Ok(tx) => return Ok(tx),

                Err(AppError::TransactionNotFound(_)) if attempt < MAX_ATTEMPTS => {
                    let delay_ms = 500 * 2_u64.pow(attempt - 1);

                    tracing::warn!(
                        tx_hash = %tx_hash,
                        attempt,
                        delay_ms,
                        "Transaction not available yet, retrying"
                    );

                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }

                Err(error) => return Err(error),
            }
        }

        unreachable!()
    }

    async fn process_messages(&self, response: Value) -> Result<()> {
        let Value::Array(streams) = response else {
            return Ok(());
        };

        for stream in streams {
            let Value::Array(mut stream_parts) = stream else {
                continue;
            };

            if stream_parts.len() != 2 {
                continue;
            }

            let messages = stream_parts.pop().unwrap();

            let Value::Array(messages) = messages else {
                continue;
            };

            for message in messages {
                let Value::Array(mut message_parts) = message else {
                    continue;
                };

                if message_parts.len() != 2 {
                    continue;
                }

                let fields = message_parts.pop().unwrap();
                let message_id = message_parts.pop().unwrap();

                let message_id = match message_id {
                    Value::BulkString(bytes) => String::from_utf8(bytes)?,
                    _ => continue,
                };

                let Value::Array(fields) = fields else {
                    continue;
                };

                let mut chain = None;
                let mut tx_hash = None;
                let mut slot = None;

                for pair in fields.chunks(2) {
                    if pair.len() != 2 {
                        continue;
                    }

                    let Value::BulkString(key) = &pair[0] else {
                        continue;
                    };

                    let Value::BulkString(value) = &pair[1] else {
                        continue;
                    };

                    let key = String::from_utf8(key.clone())?;
                    let value = String::from_utf8(value.clone())?;

                    match key.as_str() {
                        "chain" => chain = Some(value),
                        "tx_hash" => tx_hash = Some(value),
                        "slot" => slot = Some(value.parse::<u64>()?),
                        _ => {}
                    }
                }

                let candidate = CandidateTransaction {
                    chain: chain.unwrap_or_else(|| "unknown".to_string()),
                    tx_hash: tx_hash.context("Redis message missing tx_hash")?,
                    slot: slot.context("Redis message missing slot")?,
                };

                info!(
                    message_id = %message_id,
                    chain = %candidate.chain,
                    tx_hash = %candidate.tx_hash,
                    slot = candidate.slot,
                    "Parsed transaction from Redis"
                );

                match self.analyse_with_retry(&candidate.tx_hash).await {
                    Ok(_analysed) => {
                        info!(
                            message_id = %message_id,
                            tx_hash = %candidate.tx_hash,
                            "Transaction analysed successfully"
                        );
                    }

                    Err(error) => {
                        tracing::error!(
                            message_id = %message_id,
                            tx_hash = %candidate.tx_hash,
                            ?error,
                            "Failed to analyse transaction"
                        );

                        continue;
                    }
                }
            }
        }

        Ok(())
    }
}
