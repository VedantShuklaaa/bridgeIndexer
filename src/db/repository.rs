use crate::chains::{NS_WORMHOLE, RawChainRef};
use crate::domain::analysis::DecodedTransfer;
use crate::domain::bridge_transfer::BridgeTransfer;
use crate::domain::transaction::NormalisedTransaction;
use crate::protocols::{MessageId, amount::RawAmount};
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Row, Transaction};

pub async fn persist_transaction(pool: &PgPool, tx: &NormalisedTransaction) -> anyhow::Result<()> {
    persist_analysis(pool, tx, &[]).await
}

/// Writes the transaction and every decoded transfer. `transfers` is empty on the
/// WormholeScan-driven path and for re-enrichment, where the v1 body already holds the
/// one transfer to persist.
pub async fn persist_analysis(
    pool: &PgPool,
    tx: &NormalisedTransaction,
    transfers: &[DecodedTransfer],
) -> anyhow::Result<()> {
    let mut db_tx: Transaction<'_, Postgres> = pool.begin().await?;
    let transaction_id = upsert_transaction(&mut db_tx, tx).await?;

    if transfers.is_empty() {
        if let Some(bridge) = &tx.bridge_transfer {
            let analysis = serde_json::to_value(tx)?;
            write_transfer(&mut db_tx, transaction_id, legacy_row(tx, bridge, analysis)).await?;
        }
    } else {
        for (position, decoded) in transfers.iter().enumerate() {
            // The cached v1 analysis follows the row it belongs to. Only the first
            // transfer carries the WormholeScan event `/api/v1` exposes.
            let mut row_tx = tx.clone();
            row_tx.bridge_transfer = Some(decoded.bridge.clone());
            row_tx.bridge_event = if position == 0 {
                tx.bridge_event.clone()
            } else {
                None
            };
            let analysis = serde_json::to_value(&row_tx)?;

            let event_index = i16::try_from(decoded.detected.event_index).map_err(|_| {
                anyhow::anyhow!("event index too large: {}", decoded.detected.event_index)
            })?;
            write_transfer(
                &mut db_tx,
                transaction_id,
                TransferRow {
                    protocol: decoded.detected.message.protocol().as_str().to_string(),
                    direction: decoded.detected.direction.as_str().to_string(),
                    message: decoded.detected.message.clone(),
                    observed_tx_hash: decoded.detected.observed_tx.clone(),
                    event_index,
                    via: decoded.detected.via.as_ref().map(|via| via.name.clone()),
                    via_ref: decoded
                        .detected
                        .via
                        .as_ref()
                        .and_then(|via| via.reference.clone()),
                    source: decoded.detected.source.clone(),
                    destination: decoded.detected.destination.clone(),
                    token_ref: decoded
                        .detected
                        .token
                        .as_ref()
                        .map(|token| token.chain.clone()),
                    bridge: &decoded.bridge,
                    analysis,
                },
            )
            .await?;
        }
    }

    db_tx.commit().await?;
    Ok(())
}

async fn upsert_transaction(
    db_tx: &mut Transaction<'_, Postgres>,
    tx: &NormalisedTransaction,
) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        r#"
        INSERT INTO transactions (
            hash,
            chain,
            status,
            slot,
            timestamp,
            fee_lamports,
            signer
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (hash)
        DO UPDATE SET
            status = EXCLUDED.status
        RETURNING id
        "#,
    )
    .bind(&tx.hash)
    .bind(tx.chain.wormhole_id() as i16)
    .bind(format!("{:?}", tx.status))
    .bind(tx.slot as i64)
    .bind(
        tx.timestamp
            .map(|ts| chrono::DateTime::from_timestamp(ts, 0)),
    )
    .bind(tx.fee_lamports as i64)
    .bind(&tx.signer)
    .fetch_one(&mut **db_tx)
    .await?)
}

/// One `bridge_transfers` row to write, generalised across protocols.
struct TransferRow<'a> {
    protocol: String,
    direction: String,
    message: MessageId,
    observed_tx_hash: String,
    event_index: i16,
    via: Option<String>,
    via_ref: Option<String>,
    source: RawChainRef,
    destination: RawChainRef,
    token_ref: Option<RawChainRef>,
    bridge: &'a BridgeTransfer,
    analysis: serde_json::Value,
}

