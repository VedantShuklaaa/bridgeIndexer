//! Write-path benchmark for the Redis stream consumer.
//!
//! Pre-fills `bridge:transactions` with `BENCH_N` signatures that the fake upstream
//! replays as Completed transfers, starts consumer workers, and reports msgs/s and
//! the publish-to-persist latency distribution until the stream is drained.
//!
//! Requires the fake upstream, Redis and Postgres (with migrations runnable) to be
//! reachable. Point HELIUS_URL and WORMHOLE_URL at the fake. The consumer's own
//! knobs come from the app config; this binary only decides how many workers to
//! start (`BENCH_WORKERS`, default 5, the production count).
//!
//!   BENCH_N=5000 BENCH_WORKERS=5 \
//!   HELIUS_URL=http://127.0.0.1:9090 WORMHOLE_URL=http://127.0.0.1:9090 \
//!   HELIUS_LIVE_RPS=1000 WORMHOLE_RPS=1000 \
//!   cargo run --release --bin bench_write_path
//!
//! Output is one block per run: reps/s, p50/p99, the database row delta, dead
//! letters and upstream calls per message. A run that drops a message is reported
//! as INVALID rather than compared.

use anyhow::{Context, Result};
use bridge::chain_adapters::setup::build_registry;
use bridge::chains::ChainRegistry;
use bridge::config::{AppConfig, Lane};
use bridge::db::connection::connect;
use bridge::redis::consumer::RedisConsumer;
use bridge::state::AppState;
use chrono::{DateTime, Utc};
use redis::Value;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

