//! Replays dead-lettered transactions back onto the main ingestion stream.
//!
//!   cargo run --bin redrive -- --error-contains "VAA not available" --limit 20
//!   cargo run --bin redrive -- --execute --pace-ms 50
//!
//! A dry run by default: it reads `bridge:transactions:failed` and reports what it
//! would re-publish. With `--execute` each selected entry is XADDed to
//! `bridge:transactions` with the producer's three fields (chain, tx_hash, slot)
//! and, unless `--keep` is given, XDELed from the dead-letter stream. Persistence
//! is idempotent, so re-driving an entry that is already stored is safe.
//!
//! Needs `REDIS_URL` in the environment. It is never printed.

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use redis::Value;
use redis::aio::MultiplexedConnection;
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

const BRIDGE_TX_STREAM: &str = "bridge:transactions";
const FAILED_TX_STREAM: &str = "bridge:transactions:failed";
const PAGE_SIZE: usize = 100;
const API_KEY_MARKER: &str = "api-key=";

const USAGE: &str = "usage: redrive [--execute] [--pace-ms N] [--limit N]
               [--error-contains TEXT]... [--exclude-error-contains TEXT]...
               [--keep] [--verbose]

  --execute                 re-publish the selected entries (default: dry run)
  --pace-ms N               delay between re-publishes (default 500)
  --limit N                 process at most N selected entries (default: all)
  --error-contains TEXT     only entries whose error contains TEXT; repeatable (OR)
  --exclude-error-contains  skip entries whose error contains TEXT; repeatable
  --keep                    keep dead-letter entries after re-publishing
  --verbose                 print tx hashes (never printed otherwise)";

fn redis_url() -> Result<String> {
    std::env::var("REDIS_URL").map_err(|_| anyhow!("REDIS_URL not set"))
}

struct Args {
    execute: bool,
    pace_ms: u64,
    limit: Option<usize>,
    error_contains: Vec<String>,
    exclude_error_contains: Vec<String>,
    keep: bool,
    verbose: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        execute: false,
        pace_ms: 500,
        limit: None,
        error_contains: Vec::new(),
        exclude_error_contains: Vec::new(),
        keep: false,
        verbose: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--help" || flag == "-h" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        let mut value = || {
            it.next()
                .ok_or_else(|| anyhow!("{flag} needs a value\n\n{USAGE}"))
        };
        match flag.as_str() {
            "--execute" => args.execute = true,
            "--pace-ms" => args.pace_ms = value()?.parse()?,
            "--limit" => args.limit = Some(value()?.parse()?),
            "--error-contains" => args.error_contains.push(value()?),
            "--exclude-error-contains" => args.exclude_error_contains.push(value()?),
            "--keep" => args.keep = true,
            "--verbose" => args.verbose = true,
            other => anyhow::bail!("unknown argument {other}\n\n{USAGE}"),
        }
    }
    Ok(args)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeadLetter {
    id: String,
    chain: String,
    tx_hash: String,
    slot: u64,
    error: String,
}

