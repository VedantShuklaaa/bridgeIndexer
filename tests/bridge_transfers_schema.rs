//! The A2 schema: protocol-agnostic `bridge_transfers`, `wormhole_messages`, raw chain
//! references with optional resolution, and the migration of rows that already exist.

#[allow(dead_code)]
mod common;

use bridge::chains::{ChainRegistry, RawChainRef};
use bridge::db::repository::persist_transaction;
use bridge::domain::bridge_transfer::{BridgeStatus, ChainId};
use sqlx::PgPool;

/// The A2 migration. Everything older is the schema existing rows were written under.
const A2: i64 = 20261018120000;

async fn sql_resolve(pool: &PgPool, protocol: &str, id: &str) -> Option<i16> {
    sqlx::query_scalar("SELECT resolve_chain($1, $2)")
        .bind(protocol)
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn existing_rows_survive_the_migration() {
    let db = common::empty_db().await;
    let pool = &db.pool;
    common::migrate_before(pool, A2).await;

    // Rows as the old code wrote them: Wormhole IDs in SMALLINT columns, TEXT amounts.
    // (hash, sequence, destination chain, amount, status)
    let legacy: [(&str, i64, i16, Option<&str>, &str); 4] = [
        (
            "legacy-eth",
            1,
            2,
            Some("340282366920938463463374607431768211455"),
            "Pending",
        ),
        ("legacy-unknown", 2, 9999, Some("1.5"), "Pending"),
        ("legacy-detected", 3, 30, None, "Detected"),
        ("legacy-done", 4, 4, Some("42"), "Completed"),
    ];
    for (hash, sequence, destination, amount, status) in legacy {
        let transaction_id: i64 = sqlx::query_scalar(
            "INSERT INTO transactions (hash, chain, status, slot, fee_lamports)
             VALUES ($1, 1, 'Success', 1, 5000) RETURNING id",
        )
        .bind(hash)
        .fetch_one(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO bridge_transfers
                 (transaction_id, source_tx_hash, source_chain, emitter_chain, emitter_address,
                  sequence, destination_chain, amount, status, analysis)
             VALUES ($1, $2, 1, 1, 'aa', $3::numeric, $4, $5, $6, '{\"cached\": true}')",
        )
        .bind(transaction_id)
        .bind(hash)
        .bind(sequence.to_string())
        .bind(destination)
        .bind(amount)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
    }

    common::migrate_all(pool).await;

    type Row = (
        String,
        String,
        String,
        String,
        String,
        Option<i16>,
        Option<i16>,
        Option<String>,
        String,
        serde_json::Value,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT protocol, message_key, direction, observed_tx_hash, destination_ref_id,
                source_chain_id, destination_chain_id, amount::text, status, analysis
         FROM bridge_transfers ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 4, "no row may be lost");

    let eth = sql_resolve(pool, "wormhole", "2").await;
    let solana = sql_resolve(pool, "wormhole", "1").await;
    assert!(eth.is_some() && solana.is_some());

    for (row, (hash, _, _, _, _)) in rows.iter().zip(legacy) {
        assert_eq!(row.0, "wormhole");
        assert_eq!(row.2, "Outbound");
        assert_eq!(row.3, hash, "observed tx is the old source tx");
        assert_eq!(row.5, solana, "source chain resolved");
        assert_eq!(row.9, serde_json::json!({"cached": true}), "analysis kept");
    }

    // The message key is what MessageId::key() builds.
    assert_eq!(rows[0].1, "1/aa/1");
    assert_eq!(rows[3].1, "1/aa/4");

    // Known destination resolves; an unknown one keeps its raw reference.
    assert_eq!(rows[0].6, eth);
    assert_eq!(rows[1].4, "9999");
    assert_eq!(rows[1].6, None, "unknown chain stays unresolved");

    // u128::MAX survives as NUMERIC. A non-integer becomes NULL, a missing one stays NULL.
    assert_eq!(
        rows[0].7.as_deref(),
        Some("340282366920938463463374607431768211455")
    );
    assert_eq!(rows[1].7, None);
    assert_eq!(rows[2].7, None);
    assert_eq!(rows[3].7.as_deref(), Some("42"));

    assert_eq!(rows[3].8, "Completed");

    // The Wormhole fields moved to the detail table, one row per transfer.
    let detail: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT emitter_chain, emitter_address, sequence::text FROM wormhole_messages ORDER BY sequence",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(detail.len(), 4);
    assert_eq!(detail[0], (1, "aa".to_string(), "1".to_string()));

    // The Wormhole-shaped columns are gone.
    let leftover: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.columns
         WHERE table_name = 'bridge_transfers'
           AND column_name IN ('emitter_chain', 'emitter_address', 'sequence',
                               'source_chain', 'destination_chain')",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(leftover, 0);
}

#[tokio::test]
async fn persisting_writes_the_protocol_agnostic_shape() {
    let db = common::test_db().await;
    let pool = &db.pool;

    let tx = common::sample_tx("sig-known", 42, ChainId::Ethereum, Some("1000000"));
    persist_transaction(pool, &tx).await.unwrap();

    type Row = (
        String,
        String,
        String,
        String,
        Option<i16>,
        Option<i16>,
        String,
    );
    let row: Row = sqlx::query_as(
        "SELECT protocol, message_key, direction, destination_ref_id,
                source_chain_id, destination_chain_id, amount::text
         FROM bridge_transfers WHERE observed_tx_hash = 'sig-known'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(row.0, "wormhole");
    assert_eq!(row.1, "1/emitter-addr/42");
    assert_eq!(row.2, "Outbound");
    assert_eq!(row.3, "2");
    assert_eq!(row.4, sql_resolve(pool, "wormhole", "1").await);
    assert_eq!(row.5, sql_resolve(pool, "wormhole", "2").await);
    assert_eq!(row.6, "1000000");

    let (emitter_chain, emitter, sequence): (i32, String, String) = sqlx::query_as(
        "SELECT emitter_chain, emitter_address, sequence::text FROM wormhole_messages",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        (emitter_chain, emitter.as_str(), sequence.as_str()),
        (1, "emitter-addr", "42")
    );
}

#[tokio::test]
async fn an_unknown_chain_is_stored_as_unresolved_not_dropped() {
    let db = common::test_db().await;
    let pool = &db.pool;

    let tx = common::sample_tx("sig-unknown", 7, ChainId::Unknown(9999), Some("5"));
    persist_transaction(pool, &tx).await.unwrap();

    let (reference, resolved): (String, Option<i16>) = sqlx::query_as(
        "SELECT destination_ref_id, destination_chain_id FROM bridge_transfers
         WHERE observed_tx_hash = 'sig-unknown'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(reference, "9999");
    assert_eq!(resolved, None);
}

#[tokio::test]
async fn a_non_integer_amount_is_stored_as_null_and_does_not_fail_the_write() {
    let db = common::test_db().await;
    let pool = &db.pool;

    let tx = common::sample_tx("sig-decimal", 8, ChainId::Ethereum, Some("1.5"));
    persist_transaction(pool, &tx).await.unwrap();

    let amount: Option<String> = sqlx::query_scalar(
        "SELECT amount::text FROM bridge_transfers WHERE observed_tx_hash = 'sig-decimal'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(amount, None);
}

#[tokio::test]
async fn several_transfers_can_share_one_observed_tx() {
    let db = common::test_db().await;
    let pool = &db.pool;

    persist_transaction(
        pool,
        &common::sample_tx("sig-shared", 1, ChainId::Ethereum, None),
    )
    .await
    .unwrap();
    // A second transfer from the same tx: another message, next event index.
    let transaction_id: i64 =
        sqlx::query_scalar("SELECT id FROM transactions WHERE hash = 'sig-shared'")
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO bridge_transfers
             (transaction_id, protocol, message_key, direction, event_index, observed_tx_hash,
              source_ref_protocol, source_ref_id, destination_ref_protocol, destination_ref_id,
              status)
         VALUES ($1, 'wormhole', '1/emitter-addr/2', 'Outbound', 1, 'sig-shared',
                 'wormhole', '1', 'wormhole', '4', 'Detected')",
    )
    .bind(transaction_id)
    .execute(pool)
    .await
    .expect("two transfers per observed tx must be allowed");

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM bridge_transfers WHERE observed_tx_hash = 'sig-shared'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(count, 2);

    // The same message twice is still rejected.
    let duplicate = sqlx::query(
        "INSERT INTO bridge_transfers
             (transaction_id, protocol, message_key, direction, observed_tx_hash,
              source_ref_protocol, source_ref_id, destination_ref_protocol, destination_ref_id,
              status)
         VALUES ($1, 'wormhole', '1/emitter-addr/2', 'Outbound', 'other-sig',
                 'wormhole', '1', 'wormhole', '4', 'Detected')",
    )
    .bind(transaction_id)
    .execute(pool)
    .await;
    assert!(duplicate.is_err(), "(protocol, message_key) must be unique");
}

#[tokio::test]
async fn the_database_rejects_bad_direction_and_status_values() {
    let db = common::test_db().await;
    let pool = &db.pool;
    persist_transaction(
        pool,
        &common::sample_tx("sig-check", 1, ChainId::Ethereum, None),
    )
    .await
    .unwrap();

    for (column, value) in [("direction", "Sideways"), ("status", "Lost")] {
        let result = sqlx::query(&format!(
            "UPDATE bridge_transfers SET {column} = $1 WHERE observed_tx_hash = 'sig-check'"
        ))
        .bind(value)
        .execute(pool)
        .await;
        assert!(result.is_err(), "{column} = {value} should be rejected");
    }
}

#[tokio::test]
async fn sql_resolution_follows_the_same_rule_as_the_registry() {
    let db = common::test_db().await;
    let pool = &db.pool;

    // A proposal that is not active never resolves, in SQL or in Rust.
    sqlx::query(
        "INSERT INTO protocol_chain_ids (protocol, protocol_chain_id, chain_id, status, origin)
         SELECT 'wormhole', '9999', id, 'pending_review', 'llm' FROM chains WHERE slug = 'base'",
    )
    .execute(pool)
    .await
    .unwrap();

    // An active mapping onto a chain that is not active does not resolve either.
    sqlx::query(
        "INSERT INTO chains (slug, name, kind, status) VALUES ('ghost', 'Ghost', 'other', 'pending_review')",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO protocol_chain_ids (protocol, protocol_chain_id, chain_id)
         SELECT 'wormhole', '8888', id FROM chains WHERE slug = 'ghost'",
    )
    .execute(pool)
    .await
    .unwrap();

    let registry = ChainRegistry::load(pool).await.unwrap();
    for (protocol, id) in [
        ("wormhole", "1"),
        ("wormhole", "30"),
        ("wormhole", "9999"),
        ("wormhole", "8888"),
        ("wormhole", "424242"),
        ("evm", "8453"),
        ("evm", "1"),
        ("cctp", "0"),
    ] {
        let rust = registry
            .resolve(&RawChainRef::new(protocol, id))
            .key()
            .map(|k| k.0);
        assert_eq!(
            sql_resolve(pool, protocol, id).await,
            rust,
            "{protocol}:{id}"
        );
    }
}

#[tokio::test]
async fn activating_a_mapping_resolves_the_rows_stored_as_unresolved() {
    let db = common::test_db().await;
    let pool = &db.pool;

    persist_transaction(
        pool,
        &common::sample_tx("sig-late", 1, ChainId::Unknown(9999), None),
    )
    .await
    .unwrap();

    let base: i16 = sqlx::query_scalar("SELECT id FROM chains WHERE slug = 'base'")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO protocol_chain_ids (protocol, protocol_chain_id, chain_id)
         VALUES ('wormhole', '9999', $1)",
    )
    .bind(base)
    .execute(pool)
    .await
    .unwrap();

    let updated = sqlx::query(
        "UPDATE bridge_transfers
         SET destination_chain_id = resolve_chain(destination_ref_protocol, destination_ref_id)
         WHERE destination_chain_id IS NULL
           AND resolve_chain(destination_ref_protocol, destination_ref_id) IS NOT NULL",
    )
    .execute(pool)
    .await
    .unwrap()
    .rows_affected();
    assert_eq!(updated, 1);

    let resolved: Option<i16> = sqlx::query_scalar(
        "SELECT destination_chain_id FROM bridge_transfers WHERE observed_tx_hash = 'sig-late'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(resolved, Some(base));
}

#[tokio::test]
async fn a_later_analysis_without_enrichment_does_not_downgrade_the_status() {
    let db = common::test_db().await;
    let pool = &db.pool;

    // Pending: WormholeScan had the VAA.
    let pending = common::sample_tx("sig-down", 1, ChainId::Ethereum, Some("1"));
    persist_transaction(pool, &pending).await.unwrap();

    // Re-analysed while WormholeScan was unavailable: on-chain data only, Detected.
    let mut detected = pending.clone();
    detected.bridge_transfer.as_mut().unwrap().status = BridgeStatus::Detected;
    persist_transaction(pool, &detected).await.unwrap();

    let (status, analysis): (String, serde_json::Value) = sqlx::query_as(
        "SELECT status, analysis FROM bridge_transfers WHERE observed_tx_hash = 'sig-down'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(status, "Pending");
    assert_eq!(
        analysis["bridge_transfer"]["status"], "Pending",
        "the cached response must not describe an older status than the row"
    );

    // Detected can still be raised.
    let mut completed = pending.clone();
    completed.bridge_transfer.as_mut().unwrap().status = BridgeStatus::Completed;
    persist_transaction(pool, &completed).await.unwrap();
    let status: String = sqlx::query_scalar(
        "SELECT status FROM bridge_transfers WHERE observed_tx_hash = 'sig-down'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(status, "Completed");
}