const BRIDGE_TX_STREAM: &str = "bridge:transactions";
const CONSUMER_GROUP: &str = "bridge-workers";
const FAILED_TX_STREAM: &str = "bridge:transactions:failed";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    // The consumer records limiter wait and upstream-call metrics. Serve them so
    // the run can say whether the provider quota, not the write path, was waiting.
    bridge::metrics::init()?;
    let metrics_port = std::env::var("METRICS_PORT").unwrap_or_else(|_| "9100".into());

    let n = number("BENCH_N", 5000usize);
    let workers = number("BENCH_WORKERS", 5usize).max(1);
    let prefix = std::env::var("BENCH_PREFIX")
        .unwrap_or_else(|_| format!("bench-{}", Utc::now().timestamp_millis()));
    let fake_url = std::env::var("FAKE_URL").unwrap_or_else(|_| "http://127.0.0.1:9090".into());
    let timeout = Duration::from_secs(number("BENCH_TIMEOUT_SECS", 7200u64));

    let config = AppConfig::from_env()?;
    let pool = connect(&config.database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let fake = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    reset_fake_stats(&fake, &fake_url).await?;

    let signatures = select_signatures(&fake, &fake_url, &prefix, n).await?;
    anyhow::ensure!(
        signatures.len() == n,
        "wanted {n} Completed fixtures, selected {}",
        signatures.len()
    );

    let mut redis = redis::Client::open(config.redis_url.as_str())?
        .get_multiplexed_async_connection()
        .await?;
    reset_streams(&mut redis).await?;

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let registry = build_registry(&config, &http_client, Lane::Ingest);
    let chains = ChainRegistry::load(&pool).await?;
    let state =
        AppState::new_stream(pool.clone(), config.clone(), http_client, registry, chains).await?;

    let before_tx = row_count(&pool, "transactions").await?;
    let before_bt = row_count(&pool, "bridge_transfers").await?;

    let publish_times = fill_stream(&mut redis, &signatures).await?;

    eprintln!("bench: stream filled with {n} messages, starting {workers} consumers");
    let shutdown = CancellationToken::new();
    let started = Instant::now();
    let mut tasks = Vec::with_capacity(workers + 1);
    for worker in 0..workers {
        let consumer = RedisConsumer::new(
            &config.redis_url,
            state.clone(),
            format!("bench-worker-{worker}"),
            shutdown.clone(),
        )?;
        tasks.push(tokio::spawn(async move { consumer.run().await }));
    }
    // Recovery runs in production; without it a message that fails its retries
    // stays pending for good, and the benchmark would never drain.
    let recovery = RedisConsumer::new(
        &config.redis_url,
        state.clone(),
        "bench-recovery".to_string(),
        shutdown.clone(),
    )?;
    tasks.push(tokio::spawn(async move { recovery.run_recovery().await }));

    let drained = wait_for_drain(
        &mut redis,
        &pool,
        &signatures,
        n,
        before_tx,
        started,
        timeout,
    )
    .await?;
    let elapsed = started.elapsed();
    shutdown.cancel();
    for task in &tasks {
        task.abort();
    }
    for task in tasks {
        let _ = task.await;
    }

    let after_tx = row_count(&pool, "transactions").await?;
    let after_bt = row_count(&pool, "bridge_transfers").await?;
    let dead_letters: i64 = redis::cmd("XLEN")
        .arg(FAILED_TX_STREAM)
        .query_async(&mut redis)
        .await
        .unwrap_or(0);
    let pending = pending_count(&mut redis).await.unwrap_or(-1);

    let latencies = publish_to_persist_ms(&pool, &signatures, &publish_times).await?;
    anyhow::ensure!(!latencies.is_empty(), "no persisted rows to measure");

    let tx_delta = after_tx - before_tx;
    let bt_delta = after_bt - before_bt;
    let valid = drained && tx_delta == n as i64 && dead_letters == 0 && pending == 0;

    let (p50, p99) = percentiles_ms(&latencies);
    let rate = n as f64 / elapsed.as_secs_f64();
    let stats = fake_stats(&fake, &fake_url).await?;

    println!();
    println!(
        "### write-path {} n={} workers={}",
        if valid { "OK" } else { "INVALID" },
        n,
        workers
    );
    println!(
        "elapsed: {:.3}s | {:.1} msgs/s | p50 {:.2} ms | p99 {:.2} ms",
        elapsed.as_secs_f64(),
        rate,
        p50,
        p99
    );
    println!(
        "rows: transactions +{} (expected {}) | bridge_transfers +{} | dead_letters {} | pending {}",
        tx_delta, n, bt_delta, dead_letters, pending
    );
    println!(
        "config: workers={} read_count={} db_pool={} helius_live_rps={} wormhole_rps={} helius_wait_ms={}",
        config.worker.worker_count,
        config.worker.read_count,
        number("DB_POOL_MAX", 20usize),
        config.helius_live_rps,
        config.wormhole_rps,
        config.helius_live_token_wait.as_millis(),
    );
    println!(
        "upstream per message: getTransaction={:.2} getMultipleAccounts={:.2} getAsset={:.2} operations={:.2}",
        calls(&stats, "helius:getTransaction") / n as f64,
        calls(&stats, "helius:getMultipleAccounts") / n as f64,
        calls(&stats, "helius:getAsset") / n as f64,
        calls(&stats, "wormhole:operations") / n as f64,
    );
    println!(
        "upstream in-flight high-water: {}",
        stats["in_flight_high_water"].as_u64().unwrap_or(0)
    );
    match limiter_wait(&metrics_port).await {
        Some((sum, count)) if count > 0.0 => println!(
            "helius ingest limiter wait: avg {:.1} ms over {:.0} acquisitions",
            sum / count * 1000.0,
            count
        ),
        _ => println!("helius ingest limiter wait: no samples"),
    }

    if !valid {
        anyhow::bail!(
            "run is INVALID: drained={drained}, rows {tx_delta}/{n}, dead_letters={dead_letters}, pending={pending}"
        );
    }

    Ok(())
}