/// Turns one stream entry into a candidate, or gives the reason it cannot be replayed.
fn parse_entry(id: &str, fields: &HashMap<String, String>) -> Result<DeadLetter, &'static str> {
    let chain = fields.get("chain").ok_or("missing chain")?;
    let tx_hash = fields.get("tx_hash").ok_or("missing tx_hash")?;
    let error = fields.get("error").ok_or("missing error")?;
    let slot = fields
        .get("slot")
        .ok_or("missing slot")?
        .parse::<u64>()
        .map_err(|_| "unparsable slot")?;

    Ok(DeadLetter {
        id: id.to_string(),
        chain: chain.clone(),
        tx_hash: tx_hash.clone(),
        slot,
        error: error.clone(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Category {
    ServiceOverloaded,
    TxVersion32015,
    BuilderError,
    VaaNotAvailable,
    TransactionNotFound,
    RateLimited,
    Other,
}

impl Category {
    const ALL: [Category; 7] = [
        Category::ServiceOverloaded,
        Category::TxVersion32015,
        Category::BuilderError,
        Category::VaaNotAvailable,
        Category::TransactionNotFound,
        Category::RateLimited,
        Category::Other,
    ];

    fn classify(error: &str) -> Self {
        let error = error.to_ascii_lowercase();
        if error.contains("service overloaded") {
            Category::ServiceOverloaded
        } else if error.contains("-32015") {
            Category::TxVersion32015
        } else if error.contains("builder error") {
            Category::BuilderError
        } else if error.contains("vaa not available") {
            Category::VaaNotAvailable
        } else if error.contains("transaction not found") {
            Category::TransactionNotFound
        } else if error.contains("rate limited") {
            Category::RateLimited
        } else {
            Category::Other
        }
    }

    fn label(self) -> &'static str {
        match self {
            Category::ServiceOverloaded => "service overloaded",
            Category::TxVersion32015 => "tx version -32015",
            Category::BuilderError => "builder error",
            Category::VaaNotAvailable => "VAA not available",
            Category::TransactionNotFound => "transaction not found",
            Category::RateLimited => "rate limited",
            Category::Other => "other",
        }
    }
}

/// Removes the value of any `api-key=...` query parameter. Stored error text can
/// contain the provider URL, and the provider URL carries the key.
fn sanitize(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;

    while let Some(found) = lower[cursor..].find(API_KEY_MARKER) {
        let value_start = cursor + found + API_KEY_MARKER.len();
        out.push_str(&input[cursor..value_start]);

        let rest = &input[value_start..];
        let value_end = rest
            .find(|c: char| {
                c == '&' || c.is_whitespace() || c == '"' || c == '\'' || c == ')' || c == ']'
            })
            .unwrap_or(rest.len());

        out.push_str("[redacted]");
        cursor = value_start + value_end;
    }

    out.push_str(&input[cursor..]);
    out
}

fn is_selected(error: &str, includes: &[String], excludes: &[String]) -> bool {
    let error = error.to_ascii_lowercase();
    let included = includes.is_empty()
        || includes
            .iter()
            .any(|needle| error.contains(&needle.to_ascii_lowercase()));
    let excluded = excludes
        .iter()
        .any(|needle| error.contains(&needle.to_ascii_lowercase()));
    included && !excluded
}

/// The stream id is `<unix millis>-<sequence>`.
fn entry_time(id: &str) -> Option<DateTime<Utc>> {
    let millis = id.split('-').next()?.parse::<i64>().ok()?;
    DateTime::from_timestamp_millis(millis)
}

fn format_time(time: Option<DateTime<Utc>>) -> String {
    time.map_or_else(
        || "-".to_string(),
        |time| time.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    )
}

#[derive(Default)]
struct CategoryStat {
    count: usize,
    first: Option<DateTime<Utc>>,
    last: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct Stats {
    scanned: usize,
    malformed: usize,
    filtered_out: usize,
    selected: usize,
    republished: usize,
    deleted: usize,
    by_category: BTreeMap<Category, CategoryStat>,
}

fn record_category(
    stats: &mut BTreeMap<Category, CategoryStat>,
    category: Category,
    time: Option<DateTime<Utc>>,
) {
    let stat = stats.entry(category).or_default();
    stat.count += 1;
    if let Some(time) = time {
        stat.first.get_or_insert(time);
        stat.last = Some(time);
    }
}

fn spawn_signal_listener(shutdown: CancellationToken) {
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("interrupt received: finishing the current entry, then stopping");
            shutdown.cancel();
        }
    });
}

/// Sleeps for `pace_ms`, returning early (cleanly) when interrupted.
async fn pace(shutdown: &CancellationToken, pace_ms: u64) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => true,
        _ = tokio::time::sleep(Duration::from_millis(pace_ms)) => false,
    }
}

async fn read_page(
    connection: &mut MultiplexedConnection,
    start: &str,
) -> redis::RedisResult<Vec<(String, HashMap<String, String>)>> {
    let value: Value = redis::cmd("XRANGE")
        .arg(FAILED_TX_STREAM)
        .arg(start)
        .arg("+")
        .arg("COUNT")
        .arg(PAGE_SIZE)
        .query_async(connection)
        .await?;

    Ok(decode_entries(value))
}

fn decode_entries(value: Value) -> Vec<(String, HashMap<String, String>)> {
    let Value::Array(entries) = value else {
        return Vec::new();
    };

    let mut out = Vec::with_capacity(entries.len());

    for entry in entries {
        let Value::Array(mut parts) = entry else {
            continue;
        };
        if parts.len() != 2 {
            continue;
        }

        let fields = parts.pop().unwrap();
        let id = parts.pop().unwrap();

        let Some(id) = value_string(id) else {
            continue;
        };
        let Value::Array(fields) = fields else {
            continue;
        };

        let mut map = HashMap::with_capacity(fields.len() / 2);
        for pair in fields.chunks(2) {
            if pair.len() != 2 {
                continue;
            }
            let (Some(key), Some(value)) =
                (value_string(pair[0].clone()), value_string(pair[1].clone()))
            else {
                continue;
            };
            map.insert(key, value);
        }

        out.push((id, map));
    }

    out
}

fn value_string(value: Value) -> Option<String> {
    match value {
        Value::BulkString(bytes) => String::from_utf8(bytes).ok(),
        Value::SimpleString(text) => Some(text),
        _ => None,
    }
}