/// The single transfer the v1 body carries, for the WormholeScan-driven path and
/// re-enrichment. Raw chain references are the protocol's own (`wormhole`), and the
/// token chain is unknown to the v1 shape.
fn legacy_row<'a>(
    tx: &NormalisedTransaction,
    bridge: &'a BridgeTransfer,
    analysis: serde_json::Value,
) -> TransferRow<'a> {
    TransferRow {
        protocol: NS_WORMHOLE.to_string(),
        direction: bridge.direction().as_str().to_string(),
        message: MessageId::Wormhole {
            emitter_chain: bridge.message_id.emitter_chain,
            emitter_address: bridge.message_id.emitter_address.clone(),
            sequence: bridge.message_id.sequence,
        },
        observed_tx_hash: tx.hash.clone(),
        event_index: 0,
        via: None,
        via_ref: None,
        source: RawChainRef::wormhole(bridge.source_chain.wormhole_id()),
        destination: RawChainRef::wormhole(bridge.destination_chain.wormhole_id()),
        token_ref: None,
        bridge,
        analysis,
    }
}

async fn write_transfer(
    db_tx: &mut Transaction<'_, Postgres>,
    transaction_id: i64,
    row: TransferRow<'_>,
) -> anyhow::Result<()> {
    let bridge = row.bridge;
    // An amount that is not a plain integer is stored as NULL. The analysis JSON keeps it.
    let amount = bridge
        .amount
        .as_deref()
        .and_then(RawAmount::parse)
        .map(|a| a.to_string());

    let transfer_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO bridge_transfers (
            transaction_id, protocol, message_key, direction, observed_tx_hash,
            event_index, source_tx_hash,
            source_ref_protocol, source_ref_id, source_chain_id,
            source_wallet, source_explorer_url,
            destination_ref_protocol, destination_ref_id, destination_chain_id,
            destination_wallet, destination_tx_hash, destination_explorer_url,
            token_ref_protocol, token_ref_id, token_chain_id,
            token, token_symbol, amount, amount_formatted, status, analysis,
            via, via_ref
        )
        VALUES (
            $1, $2, $3, $4, $5,
            $6, $7,
            $8, $9, resolve_chain($8, $9),
            $10, $11,
            $12, $13, resolve_chain($12, $13),
            $14, $15, $16,
            $17, $18, resolve_chain($17, $18),
            $19, $20, $21::numeric, $22, $23, $24,
            $25, $26
        )
        ON CONFLICT (protocol, message_key)
        DO UPDATE SET
            -- an inbound redeem learns its origin tx later, from enrichment
            source_tx_hash = COALESCE(EXCLUDED.source_tx_hash, bridge_transfers.source_tx_hash),
            source_explorer_url = COALESCE(EXCLUDED.source_explorer_url, bridge_transfers.source_explorer_url),
            destination_tx_hash = COALESCE(EXCLUDED.destination_tx_hash, bridge_transfers.destination_tx_hash),
            destination_wallet = COALESCE(EXCLUDED.destination_wallet, bridge_transfers.destination_wallet),
            destination_explorer_url = COALESCE(EXCLUDED.destination_explorer_url, bridge_transfers.destination_explorer_url),
            token_symbol = COALESCE(EXCLUDED.token_symbol, bridge_transfers.token_symbol),
            amount_formatted = COALESCE(EXCLUDED.amount_formatted, bridge_transfers.amount_formatted),
            -- composition can fill the router and token chain on a later pass
            via = COALESCE(EXCLUDED.via, bridge_transfers.via),
            via_ref = COALESCE(EXCLUDED.via_ref, bridge_transfers.via_ref),
            token_ref_protocol = COALESCE(EXCLUDED.token_ref_protocol, bridge_transfers.token_ref_protocol),
            token_ref_id = COALESCE(EXCLUDED.token_ref_id, bridge_transfers.token_ref_id),
            token_chain_id = COALESCE(EXCLUDED.token_chain_id, bridge_transfers.token_chain_id),
            status = CASE WHEN bridge_transfers.status = 'Completed'
                            OR (bridge_transfers.status = 'Pending' AND EXCLUDED.status = 'Detected')
                          THEN bridge_transfers.status ELSE EXCLUDED.status END,
            -- only replace the cached response if it describes the same observed tx
            analysis = CASE WHEN bridge_transfers.observed_tx_hash = EXCLUDED.observed_tx_hash
                             AND NOT (bridge_transfers.status = 'Pending' AND EXCLUDED.status = 'Detected')
                            THEN EXCLUDED.analysis ELSE bridge_transfers.analysis END,
            updated_at = NOW()
        RETURNING id
        "#,
    )
    .bind(transaction_id)
    .bind(&row.protocol)
    .bind(row.message.key())
    .bind(&row.direction)
    .bind(&row.observed_tx_hash)
    .bind(row.event_index)
    .bind(&bridge.source_tx_hash)
    .bind(&row.source.protocol)
    .bind(&row.source.id)
    .bind(&bridge.source_wallet)
    .bind(&bridge.source_explorer_url)
    .bind(&row.destination.protocol)
    .bind(&row.destination.id)
    .bind(&bridge.destination_wallet)
    .bind(&bridge.destination_tx_hash)
    .bind(&bridge.destination_explorer_url)
    .bind(row.token_ref.as_ref().map(|r| r.protocol.as_str()))
    .bind(row.token_ref.as_ref().map(|r| r.id.as_str()))
    .bind(&bridge.token)
    .bind(&bridge.token_symbol)
    .bind(amount)
    .bind(&bridge.amount_formatted)
    .bind(format!("{:?}", bridge.status))
    .bind(row.analysis)
    .bind(row.via)
    .bind(row.via_ref)
    .fetch_one(&mut **db_tx)
    .await?;

    // One message identity per row today; other protocols will add their own detail.
    let MessageId::Wormhole {
        emitter_chain,
        emitter_address,
        sequence,
    } = &row.message;

    sqlx::query(
        r#"
        INSERT INTO wormhole_messages (transfer_id, emitter_chain, emitter_address, sequence)
        VALUES ($1, $2, $3, $4::numeric)
        ON CONFLICT (transfer_id) DO NOTHING
        "#,
    )
    .bind(transfer_id)
    .bind(i32::from(*emitter_chain))
    .bind(emitter_address)
    .bind(sequence.to_string())
    .execute(&mut **db_tx)
    .await?;

    Ok(())
}

