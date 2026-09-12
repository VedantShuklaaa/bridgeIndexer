use bridge::db::repository::persist_transaction;
use bridge::domain::bridge_transfer::{BridgeMessageId, BridgeStatus, BridgeTransfer, ChainId};
use bridge::domain::transaction::{Chain, NormalisedTransaction, TxStatus};
use sqlx::PgPool;
use testcontainers_modules::{postgres::Postgres, testcontainers::runners::AsyncRunner};

fn sample_tx(hash: &str) -> NormalisedTransaction {
    NormalisedTransaction {
        hash: hash.to_string(),
        chain: Chain::Solana,
        status: TxStatus::Success,
        slot: 123,
        timestamp: Some(1_700_000_000),
        fee_lamports: 5000,
        signer: Some("SomeSignerPubkey".to_string()),
        bridge_event: None,
        bridge_transfer: Some(BridgeTransfer {
            source_chain: ChainId::Solana,
            source_tx_hash: hash.to_string(),
            source_wallet: Some("SomeSignerPubkey".to_string()),
            source_explorer_url: None,
            destination_chain: ChainId::Ethereum,
            destination_wallet: None,
            destination_tx_hash: None,
            destination_explorer_url: None,
            token: None,
            token_symbol: Some("W".to_string()),
            amount: Some("1000000".to_string()),
            amount_formatted: Some("1.0".to_string()),
            message_id: BridgeMessageId {
                emitter_chain: 1,
                emitter_address: "emitter-addr".to_string(),
                sequence: 42,
            },
            status: BridgeStatus::Pending,
        }),
    }
}

#[tokio::test]
async fn redelivering_the_same_transaction_does_not_duplicate_rows() {
    let container = Postgres::default()
        .start()
        .await
        .expect("failed to start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get mapped port");
    let database_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let pool = PgPool::connect(&database_url)
        .await
        .expect("failed to connect to test postgres");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("failed to run migrations");

    let tx = sample_tx("dup-test-signature-123");

    // Simulates XAUTOCLAIM redelivering the same message after a crash.
    persist_transaction(&pool, &tx)
        .await
        .expect("first persist failed");
    persist_transaction(&pool, &tx)
        .await
        .expect("second persist (redelivery) failed");

    let tx_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE hash = $1")
        .bind(&tx.hash)
        .fetch_one(&pool)
        .await
        .unwrap();

    let bridge_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM bridge_transfers WHERE source_tx_hash = $1")
            .bind(&tx.hash)
            .fetch_one(&pool)
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
}
