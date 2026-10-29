//! The reconcile precision pass must not write. Like `replay`, it calls
//! `analyse_tx`, which fetches, decodes and enriches but never persists; every
//! write in the pipeline lives in the Redis consumer and the read-through writer.
//!
//! This seeds persisted transfers, snapshots every row of the tables the pipeline
//! writes, runs the precision pass against the fake upstream, and asserts the
//! snapshot is byte-for-byte unchanged. The full-row snapshot covers the row
//! counts and the `created_at`, `updated_at` and `analysis` columns on the seeded
//! rows, and would catch an insert just as well.

#[allow(dead_code)]
mod common;

use bridge::chain_adapters::setup::build_registry;
use bridge::chains::ChainRegistry;
use bridge::config::{AppConfig, IngestConfig, ReenrichConfig};
use bridge::db::repository;
use bridge::domain::bridge_transfer::ChainId;
use bridge::services::precision::precision_pass;
use bridge::state::AppState;
use reqwest::Client;
use serde_json::Value;
use sqlx::PgPool;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// Kills the fake upstream when the test ends, including on panic.
struct FakeUpstream {
    child: Child,
    base_url: String,
}

impl Drop for FakeUpstream {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_fake_upstream(port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_fake_upstream"))
        .env("FAKE_PORT", port.to_string())
        .env("FAKE_LATENCY_MS", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn fake_upstream")
}

async fn start_fake_upstream() -> FakeUpstream {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind a free port")
        .local_addr()
        .expect("local address")
        .port();
    // The guard owns the child from here, so the panic below still kills it.
    let fake = FakeUpstream {
        child: spawn_fake_upstream(port),
        base_url: format!("http://127.0.0.1:{port}"),
    };

    let client = Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .expect("health-check client");
    for _ in 0..100 {
        if client
            .get(format!("{}/stats", fake.base_url))
            .send()
            .await
            .is_ok()
        {
            return fake;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("fake_upstream did not answer on {}", fake.base_url);
}

/// Signatures whose fixture makes the classifier produce a transfer, so the pass
/// exercises the path that would persist if it were allowed to.
async fn transfer_signatures(base_url: &str, count: usize) -> Vec<String> {
    let client = Client::new();
    let mut found = Vec::new();
    for i in 0..500 {
        let hash = format!("precision-read-only-{i}");
        let body: Value = client
            .get(format!("{base_url}/debug/fixture/{hash}"))
            .send()
            .await
            .expect("debug fixture request")
            .json()
            .await
            .expect("debug fixture body");
        let status = body["status"].as_str().unwrap_or_default();
        if status != "NoTransfer" && status != "Malformed" {
            found.push(hash);
            if found.len() == count {
                return found;
            }
        }
    }
    panic!(
        "the fixtures only hold {} transfer transactions",
        found.len()
    );
}

async fn get_transactions(base_url: &str) -> u64 {
    let body: Value = Client::new()
        .get(format!("{base_url}/stats"))
        .send()
        .await
        .expect("stats request")
        .json()
        .await
        .expect("stats body");
    body["calls"]["helius:getTransaction"].as_u64().unwrap_or(0)
}

async fn table_rows(pool: &PgPool, table: &str, order_by: &str) -> Value {
    sqlx::query_scalar(&format!(
        "SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY t.{order_by}), '[]'::jsonb)
         FROM {table} t"
    ))
    .fetch_one(pool)
    .await
    .unwrap_or_else(|error| panic!("snapshot {table}: {error}"))
}

/// Every row of every table the pipeline writes, in a stable order.
async fn snapshot(pool: &PgPool) -> Value {
    serde_json::json!({
        "transactions": table_rows(pool, "transactions", "id").await,
        "bridge_transfers": table_rows(pool, "bridge_transfers", "id").await,
        "wormhole_messages": table_rows(pool, "wormhole_messages", "transfer_id").await,
        "indexer_state": table_rows(pool, "indexer_state", "key").await,
    })
}

fn test_config(database_url: &str, upstream_url: &str) -> AppConfig {
    AppConfig {
        database_url: database_url.to_string(),
        // The analysis path never opens Redis; a value here is only a reminder that
        // nothing in the pass should need it.
        redis_url: "redis://127.0.0.1:1".to_string(),
        helius_url: upstream_url.to_string(),
        helius_live_url: upstream_url.to_string(),
        helius_reconcile_url: upstream_url.to_string(),
        wormhole_url: upstream_url.to_string(),
        helius_api_rps: 20,
        helius_live_rps: 20,
        reconcile_rps: 20,
        wormhole_rps: 20,
        upstream_permits: 4,
        port: 0,
        solana_token_bridge_program: "worm2ZoG2kUd4vFXhvjh93UUH596ayRfgQ2MgjNMTth".to_string(),
        solana_ws_url: "ws://127.0.0.1:0".to_string(),
        allowed_origins: Vec::new(),
        ingest: IngestConfig {
            enabled: false,
            backfill: false,
            backfill_max: 0,
            backfill_pace_ms: 0,
            dedupe_ttl_secs: 0,
        },
        reenrich: ReenrichConfig {
            enabled: false,
            interval_secs: 1,
            batch: 1,
            min_age_secs: 0,
            retry_secs: 0,
            max_age_secs: 1,
        },
        chain_registry_refresh_secs: 60,
    }
}

#[tokio::test]
async fn the_precision_pass_leaves_every_table_untouched() {
    let fake = start_fake_upstream().await;
    let hashes = transfer_signatures(&fake.base_url, 3).await;

    let db = common::test_db().await;

    // Three persisted transactions in the reconciliation window.
    for (index, hash) in hashes.iter().enumerate() {
        let tx = common::sample_tx(hash, 9000 + index as u64, ChainId::Ethereum, Some("1000"));
        repository::persist_analysis(&db.pool, &tx, &[])
            .await
            .expect("seed a persisted transfer");
    }

    let before = snapshot(&db.pool).await;
    assert_eq!(before["transactions"].as_array().unwrap().len(), 3);
    assert_eq!(before["bridge_transfers"].as_array().unwrap().len(), 3);
    assert_eq!(before["wormhole_messages"].as_array().unwrap().len(), 3);
    let calls_before = get_transactions(&fake.base_url).await;

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("http client");
    // The state is handed the pool directly; the analysis path never reads
    // `database_url`. Nothing in the pass may need it, which is the point.
    let config = test_config("postgres://unused", &fake.base_url);
    let registry = build_registry(&config, &client, config.helius_api_rps);
    let chains = ChainRegistry::load(&db.pool).await.expect("chain registry");
    let state =
        AppState::new(db.pool.clone(), config, client, registry, chains).expect("app state");

    let sample: Vec<&str> = hashes.iter().map(String::as_str).collect();
    let report = precision_pass(&state, &sample).await;

    // The pass really ran against the fake upstream and produced transfers: this is
    // the shape that would have been written if the pass persisted.
    assert_eq!(report.sampled, hashes.len());
    assert!(
        report.summary.transfers() > 0,
        "no transfer was re-analysed, so the test would not prove a write-free transfer path"
    );
    let calls_after = get_transactions(&fake.base_url).await;
    assert!(
        calls_after >= calls_before + hashes.len() as u64,
        "expected one getTransaction per sampled signature: {calls_before} -> {calls_after}"
    );

    let after = snapshot(&db.pool).await;
    assert_eq!(
        after, before,
        "the precision pass changed the database: {before} -> {after}"
    );
}