async fn republish(
    connection: &mut MultiplexedConnection,
    entry: &DeadLetter,
) -> redis::RedisResult<()> {
    let _: String = redis::cmd("XADD")
        .arg(BRIDGE_TX_STREAM)
        .arg("*")
        .arg("chain")
        .arg(&entry.chain)
        .arg("tx_hash")
        .arg(&entry.tx_hash)
        .arg("slot")
        .arg(entry.slot)
        .query_async(connection)
        .await?;

    Ok(())
}

async fn delete(connection: &mut MultiplexedConnection, id: &str) -> redis::RedisResult<i64> {
    redis::cmd("XDEL")
        .arg(FAILED_TX_STREAM)
        .arg(id)
        .query_async(connection)
        .await
}

async fn remaining(connection: &mut MultiplexedConnection) -> redis::RedisResult<i64> {
    redis::cmd("XLEN")
        .arg(FAILED_TX_STREAM)
        .query_async(connection)
        .await
}

fn print_summary(stats: &Stats, remaining: i64, args: &Args) {
    println!();
    println!(
        "summary: selected {}, re-published {}, deleted {}, skipped {}, remaining in dead-letter stream {}",
        stats.selected, stats.republished, stats.deleted, stats.malformed, remaining
    );
    if !args.error_contains.is_empty() || !args.exclude_error_contains.is_empty() {
        println!(
            "filters excluded {} of {} scanned entries",
            stats.filtered_out, stats.scanned
        );
    }
    if args.keep {
        println!("--keep: dead-letter entries left in place");
    }
}

