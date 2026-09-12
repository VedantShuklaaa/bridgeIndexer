use crate::db::repository::persist_transaction;
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
const FAILED_TX_STREAM: &str = "bridge:transactions:failed";

pub struct RedisConsumer {
    client: redis::Client,
    state: AppState,
    consumer_name: String,
}

impl RedisConsumer {
    pub fn new(redis_url: &str, state: AppState, consumer_name: String) -> Result<Self> {
        let client = redis::Client::open(redis_url)?;

        Ok(Self {
            client,
            state,
            consumer_name,
        })
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
            consumer = %self.consumer_name,
            "Redis consumer started"
        );

        loop {
            let response: Value = redis::cmd("XREADGROUP")
                .arg("GROUP")
                .arg(CONSUMER_GROUP)
                .arg(&self.consumer_name)
                .arg("COUNT")
                .arg(10)
                .arg("BLOCK")
                .arg(5000)
                .arg("STREAMS")
                .arg(BRIDGE_TX_STREAM)
                .arg(">")
                .query_async(&mut connection)
                .await?;

            self.process_messages(&mut connection, response).await?;
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

                Err(AppError::TransactionNotFound(_)) | Err(AppError::VaaNotAvailable(_))
                    if attempt < MAX_ATTEMPTS =>
                {
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

    async fn reclaim_pending_messages(
        &self,
        connection: &mut redis::aio::MultiplexedConnection,
    ) -> Result<()> {
        let response: Value = redis::cmd("XAUTOCLAIM")
            .arg(BRIDGE_TX_STREAM)
            .arg(CONSUMER_GROUP)
            .arg(&self.consumer_name)
            .arg(5 * 60 * 1000)
            .arg("0-0")
            .arg("COUNT")
            .arg(10)
            .query_async(&mut *connection)
            .await?;

        let Value::Array(mut parts) = response else {
            return Ok(());
        };

        if parts.len() < 2 {
            return Ok(());
        }

        let messages = parts.remove(1);

        let Value::Array(messages) = messages else {
            return Ok(());
        };

        if messages.is_empty() {
            return Ok(());
        }

        info!(count = messages.len(), "Reclaimed pending Redis messages");

        let response = Value::Array(vec![Value::Array(vec![
            Value::BulkString(BRIDGE_TX_STREAM.as_bytes().to_vec()),
            Value::Array(messages),
        ])]);

        self.process_messages(connection, response).await?;

        Ok(())
    }

    pub async fn run_recovery(&self) -> Result<()> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .context("failed to connect to Redis recovery worker")?;

        info!(
            stream = BRIDGE_TX_STREAM,
            group = CONSUMER_GROUP,
            consumer = %self.consumer_name,
            "Redis recovery worker started"
        );

        loop {
            self.reclaim_pending_messages(&mut connection).await?;

            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    async fn process_messages(
        &self,
        connection: &mut redis::aio::MultiplexedConnection,
        response: Value,
    ) -> Result<()> {
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
                    Ok(analysed) => {
                        if let Err(error) = persist_transaction(&self.state.db, &analysed).await {
                            tracing::error!(
                                worker = %self.consumer_name,
                                message_id = %message_id,
                                tx_hash = %candidate.tx_hash,
                                ?error,
                                "Failed to persist transaction"
                            );

                            continue;
                        }

                        tracing::info!(
                            worker = %self.consumer_name,
                            message_id = %message_id,
                            tx_hash = %candidate.tx_hash,
                            "Transaction persisted successfully"
                        );

                        let _: i64 = redis::cmd("XACK")
                            .arg(BRIDGE_TX_STREAM)
                            .arg(CONSUMER_GROUP)
                            .arg(&message_id)
                            .query_async(&mut *connection)
                            .await?;

                        tracing::info!(
                            worker = %self.consumer_name,
                            message_id = %message_id,
                            tx_hash = %candidate.tx_hash,
                            "Redis message acknowledged"
                        );
                    }

                    Err(error) => {
                        tracing::error!(
                            worker = %self.consumer_name,
                            message_id = %message_id,
                            tx_hash = %candidate.tx_hash,
                            ?error,
                            "Failed to analyse transaction"
                        );

                        match error {
                            AppError::InvalidTransactionHash(_)
                            | AppError::Normalisation(_)
                            | AppError::BadRequest(_) => {
                                let _: String = redis::cmd("XADD")
                                    .arg(FAILED_TX_STREAM)
                                    .arg("*")
                                    .arg("message_id")
                                    .arg(&message_id)
                                    .arg("chain")
                                    .arg(&candidate.chain)
                                    .arg("tx_hash")
                                    .arg(&candidate.tx_hash)
                                    .arg("slot")
                                    .arg(candidate.slot)
                                    .arg("error")
                                    .arg(error.to_string())
                                    .query_async(&mut *connection)
                                    .await?;

                                let _: i64 = redis::cmd("XACK")
                                    .arg(BRIDGE_TX_STREAM)
                                    .arg(CONSUMER_GROUP)
                                    .arg(&message_id)
                                    .query_async(&mut *connection)
                                    .await?;

                                tracing::error!(
                                    worker = %self.consumer_name,
                                    message_id = %message_id,
                                    tx_hash = %candidate.tx_hash,
                                    "Moved failed transaction to dead-letter stream"
                                );
                            }

                            _ => {
                                continue;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