pub async fn get_last_ingested_slot(pool: &PgPool, chain: &str) -> Result<u64> {
    let key = format!("{chain}_last_ingested_slot");
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM indexer_state WHERE key = $1")
            .bind(&key)
            .fetch_optional(pool)
            .await?;

    Ok(value.map(|v| v.parse()).transpose()?.unwrap_or(0))
}

/// Records the newest slot ingested for `chain`. The checkpoint only moves forward: log
/// notifications can arrive out of slot order, and rewinding would make the next backfill
/// replay (or skip past) work that was already done.
pub async fn set_last_ingested_slot(pool: &PgPool, chain: &str, slot: u64) -> Result<()> {
    let key = format!("{chain}_last_ingested_slot");
    sqlx::query(
        "INSERT INTO indexer_state (key, value) VALUES ($1, $2)
         ON CONFLICT (key) DO UPDATE
         SET value = GREATEST(indexer_state.value::bigint, EXCLUDED.value::bigint)::text,
             -- the value only moves forward, but the heartbeat tracks every handled
             -- notification, including ones at or below the checkpoint (/health).
             updated_at = NOW()",
    )
    .bind(&key)
    .bind(slot.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_cached_analysis(pool: &PgPool, hash: &str) -> Result<Option<serde_json::Value>> {
    Ok(sqlx::query_scalar(
        r#"
        SELECT analysis FROM bridge_transfers
        WHERE observed_tx_hash = $1
          AND analysis IS NOT NULL
          AND (status = 'Completed' OR updated_at > NOW() - INTERVAL '30 seconds')
        ORDER BY event_index
        LIMIT 1
        "#,
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?)
}

/// A transfer due for another enrichment attempt.
pub struct DueTransfer {
    pub observed_tx_hash: String,
    pub analysis: serde_json::Value,
}

/// Open transfers that are old enough to leave to enrichment (`min_age_secs` since they
/// were created), young enough to still be worth asking about (`max_age_secs`) and not
/// attempted in the last `retry_secs`. Least recently attempted first. Served by
/// `idx_bt_open`.
pub async fn list_open_transfers(
    pool: &PgPool,
    min_age_secs: i64,
    max_age_secs: i64,
    retry_secs: i64,
    limit: i64,
) -> Result<Vec<DueTransfer>> {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        r#"
        SELECT observed_tx_hash, analysis
        FROM bridge_transfers
        WHERE status IN ('Detected', 'Pending')
          AND analysis IS NOT NULL
          AND created_at < NOW() - ($1::double precision * INTERVAL '1 second')
          AND created_at > NOW() - ($2::double precision * INTERVAL '1 second')
          AND updated_at < NOW() - ($3::double precision * INTERVAL '1 second')
        ORDER BY updated_at ASC
        LIMIT $4
        "#,
    )
    .bind(min_age_secs)
    .bind(max_age_secs)
    .bind(retry_secs)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(observed_tx_hash, analysis)| DueTransfer {
            observed_tx_hash,
            analysis,
        })
        .collect())
}