async fn run(args: &Args) -> Result<()> {
    let url = redis_url()?;
    let client = redis::Client::open(url.as_str())?;
    let mut connection = client
        .get_multiplexed_async_connection()
        .await
        .map_err(|error| {
            anyhow!(
                "failed to connect to Redis: {}",
                sanitize(&error.to_string())
            )
        })?;

    let shutdown = CancellationToken::new();
    spawn_signal_listener(shutdown.clone());

    if !args.execute {
        println!("dry run: nothing will be written (pass --execute to re-publish)");
    }

    let mut stats = Stats::default();
    let mut start = "-".to_string();
    let mut interrupted = false;

    'pages: loop {
        let page = read_page(&mut connection, &start).await?;
        if page.is_empty() {
            break;
        }
        let page_len = page.len();

        for (id, fields) in &page {
            start = format!("({id}");
            stats.scanned += 1;

            let entry = match parse_entry(id, fields) {
                Ok(entry) => entry,
                Err(reason) => {
                    stats.malformed += 1;
                    println!("skipped malformed entry {id}: {reason}");
                    continue;
                }
            };

            if !is_selected(
                &entry.error,
                &args.error_contains,
                &args.exclude_error_contains,
            ) {
                stats.filtered_out += 1;
                continue;
            }

            if args.limit.is_some_and(|limit| stats.selected >= limit) {
                break 'pages;
            }

            stats.selected += 1;
            let category = Category::classify(&entry.error);
            record_category(&mut stats.by_category, category, entry_time(&entry.id));

            if args.execute {
                if let Err(error) = republish(&mut connection, &entry).await {
                    let remaining = remaining(&mut connection).await.unwrap_or(-1);
                    print_summary(&stats, remaining, args);
                    return Err(anyhow!(
                        "re-publish failed for entry {}: {}",
                        entry.id,
                        sanitize(&error.to_string())
                    ));
                }
                stats.republished += 1;

                interrupted = pace(&shutdown, args.pace_ms).await;

                if !args.keep {
                    match delete(&mut connection, &entry.id).await {
                        Ok(0) => {}
                        Ok(_) => stats.deleted += 1,
                        Err(error) => {
                            let remaining = remaining(&mut connection).await.unwrap_or(-1);
                            print_summary(&stats, remaining, args);
                            return Err(anyhow!(
                                "failed to delete entry {}: {}",
                                entry.id,
                                sanitize(&error.to_string())
                            ));
                        }
                    }
                }

                if args.verbose {
                    println!(
                        "re-published {} {}{}",
                        entry.id,
                        entry.tx_hash,
                        if args.keep { " (kept)" } else { "" }
                    );
                }

                if interrupted {
                    break 'pages;
                }
            } else if args.verbose {
                println!(
                    "  {} {} {:<22} chain={} slot={} tx={} error={}",
                    entry.id,
                    format_time(entry_time(&entry.id)),
                    category.label(),
                    entry.chain,
                    entry.slot,
                    entry.tx_hash,
                    sanitize(&entry.error)
                );
            }
        }

        if page_len < PAGE_SIZE {
            break;
        }
        if shutdown.is_cancelled() {
            interrupted = true;
            break;
        }
    }

    let remaining = remaining(&mut connection).await?;

    if !args.execute {
        println!(
            "scanned {} dead-letter entries (oldest first), skipped {} malformed",
            stats.scanned, stats.malformed
        );
        println!("entries by error category (selected, first/last from the entry id):");
        for category in Category::ALL {
            let stat = stats.by_category.get(&category);
            println!(
                "    {:<22} {:>6}  first {}  last {}",
                category.label(),
                stat.map_or(0, |stat| stat.count),
                format_time(stat.and_then(|stat| stat.first)),
                format_time(stat.and_then(|stat| stat.last))
            );
        }
    }

    if interrupted {
        println!("stopped on interrupt after the current entry");
    }

    print_summary(&stats, remaining, args);
    Ok(())
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    if let Err(error) = run(&args).await {
        eprintln!("error: {}", sanitize(&format!("{error:#}")));
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn parses_a_dead_letter_entry() {
        let entry = parse_entry(
            "1700000000000-0",
            &fields(&[
                ("message_id", "1700000000000-0"),
                ("chain", "solana"),
                ("tx_hash", "5abc"),
                ("slot", "250000000"),
                ("error", "VAA not available for transaction: 5abc"),
            ]),
        )
        .expect("valid entry");

        assert_eq!(
            entry,
            DeadLetter {
                id: "1700000000000-0".to_string(),
                chain: "solana".to_string(),
                tx_hash: "5abc".to_string(),
                slot: 250_000_000,
                error: "VAA not available for transaction: 5abc".to_string(),
            }
        );
    }

    #[test]
    fn skips_malformed_entries() {
        let valid = fields(&[
            ("chain", "solana"),
            ("tx_hash", "5abc"),
            ("slot", "250000000"),
            ("error", "transaction not found: 5abc"),
        ]);

        for missing in ["chain", "tx_hash", "slot", "error"] {
            let mut broken = valid.clone();
            broken.remove(missing);
            assert_eq!(
                parse_entry("1-0", &broken),
                Err(match missing {
                    "chain" => "missing chain",
                    "tx_hash" => "missing tx_hash",
                    "slot" => "missing slot",
                    _ => "missing error",
                })
            );
        }

        let mut broken_slot = valid.clone();
        broken_slot.insert("slot".to_string(), "not-a-number".to_string());
        assert_eq!(parse_entry("1-0", &broken_slot), Err("unparsable slot"));
    }

    #[test]
    fn categorises_errors() {
        let cases = [
            (
                "service overloaded, retry shortly",
                Category::ServiceOverloaded,
            ),
            (
                "upstream provider error (helius): Transaction version (unsupported) -32015",
                Category::TxVersion32015,
            ),
            (
                "upstream provider error (helius): builder error: missing field `params`",
                Category::BuilderError,
            ),
            (
                "VAA not available for transaction: 5abc",
                Category::VaaNotAvailable,
            ),
            ("transaction not found: 5abc", Category::TransactionNotFound),
            (
                "upstream provider rate limited: unknown",
                Category::RateLimited,
            ),
            (
                "failed to normalise transaction data: boom",
                Category::Other,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(Category::classify(error), expected, "{error}");
        }

        assert_eq!(
            Category::classify("VAA NOT AVAILABLE"),
            Category::VaaNotAvailable,
            "matching is case-insensitive"
        );
    }

    #[test]
    fn redacts_api_keys() {
        assert_eq!(
            sanitize("fetch failed: https://x.example/?api-key=SECRET&limit=5"),
            "fetch failed: https://x.example/?api-key=[redacted]&limit=5"
        );
        assert_eq!(sanitize("api-key=SECRET"), "api-key=[redacted]");
        assert_eq!(sanitize("API-KEY=SECRET"), "API-KEY=[redacted]");
        assert_eq!(
            sanitize("two api-key=a api-key=b"),
            "two api-key=[redacted] api-key=[redacted]"
        );
        assert_eq!(sanitize("no secrets here"), "no secrets here");
    }

    #[test]
    fn filters_include_and_exclude_together() {
        let includes = vec!["vaa".to_string(), "overloaded".to_string()];
        let excludes = vec!["api-key".to_string()];

        assert!(is_selected(
            "VAA not available for transaction: x",
            &includes,
            &excludes
        ));
        assert!(is_selected(
            "service overloaded, retry shortly",
            &includes,
            &excludes
        ));
        assert!(!is_selected(
            "transaction not found: x",
            &includes,
            &excludes
        ));
        assert!(!is_selected(
            "VAA not available: fetch https://x.example/?api-key=SECRET failed",
            &includes,
            &excludes
        ));
        assert!(is_selected("transaction not found: x", &[], &excludes));
        assert!(!is_selected("call failed api-key=SECRET", &[], &excludes));
    }

    #[test]
    fn reads_entry_time_from_the_id() {
        assert_eq!(
            entry_time("1700000000123-0").map(|time| time.timestamp_millis()),
            Some(1_700_000_000_123)
        );
        assert!(entry_time("not-an-id").is_none());
    }
}
