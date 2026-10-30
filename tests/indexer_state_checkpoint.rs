//! The ingestion checkpoint is monotonic: out-of-order log notifications must never rewind
//! it, because the next backfill resumes from that slot.

mod common;

use bridge::db::repository::{get_last_ingested_slot, set_last_ingested_slot};

#[tokio::test]
async fn the_checkpoint_never_moves_backwards() {
    let db = common::test_db().await;
    let pool = &db.pool;

    set_last_ingested_slot(pool, "solana", 100).await.unwrap();
    assert_eq!(get_last_ingested_slot(pool, "solana").await.unwrap(), 100);

    set_last_ingested_slot(pool, "solana", 90).await.unwrap();
    assert_eq!(get_last_ingested_slot(pool, "solana").await.unwrap(), 100);

    set_last_ingested_slot(pool, "solana", 110).await.unwrap();
    assert_eq!(get_last_ingested_slot(pool, "solana").await.unwrap(), 110);
}

#[tokio::test]
async fn a_fresh_database_starts_at_slot_zero() {
    let db = common::test_db().await;

    assert_eq!(get_last_ingested_slot(&db.pool, "solana").await.unwrap(), 0);
}