/// Completed inbound rows whose origin tx is still unknown, due for another attempt.
/// A redeem is stored `Completed` before WormholeScan has matched its operation, so the
/// origin tx arrives later. Unlike open transfers there is no `max_age_secs`: the origin
/// tx can become known at any time, and the row is otherwise terminal. Only `min_age_secs`
/// and `retry_secs` apply. Served by `idx_bt_missing_origin`.
pub async fn list_missing_origin_transfers(
    pool: &PgPool,
    min_age_secs: i64,
    retry_secs: i64,
    limit: i64,
) -> Result<Vec<DueTransfer>> {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        r#"
        SELECT observed_tx_hash, analysis
        FROM bridge_transfers
        WHERE status = 'Completed'
          AND direction = 'Inbound'
          AND source_tx_hash IS NULL
          AND analysis IS NOT NULL
          AND created_at < NOW() - ($1::double precision * INTERVAL '1 second')
          AND updated_at < NOW() - ($2::double precision * INTERVAL '1 second')
        ORDER BY updated_at ASC
        LIMIT $3
        "#,
    )
    .bind(min_age_secs)
    .bind(retry_secs)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(observed_tx_hash, analysis)| DueTransfer {
            observed_tx_hash,
            analysis,
        })
        .collect())
}

/// One persisted transfer in a block-time window, for the read-only `reconcile`
/// tool. `block_time` is the chain's `blockTime` (`transactions.timestamp`);
/// `stored_at` is when the `bridge_transfers` row was written.
#[derive(Debug, sqlx::FromRow)]
pub struct PersistedTransferRow {
    pub observed_tx_hash: String,
    pub slot: i64,
    pub block_time: i64,
    pub stored_at: i64,
}

/// Persisted transfers of one protocol whose transaction was in
/// `[start, end]` (Unix seconds, inclusive). Read-only.
pub async fn persisted_transfers_in_window(
    pool: &PgPool,
    protocol: &str,
    start: i64,
    end: i64,
) -> Result<Vec<PersistedTransferRow>> {
    let rows: Vec<PersistedTransferRow> = sqlx::query_as(
        r#"
        SELECT bt.observed_tx_hash,
               t.slot,
               EXTRACT(EPOCH FROM t.timestamp)::bigint AS block_time,
               EXTRACT(EPOCH FROM bt.created_at)::bigint AS stored_at
        FROM bridge_transfers bt
        JOIN transactions t ON t.id = bt.transaction_id
        WHERE bt.observed_tx_hash IS NOT NULL
          AND bt.protocol = $1
          AND t.timestamp >= to_timestamp($2::double precision)
          AND t.timestamp <= to_timestamp($3::double precision)
        ORDER BY t.timestamp, bt.event_index
        "#,
    )
    .bind(protocol)
    .bind(start as f64)
    .bind(end as f64)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// A stored analysis plus whether it is still fresh.
pub struct CachedAnalysis {
    pub analysis: serde_json::Value,
    pub fresh: bool,
}

/// Returns the stored analysis regardless of age, flagged fresh/stale.
/// Completed transfers are always fresh; anything else is fresh for `fresh_secs`
/// after its last update. Used for stale-while-revalidate.
pub async fn get_analysis_any_age(
    pool: &PgPool,
    hash: &str,
    fresh_secs: f64,
) -> Result<Option<CachedAnalysis>> {
    let row: Option<(serde_json::Value, bool)> = sqlx::query_as(
        r#"
        SELECT analysis,
               COALESCE(
                   status = 'Completed'
                   OR updated_at > NOW() - ($2::double precision * INTERVAL '1 second'),
                   FALSE
               ) AS fresh
        FROM bridge_transfers
        WHERE observed_tx_hash = $1
          AND analysis IS NOT NULL
        ORDER BY event_index
        LIMIT 1
        "#,
    )
    .bind(hash)
    .bind(fresh_secs)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(analysis, fresh)| CachedAnalysis { analysis, fresh }))
}

/// The transaction row for the generalised read path.
pub struct StoredTransaction {
    pub hash: String,
    /// Wormhole id of the chain the transaction was observed on.
    pub chain: i16,
    pub status: String,
    pub slot: i64,
    pub timestamp: Option<DateTime<Utc>>,
    pub fee_lamports: i64,
    pub signer: Option<String>,
}