/// Signatures whose fixture the fake resolves to Completed. Only those persist a
/// transfer, so the row-delta check means "no message was dropped".
async fn select_signatures(
    fake: &reqwest::Client,
    fake_url: &str,
    prefix: &str,
    n: usize,
) -> Result<Vec<String>> {
    use futures_util::stream::{self, StreamExt};

    let base = fake_url.trim_end_matches('/');
    let mut selected = Vec::with_capacity(n);
    let mut index = 0usize;
    while selected.len() < n {
        let batch: Vec<String> = (index..index + 500)
            .map(|i| base58(&sha512(format!("{prefix}/{i}").as_bytes())))
            .collect();
        index += batch.len();
        let results = stream::iter(batch)
            .map(|signature| {
                let fake = fake.clone();
                let url = format!("{base}/debug/fixture/{signature}");
                async move {
                    let info: serde_json::Value = fake
                        .get(&url)
                        .send()
                        .await
                        .context("fake /debug/fixture")?
                        .json()
                        .await?;
                    Ok::<_, anyhow::Error>((
                        signature,
                        info["completed"].as_bool().unwrap_or(false),
                    ))
                }
            })
            .buffer_unordered(32)
            .collect::<Vec<_>>()
            .await;
        for result in results {
            let (signature, completed) = result?;
            if completed && selected.len() < n {
                selected.push(signature);
            }
        }
    }
    selected.sort();
    Ok(selected)
}

async fn reset_streams(connection: &mut redis::aio::MultiplexedConnection) -> Result<()> {
    let _: i64 = redis::cmd("DEL")
        .arg(BRIDGE_TX_STREAM)
        .arg(FAILED_TX_STREAM)
        .query_async(connection)
        .await?;
    Ok(())
}

async fn fill_stream(
    connection: &mut redis::aio::MultiplexedConnection,
    signatures: &[String],
) -> Result<Vec<DateTime<Utc>>> {
    let mut times = Vec::with_capacity(signatures.len());
    for (index, chunk) in signatures.chunks(500).enumerate() {
        let mut pipe = redis::pipe();
        for (offset, signature) in chunk.iter().enumerate() {
            let slot = 1_000_000 + (index * 500 + offset) as u64;
            let mut add = redis::cmd("XADD");
            add.arg(BRIDGE_TX_STREAM)
                .arg("*")
                .arg("chain")
                .arg("solana")
                .arg("tx_hash")
                .arg(signature)
                .arg("slot")
                .arg(slot);
            pipe.add_command(add);
            times.push(Utc::now());
        }
        let _: Value = pipe.query_async(connection).await?;
    }
    Ok(times)
}

