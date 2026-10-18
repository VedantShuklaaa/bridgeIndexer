#[allow(dead_code)]
mod common;

use bridge::db::repository::persist_transaction;
use bridge::domain::bridge_transfer::ChainId;

#[tokio::test]
async fn redelivering_the_same_transaction_does_not_duplicate_rows() {
    let db = common::test_db().await;
    let pool = &db.pool;

    let tx = common::sample_tx(
        "dup-test-signature-123",
        42,
        ChainId::Ethereum,
        Some("1000000"),
    );

    // Simulates XAUTOCLAIM redelivering the same message after a crash.
    persist_transaction(pool, &tx)
        .await
        .expect("first persist failed");
    persist_transaction(pool, &tx)
        .await
        .expect("second persist (redelivery) failed");

    let tx_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE hash = $1")
        .bind(&tx.hash)
        .fetch_one(pool)
        .await
        .unwrap();

    let bridge_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bridge_transfers WHERE observed_tx_hash = $1")
            .bind(&tx.hash)
            .fetch_one(pool)
            .await
            .unwrap();

    let detail_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wormhole_messages")
        .fetch_one(pool)
        .await
        .unwrap();

    assert_eq!(
        tx_count, 1,
        "duplicate delivery created a second transactions row"
    );
    assert_eq!(
        bridge_count, 1,
        "duplicate delivery created a second bridge_transfers row"
    );
    assert_eq!(
        detail_count, 1,
        "duplicate delivery created a second wormhole_messages row"
    );
}