/// One `bridge_transfers` row, with its protocol detail. A row can exist for a protocol
/// whose detail table is not `wormhole_messages`; the message columns are then `None`.
pub struct StoredTransfer {
    pub protocol: String,
    pub direction: String,
    pub observed_tx_hash: String,
    pub event_index: i16,
    pub via: Option<String>,
    pub via_ref: Option<String>,
    pub source_ref_protocol: String,
    pub source_ref_id: String,
    pub source_tx_hash: Option<String>,
    pub source_wallet: Option<String>,
    pub source_explorer_url: Option<String>,
    pub destination_ref_protocol: String,
    pub destination_ref_id: String,
    pub destination_wallet: Option<String>,
    pub destination_tx_hash: Option<String>,
    pub destination_explorer_url: Option<String>,
    pub token_ref_protocol: Option<String>,
    pub token_ref_id: Option<String>,
    pub token: Option<String>,
    pub token_symbol: Option<String>,
    pub amount: Option<String>,
    pub amount_formatted: Option<String>,
    pub status: String,
    pub emitter_chain: Option<i32>,
    pub emitter_address: Option<String>,
    pub sequence: Option<String>,
}

pub struct StoredTransactionWithTransfers {
    pub transaction: StoredTransaction,
    pub transfers: Vec<StoredTransfer>,
}

/// The transaction and every transfer decoded from it, ordered by `event_index`.
/// A transaction with no transfers comes back with an empty `transfers` list.
pub async fn get_transaction_with_transfers(
    pool: &PgPool,
    hash: &str,
) -> Result<Option<StoredTransactionWithTransfers>> {
    let rows = sqlx::query(
        r#"
        SELECT t.hash, t.chain, t.status AS tx_status, t.slot, t.timestamp,
               t.fee_lamports, t.signer,
               b.protocol, b.direction, b.observed_tx_hash, b.event_index,
               b.via, b.via_ref,
               b.source_ref_protocol, b.source_ref_id, b.source_tx_hash,
               b.source_wallet, b.source_explorer_url,
               b.destination_ref_protocol, b.destination_ref_id, b.destination_wallet,
               b.destination_tx_hash, b.destination_explorer_url,
               b.token_ref_protocol, b.token_ref_id, b.token, b.token_symbol,
               b.amount::text AS amount, b.amount_formatted,
               b.status AS transfer_status,
               w.emitter_chain, w.emitter_address, w.sequence::text AS sequence
        FROM transactions t
        LEFT JOIN bridge_transfers b ON b.transaction_id = t.id
        LEFT JOIN wormhole_messages w ON w.transfer_id = b.id
        WHERE t.hash = $1
        ORDER BY b.event_index
        "#,
    )
    .bind(hash)
    .fetch_all(pool)
    .await?;

    let Some(first) = rows.first() else {
        return Ok(None);
    };

    let transaction = StoredTransaction {
        hash: first.try_get("hash")?,
        chain: first.try_get("chain")?,
        status: first.try_get("tx_status")?,
        slot: first.try_get("slot")?,
        timestamp: first.try_get("timestamp")?,
        fee_lamports: first.try_get("fee_lamports")?,
        signer: first.try_get("signer")?,
    };

    let mut transfers = Vec::new();
    for row in &rows {
        let Some(protocol) = row.try_get::<Option<String>, _>("protocol")? else {
            continue;
        };
        transfers.push(StoredTransfer {
            protocol,
            direction: row.try_get("direction")?,
            observed_tx_hash: row.try_get("observed_tx_hash")?,
            event_index: row.try_get("event_index")?,
            via: row.try_get("via")?,
            via_ref: row.try_get("via_ref")?,
            source_ref_protocol: row.try_get("source_ref_protocol")?,
            source_ref_id: row.try_get("source_ref_id")?,
            source_tx_hash: row.try_get("source_tx_hash")?,
            source_wallet: row.try_get("source_wallet")?,
            source_explorer_url: row.try_get("source_explorer_url")?,
            destination_ref_protocol: row.try_get("destination_ref_protocol")?,
            destination_ref_id: row.try_get("destination_ref_id")?,
            destination_wallet: row.try_get("destination_wallet")?,
            destination_tx_hash: row.try_get("destination_tx_hash")?,
            destination_explorer_url: row.try_get("destination_explorer_url")?,
            token_ref_protocol: row.try_get("token_ref_protocol")?,
            token_ref_id: row.try_get("token_ref_id")?,
            token: row.try_get("token")?,
            token_symbol: row.try_get("token_symbol")?,
            amount: row.try_get("amount")?,
            amount_formatted: row.try_get("amount_formatted")?,
            status: row.try_get("transfer_status")?,
            emitter_chain: row.try_get("emitter_chain")?,
            emitter_address: row.try_get("emitter_address")?,
            sequence: row.try_get("sequence")?,
        });
    }

    Ok(Some(StoredTransactionWithTransfers {
        transaction,
        transfers,
    }))
}
