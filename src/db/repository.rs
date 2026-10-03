use crate::domain::bridge_transfer::BridgeTransfer;
use crate::domain::transaction::NormalisedTransaction;
use anyhow::Result;
use sqlx::{PgPool, Postgres, Transaction};

pub async fn persist_transaction(pool: &PgPool, tx: &NormalisedTransaction) -> anyhow::Result<()> {
    let mut db_tx: Transaction<'_, Postgres> = pool.begin().await?;

    let transaction_id: i64 = sqlx::query_scalar(
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
    .fetch_one(&mut *db_tx)
    .await?;

    if let Some(bridge) = &tx.bridge_transfer {
        let analysis = serde_json::to_value(tx)?;
        persist_bridge_transfer(&mut db_tx, transaction_id, bridge, &analysis).await?;
    }

    db_tx.commit().await?;

    Ok(())
}

async fn persist_bridge_transfer(
    db_tx: &mut Transaction<'_, Postgres>,
    transaction_id: i64,
    bridge: &BridgeTransfer,
    analysis: &serde_json::Value,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO bridge_transfers (
            transaction_id, source_tx_hash, source_chain, source_wallet,
            source_explorer_url, emitter_chain, emitter_address, sequence,
            destination_chain, destination_wallet, destination_tx_hash,
            destination_explorer_url, token, token_symbol, amount,
            amount_formatted, status, analysis
        )
        VALUES (
            $1, $2, $3, $4, $5,
            $6, $7, $8::numeric,
            $9, $10, $11, $12,
            $13, $14, $15, $16, $17, $18
        )
        ON CONFLICT (emitter_chain, emitter_address, sequence)
        DO UPDATE SET
            destination_tx_hash = COALESCE(EXCLUDED.destination_tx_hash, bridge_transfers.destination_tx_hash),
            destination_wallet = COALESCE(EXCLUDED.destination_wallet, bridge_transfers.destination_wallet),
            destination_explorer_url = COALESCE(EXCLUDED.destination_explorer_url, bridge_transfers.destination_explorer_url),
            token_symbol = COALESCE(EXCLUDED.token_symbol, bridge_transfers.token_symbol),
            amount_formatted = COALESCE(EXCLUDED.amount_formatted, bridge_transfers.amount_formatted),
            status = CASE WHEN bridge_transfers.status = 'Completed'
                          THEN 'Completed' ELSE EXCLUDED.status END,
            -- only replace the cached response if it describes the same source tx
            analysis = CASE WHEN bridge_transfers.source_tx_hash = EXCLUDED.source_tx_hash
                            THEN EXCLUDED.analysis ELSE bridge_transfers.analysis END,
            updated_at = NOW()
        "#,
    )
    .bind(transaction_id)
    .bind(&bridge.source_tx_hash)
    .bind(bridge.source_chain.wormhole_id() as i16)
    .bind(&bridge.source_wallet)
    .bind(&bridge.source_explorer_url)
    .bind(bridge.message_id.emitter_chain as i16)
    .bind(&bridge.message_id.emitter_address)
    .bind(bridge.message_id.sequence.to_string())
    .bind(bridge.destination_chain.wormhole_id() as i16)
    .bind(&bridge.destination_wallet)
    .bind(&bridge.destination_tx_hash)
    .bind(&bridge.destination_explorer_url)
    .bind(&bridge.token)
    .bind(&bridge.token_symbol)
    .bind(&bridge.amount)
    .bind(&bridge.amount_formatted)
    .bind(format!("{:?}", bridge.status))
    .bind(analysis)
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

pub async fn set_last_ingested_slot(pool: &PgPool, chain: &str, slot: u64) -> Result<()> {
    let key = format!("{chain}_last_ingested_slot");
    sqlx::query(
        "INSERT INTO indexer_state (key, value) VALUES ($1, $2)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
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
        WHERE source_tx_hash = $1
          AND analysis IS NOT NULL
          AND (status = 'Completed' OR updated_at > NOW() - INTERVAL '30 seconds')
        "#,
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?)
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
        WHERE source_tx_hash = $1
          AND analysis IS NOT NULL
        "#,
    )
    .bind(hash)
    .bind(fresh_secs)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(analysis, fresh)| CachedAnalysis { analysis, fresh }))
}