/// Drained means the group has no undelivered entries and no unacked ones, and
/// the database holds every expected row. Checking both sides catches a consumer
/// that acks without persisting.
async fn wait_for_drain(
    connection: &mut redis::aio::MultiplexedConnection,
    pool: &PgPool,
    _signatures: &[String],
    n: usize,
    before_tx: i64,
    started: Instant,
    timeout: Duration,
) -> Result<bool> {
    loop {
        if started.elapsed() > timeout {
            anyhow::bail!(
                "timed out after {:?} waiting for the stream to drain",
                timeout
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;

        let rows = row_count(pool, "transactions").await? - before_tx;
        let (lag, pending) = group_lag_and_pending(connection).await.unwrap_or((1, 1));
        if rows >= n as i64 && lag == 0 && pending == 0 {
            return Ok(true);
        }
    }
}

async fn group_lag_and_pending(
    connection: &mut redis::aio::MultiplexedConnection,
) -> Result<(i64, i64)> {
    let response: Value = redis::cmd("XINFO")
        .arg("GROUPS")
        .arg(BRIDGE_TX_STREAM)
        .query_async(&mut *connection)
        .await?;
    let Value::Array(groups) = response else {
        return Ok((1, 1));
    };
    let Some(Value::Array(fields)) = groups.first() else {
        return Ok((1, 1));
    };
    let (mut lag, mut pending) = (0i64, 0i64);
    for pair in fields.chunks(2) {
        let (Value::BulkString(key), value) = (&pair[0], &pair[1]) else {
            continue;
        };
        match key.as_slice() {
            b"pending" => pending = int_value(value),
            b"lag" => lag = int_value(value),
            _ => {}
        }
    }
    Ok((lag, pending))
}

fn int_value(value: &Value) -> i64 {
    match value {
        Value::Int(n) => *n,
        Value::BulkString(bytes) => String::from_utf8_lossy(bytes).parse().unwrap_or(0),
        _ => 0,
    }
}

async fn pending_count(connection: &mut redis::aio::MultiplexedConnection) -> Result<i64> {
    let response: Value = redis::cmd("XPENDING")
        .arg(BRIDGE_TX_STREAM)
        .arg(CONSUMER_GROUP)
        .query_async(connection)
        .await?;
    let Value::Array(parts) = response else {
        return Ok(0);
    };
    Ok(parts.first().map(int_value).unwrap_or(0))
}

async fn row_count(pool: &PgPool, table: &str) -> Result<i64> {
    let sql = format!("SELECT count(*) FROM {table}");
    Ok(sqlx::query_scalar(&sql).fetch_one(pool).await?)
}

async fn publish_to_persist_ms(
    pool: &PgPool,
    signatures: &[String],
    publish_times: &[DateTime<Utc>],
) -> Result<Vec<f64>> {
    use std::collections::HashMap;

    let rows: Vec<(String, DateTime<Utc>)> =
        sqlx::query_as("SELECT hash, created_at FROM transactions WHERE hash = ANY($1::text[])")
            .bind(signatures)
            .fetch_all(pool)
            .await?;
    let created: HashMap<&str, DateTime<Utc>> =
        rows.iter().map(|(hash, at)| (hash.as_str(), *at)).collect();

    Ok(signatures
        .iter()
        .zip(publish_times)
        .filter_map(|(signature, published)| {
            let at = created.get(signature.as_str())?;
            let ms = (*at - *published).num_microseconds()? as f64 / 1000.0;
            Some(ms.max(0.0))
        })
        .collect())
}

fn percentiles_ms(values: &[f64]) -> (f64, f64) {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pick = |q: f64| {
        let index = ((sorted.len() as f64 - 1.0) * q).round() as usize;
        sorted[index]
    };
    (pick(0.50), pick(0.99))
}

async fn reset_fake_stats(fake: &reqwest::Client, fake_url: &str) -> Result<()> {
    let url = format!("{}/stats/reset", fake_url.trim_end_matches('/'));
    fake.post(&url).send().await?.error_for_status()?;
    Ok(())
}

async fn fake_stats(fake: &reqwest::Client, fake_url: &str) -> Result<serde_json::Value> {
    let url = format!("{}/stats", fake_url.trim_end_matches('/'));
    Ok(fake
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// The Helius ingest-lane histogram sum and count from the local Prometheus
/// endpoint, as recorded by `acquire_token_within`.
async fn limiter_wait(port: &str) -> Option<(f64, f64)> {
    let body = reqwest::get(format!("http://127.0.0.1:{port}/metrics"))
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    let mut sum = None;
    let mut count = None;
    for line in body.lines() {
        if !line.contains("provider=\"helius\"") || !line.contains("lane=\"ingest\"") {
            continue;
        }
        if line.starts_with("limiter_wait_seconds_sum") {
            sum = line.rsplit(' ').next()?.parse::<f64>().ok();
        } else if line.starts_with("limiter_wait_seconds_count") {
            count = line.rsplit(' ').next()?.parse::<f64>().ok();
        }
    }
    Some((sum?, count?))
}

fn calls(stats: &serde_json::Value, method: &str) -> f64 {
    stats["calls"][method].as_f64().unwrap_or(0.0)
}

fn number<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn base58(raw: &[u8]) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut digits = Vec::with_capacity(raw.len() * 2);
    for byte in raw {
        let mut carry = u32::from(*byte);
        for digit in &mut digits {
            carry += u32::from(*digit) << 8;
            *digit = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let zeros = raw.iter().take_while(|byte| **byte == 0).count();
    let mut out = String::with_capacity(zeros + digits.len());
    out.extend(std::iter::repeat_n('1', zeros));
    for digit in digits.iter().rev() {
        out.push(ALPHABET[*digit as usize] as char);
    }
    out
}

/// A deterministic, dependency-free stand-in for a signature: the fake hashes the
/// string anyway, and this only has to spread candidates over fixtures.
fn sha512(input: &[u8]) -> [u8; 64] {
    let mut out = [0u8; 64];
    let mut state = 0xcbf2_9ce4_8422_2325u64;
    for byte in input {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
    for (index, chunk) in out.chunks_mut(8).enumerate() {
        state ^= index as u64;
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
        let mut mixed = state ^ state.rotate_left(27);
        mixed = mixed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let bytes = mixed.to_be_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    out
}
