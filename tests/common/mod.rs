//! Shared Postgres setup for integration tests.
//!
//! By default each test starts its own Postgres container (needs Docker), like CI.
//! Set `TEST_DATABASE_URL` to a server you already run and each test gets a fresh
//! database on it instead, which is handy where Docker is unavailable.

use sqlx::{PgPool, postgres::PgConnectOptions};
use std::str::FromStr;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{ContainerAsync, runners::AsyncRunner},
};

pub struct TestDb {
    pub pool: PgPool,
    // Held so the container lives as long as the test.
    _container: Option<ContainerAsync<Postgres>>,
}

/// A migrated, empty-of-test-data database.
pub async fn test_db() -> TestDb {
    let (pool, container) = match std::env::var("TEST_DATABASE_URL") {
        Ok(url) => (fresh_database(&url).await, None),
        Err(_) => {
            let container = Postgres::default()
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

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("failed to run migrations");

    TestDb {
        pool,
        _container: container,
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
