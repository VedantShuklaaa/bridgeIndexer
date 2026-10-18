use bridge::domain::bridge_transfer::{BridgeMessageId, BridgeStatus, BridgeTransfer, ChainId};
use bridge::domain::transaction::{Chain, NormalisedTransaction, TxStatus};
use sqlx::{PgPool, postgres::PgConnectOptions};
use std::borrow::Cow;
use std::str::FromStr;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

/// The production database is Postgres 18, so tests run against 18 and not older.
const POSTGRES_TAG: &str = "18";

pub struct TestDb {
    pub pool: PgPool,
    // Held so the container lives as long as the test.
    _container: Option<ContainerAsync<Postgres>>,
}

/// A migrated database with no test data.
pub async fn test_db() -> TestDb {
    let db = empty_db().await;
    sqlx::migrate!("./migrations")
        .run(&db.pool)
        .await
        .expect("failed to run migrations");
    db
}

/// A database with no migrations applied, for tests that stage old data first.
pub async fn empty_db() -> TestDb {
    let (pool, container) = match std::env::var("TEST_DATABASE_URL") {
        Ok(url) => (fresh_database(&url).await, None),
        Err(_) => {
            let container = Postgres::default()
                .with_tag(POSTGRES_TAG)
                .start()
                .await
                .expect("failed to start postgres container");
            let port = container
                .get_host_port_ipv4(5432)
                .await
                .expect("failed to get mapped port");
            let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
            let pool = PgPool::connect(&url)
                .await
                .expect("failed to connect to test postgres");
            (pool, Some(container))
        }
    };

    TestDb {
        pool,
        _container: container,
    }
}

/// Applies only the migrations older than `version`. Running the full set afterwards
/// applies the rest, which is how a migration is tested against existing rows.
pub async fn migrate_before(pool: &PgPool, version: i64) {
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.migrations = Cow::Owned(
        migrator
            .migrations
            .iter()
            .filter(|m| m.version < version)
            .cloned()
            .collect(),
    );
    migrator
        .run(pool)
        .await
        .expect("failed to run earlier migrations");
}

pub async fn migrate_all(pool: &PgPool) {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .expect("failed to run migrations");
}

/// A Solana tx holding one Wormhole send to `destination`.
pub fn sample_tx(
    hash: &str,
    sequence: u64,
    destination: ChainId,
    amount: Option<&str>,
) -> NormalisedTransaction {
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
            destination_chain: destination,
            destination_wallet: None,
            destination_tx_hash: None,
            destination_explorer_url: None,
            token: None,
            token_symbol: Some("W".to_string()),
            amount: amount.map(String::from),
            amount_formatted: Some("1.0".to_string()),
            message_id: BridgeMessageId {
                emitter_chain: 1,
                emitter_address: "emitter-addr".to_string(),
                sequence,
            },
            status: BridgeStatus::Pending,
        }),
    }
}

async fn fresh_database(base_url: &str) -> PgPool {
    let admin_options = PgConnectOptions::from_str(base_url).expect("invalid TEST_DATABASE_URL");
    let admin = PgPool::connect_with(admin_options.clone())
        .await
        .expect("failed to connect to TEST_DATABASE_URL");

    let name = format!("bridge_test_{:016x}", rand::random::<u64>());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .expect("failed to create test database");

    PgPool::connect_with(admin_options.database(&name))
        .await
        .expect("failed to connect to test database")
}
