use sqlx::{PgPool, Postgres, Transaction};

use crate::domain::bridge_transfer::BridgeTransfer;
use crate::domain::transaction::NormalisedTransaction;

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
        persist_bridge_transfer(&mut db_tx, transaction_id, bridge).await?;
    }

    db_tx.commit().await?;

    Ok(())
}

async fn persist_bridge_transfer(
    db_tx: &mut Transaction<'_, Postgres>,
    transaction_id: i64,
    bridge: &BridgeTransfer,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO bridge_transfers (
            transaction_id,
            source_tx_hash,
            source_chain,
            source_wallet,
            source_explorer_url,
            emitter_chain,
            emitter_address,
            sequence,
            destination_chain,
            destination_wallet,
            destination_tx_hash,
            destination_explorer_url,
            token,
            token_symbol,
            amount,
            amount_formatted,
            status
        )
        VALUES (
            $1, $2, $3, $4, $5,
            $6, $7, $8,
            $9, $10, $11, $12,
            $13, $14, $15, $16, $17
        )
        ON CONFLICT (source_tx_hash)
        DO UPDATE SET
            destination_tx_hash = EXCLUDED.destination_tx_hash,
            destination_wallet = EXCLUDED.destination_wallet,
            destination_explorer_url = EXCLUDED.destination_explorer_url,
            token_symbol = EXCLUDED.token_symbol,
            amount_formatted = EXCLUDED.amount_formatted,
            status = EXCLUDED.status,
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
    .bind(bridge.message_id.sequence as i64)
    .bind(bridge.destination_chain.wormhole_id() as i16)
    .bind(&bridge.destination_wallet)
    .bind(&bridge.destination_tx_hash)
    .bind(&bridge.destination_explorer_url)
    .bind(&bridge.token)
    .bind(&bridge.token_symbol)
    .bind(&bridge.amount)
    .bind(&bridge.amount_formatted)
    .bind(format!("{:?}", bridge.status))
    .execute(&mut **db_tx)
    .await?;

    Ok(())
}
