//! Measures the WebSocket ingester's recall and the classifier's precision per
//! subscribed program over a time window. Read-only: it never writes to Postgres
//! or Redis.
//!
//!   cargo run --bin reconcile -- --window-hours 2 --precision-sample 20
//!
//! Ground truth is `getSignaturesForAddress`. Signatures already in
//! `bridge_transfers` are subtracted before any `getTransaction`, so the run only
//! pays for what is actually missing. The remainder goes through the pipeline's
//! own fetch/preload path and the adapter classifier, offline.
//!
//! It has its own Helius lane (`HELIUS_RECONCILE_URL`, falling back to
//! `HELIUS_URL`, at `RECONCILE_RPS` per second) so it cannot starve the live
//! pipeline. Exits 1 when recall is below `--min-recall` and the `--max-fetch` cap
//! was not hit; when the cap was hit, recall is printed as a lower bound.

use bridge::chain_adapters::setup::build_registry;
use bridge::chains::ChainRegistry;
use bridge::clients::helius;
use bridge::clients::limiter::call_upstream;
use bridge::config::AppConfig;
use bridge::db;
use bridge::db::repository;
use bridge::protocols::{Classification, Protocol, SolanaTx};
use bridge::services::analyzer::load_solana_tx;
use bridge::services::precision::{PrecisionReport, precision_pass};
use bridge::services::reconcile::{
    ClassificationTally, Diff, FetchOutcome, LagStats, MissCluster, PersistedTransaction, Recall,
    Signature, SignatureScan, Window, cluster, diff, lag_stats, recall, sample_evenly,
};
use bridge::state::AppState;
use reqwest::Client;
use rustls::crypto::ring;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: reconcile [--window-hours H] [--end-offset-secs S] [--gap-secs S]
                   [--precision-sample N] [--program ID] [--json]
                   [--max-fetch N] [--min-recall PCT]

  --window-hours     window to reconcile, ending --end-offset-secs ago (default 2)
  --end-offset-secs  how long ago the window ends, so the ingester could have seen
                     it (default 600)
  --gap-secs         a miss with no persisted transfer this close on either side is
                     a gap (default 300)
  --precision-sample persisted transactions to re-check against WormholeScan
                     (default 0, off)
  --program          program ID to reconcile (default: every registered program)
  --json             machine-readable output on stdout
  --max-fetch        hard cap on signatures fetched with getTransaction; counts
                     signatures, not Helius calls, so retries are not counted
                     separately (default 2000)
  --min-recall       exit 1 below this percentage, unless the cap was hit
                     (default 99.9)

Reads HELIUS_RECONCILE_URL (falls back to HELIUS_URL) at RECONCILE_RPS (default 3).";

struct Args {
    window_hours: f64,
    end_offset_secs: i64,
    gap_secs: i64,
    precision_sample: usize,
    program: Option<String>,
    json: bool,
    max_fetch: usize,
    min_recall: f64,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        window_hours: 2.0,
        end_offset_secs: 600,
        gap_secs: 300,
        precision_sample: 0,
        program: None,
        json: false,
        max_fetch: 2000,
        min_recall: 99.9,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--help" || flag == "-h" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        if flag == "--json" {
            args.json = true;
            continue;
        }
        let mut value = || {
            it.next()
                .ok_or_else(|| anyhow::anyhow!("{flag} needs a value\n\n{USAGE}"))
        };
        match flag.as_str() {
            "--window-hours" => args.window_hours = value()?.parse()?,
            "--end-offset-secs" => args.end_offset_secs = value()?.parse()?,
            "--gap-secs" => args.gap_secs = value()?.parse()?,
            "--precision-sample" => args.precision_sample = value()?.parse()?,
            "--program" => args.program = Some(value()?),
            "--max-fetch" => args.max_fetch = value()?.parse()?,
            "--min-recall" => args.min_recall = value()?.parse()?,
            other => anyhow::bail!("unknown argument {other}\n\n{USAGE}"),
        }
    }
    anyhow::ensure!(args.window_hours > 0.0, "--window-hours must be above 0");
    anyhow::ensure!(
        args.end_offset_secs >= 0,
        "--end-offset-secs must not be negative"
    );
    anyhow::ensure!(args.gap_secs >= 0, "--gap-secs must not be negative");
    anyhow::ensure!(args.max_fetch > 0, "--max-fetch must be above 0");
    anyhow::ensure!(
        (0.0..=100.0).contains(&args.min_recall),
        "--min-recall must be between 0 and 100"
    );
    Ok(args)
}

/// A fetched transaction, shared between programs so a signature that touches two
/// of them costs one `getTransaction`.
#[derive(Clone)]
enum Fetched {
    View(SolanaTx),
    Unreadable,
    Failed(String),
}

struct ProgramReport {
    program: String,
    protocols: Vec<Protocol>,
    scan: SignatureScan,
    diff: Diff,
    tally: ClassificationTally,
    recall: Recall,
    lag: Option<LagStats>,
    misses: Vec<MissCluster>,
    precision: Option<PrecisionReport>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");
    // Keep stdout clean for --json; progress and logs go to stderr.
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();

    let args = parse_args()?;

    let mut config = AppConfig::from_env()?;
    // The reconcile tool's own Helius lane: its endpoint and quota, never the
    // live pipeline's limiter.
    config.helius_url = config.helius_reconcile_url.clone();
    config.helius_api_rps = config.reconcile_rps;

    let db = db::connection::connect(&config.database_url).await?;
    let http_client = Client::builder().timeout(Duration::from_secs(15)).build()?;
    let registry = build_registry(&config, &http_client, config.reconcile_rps);
    let chains = ChainRegistry::load(&db).await?;
    let state = AppState::new(db, config, http_client, registry, chains)?;

    let registered = state.protocols.program_ids();
    let programs = match &args.program {
        Some(program) => {
            anyhow::ensure!(
                registered.iter().any(|id| id == program),
                "program {program} is not in the registry (registered: {})",
                registered.join(", ")
            );
            vec![program.clone()]
        }
        None => registered,
    };
    anyhow::ensure!(
        !programs.is_empty(),
        "the protocol registry has no program IDs to reconcile"
    );

    let now = chrono::Utc::now().timestamp();
    let window_secs = (args.window_hours * 3600.0).round() as i64;
    anyhow::ensure!(window_secs > 0, "--window-hours rounds to an empty window");
    let window = Window::plan(now, window_secs, args.end_offset_secs);
    eprintln!(
        "reconciling {} .. {} for {} program(s)",
        timestamp(window.start),
        timestamp(window.end),
        programs.len()
    );

    let started = Instant::now();
    // One budget for every getTransaction in the run: miss classification first,
    // then the optional precision sample.
    let mut fetched = 0usize;
    let mut cache: HashMap<String, Fetched> = HashMap::new();
    let mut reports = Vec::with_capacity(programs.len());

    for program in &programs {
        eprintln!("scanning {program} ...");
        let scan = scan_signatures(&state, program, window).await?;
        let protocols = state.protocols.protocols_for(program);
        let persisted = persisted_rows(&state, &protocols, window).await?;
        let diff = diff(&scan.signatures, &persisted);

        let mut tally = ClassificationTally::default();
        let mut misses = Vec::new();
        for miss in &diff.missing {
            let outcome = missing_outcome(
                &state,
                &protocols,
                miss,
                &mut cache,
                &mut fetched,
                args.max_fetch,
            )
            .await;
            tally.record(&outcome);
            if matches!(outcome, FetchOutcome::Transfers(_)) {
                misses.push(cluster(miss, &diff.persisted, args.gap_secs));
            }
        }

        let recall = recall(
            diff.persisted_transactions() as u64,
            tally.missed,
            tally.unfetched,
        );
        let lag = lag_stats(&diff.persisted);

        let precision = if args.precision_sample > 0 && !diff.persisted.is_empty() {
            let mut seen = BTreeSet::new();
            let hashes: Vec<&str> = diff
                .persisted
                .iter()
                .filter(|row| seen.insert(row.observed_tx_hash.as_str()))
                .map(|row| row.observed_tx_hash.as_str())
                .collect();
            let remaining = args.max_fetch.saturating_sub(fetched);
            if remaining == 0 {
                eprintln!("no getTransaction budget left for the precision pass");
                None
            } else {
                let n = args.precision_sample.min(remaining).min(hashes.len());
                let sample: Vec<&str> = sample_evenly(&hashes, n).into_iter().copied().collect();
                eprintln!("precision pass over {} transactions ...", sample.len());
                // One getTransaction per sampled signature, charged to the same
                // budget before the pass runs.
                fetched += sample.len();
                Some(precision_pass(&state, &sample).await)
            }
        } else {
            None
        };

        reports.push(ProgramReport {
            program: program.clone(),
            protocols,
            scan,
            diff,
            tally,
            recall,
            lag,
            misses,
            precision,
        });
    }

    let elapsed = started.elapsed();
    if args.json {
        let body = json!({
            "window": {
                "start": window.start,
                "end": window.end,
                "window_hours": args.window_hours,
                "end_offset_secs": args.end_offset_secs,
            },
            "min_recall": args.min_recall,
            "max_fetch": args.max_fetch,
            "get_transactions": fetched,
            "cap_hit": reports.iter().any(|report| report.recall.capped),
            "elapsed_secs": elapsed.as_secs_f64(),
            "programs": reports.iter().map(report_json).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        print_header(&args, window, elapsed);
        for report in &reports {
            print_report(report, &args);
        }
    }

    if let Some(report) = reports
        .iter()
        .filter(|report| !report.recall.capped)
        .find(|report| report.recall.percent.is_some_and(|p| p < args.min_recall))
    {
        let percent = report.recall.percent.unwrap_or(0.0);
        eprintln!(
            "recall {percent:.2}% on {} is below the required {:.2}%",
            report.program, args.min_recall
        );
        std::process::exit(1);
    }

    Ok(())
}

/// Pages `getSignaturesForAddress` from newest to oldest until the window's start.
async fn scan_signatures(
    state: &AppState,
    program: &str,
    window: Window,
) -> anyhow::Result<SignatureScan> {
    let mut scan = SignatureScan::default();
    let mut before: Option<String> = None;

    loop {
        let page = call_upstream(&state.helius_limiter, || {
            helius::get_signatures_for_address(
                &state.http_client,
                &state.config.helius_url,
                program,
                before.clone(),
            )
        })
        .await?;

        if page.is_empty() {
            scan.push_page(&[], window);
            break;
        }

        let last = page
            .last()
            .and_then(|entry| entry["signature"].as_str())
            .map(str::to_string);
        let done = scan.push_page(&page, window);
        if done {
            break;
        }

        // `before` is exclusive. A repeated cursor would loop forever.
        match last {
            Some(last) if before.as_deref() != Some(last.as_str()) => before = Some(last),
            _ => break,
        }
    }

    Ok(scan)
}

async fn persisted_rows(
    state: &AppState,
    protocols: &[Protocol],
    window: Window,
) -> anyhow::Result<Vec<PersistedTransaction>> {
    let mut rows = Vec::new();
    for protocol in protocols {
        let found = repository::persisted_transfers_in_window(
            &state.db,
            protocol.as_str(),
            window.start,
            window.end,
        )
        .await?;
        rows.extend(found.into_iter().map(|row| PersistedTransaction {
            observed_tx_hash: row.observed_tx_hash,
            slot: row.slot,
            block_time: row.block_time,
            stored_at: row.stored_at,
        }));
    }
    Ok(rows)
}

async fn missing_outcome(
    state: &AppState,
    protocols: &[Protocol],
    miss: &Signature,
    cache: &mut HashMap<String, Fetched>,
    fetched: &mut usize,
    max_fetch: usize,
) -> FetchOutcome {
    if let Some(cached) = cache.get(&miss.signature) {
        return classify(state, protocols, cached);
    }
    if *fetched >= max_fetch {
        return FetchOutcome::Unfetched;
    }

    *fetched += 1;
    let result = match load_solana_tx(state, &miss.signature).await {
        Ok(Some(view)) => Fetched::View(view),
        Ok(None) => Fetched::Unreadable,
        Err(error) => Fetched::Failed(error.code().to_string()),
    };
    let outcome = classify(state, protocols, &result);
    cache.insert(miss.signature.clone(), result);
    outcome
}

fn classify(state: &AppState, protocols: &[Protocol], fetched: &Fetched) -> FetchOutcome {
    let view = match fetched {
        Fetched::View(view) => view,
        Fetched::Unreadable => {
            return FetchOutcome::FetchFailed {
                kind: "UNREADABLE_TX".to_string(),
            };
        }
        Fetched::Failed(kind) => {
            return FetchOutcome::FetchFailed { kind: kind.clone() };
        }
    };

    let mut transfers = 0usize;
    let mut rejection: Option<(&'static str, &'static str)> = None;
    for (protocol, classification) in state.protocols.classify(view) {
        if !protocols.contains(&protocol) {
            continue;
        }
        match classification {
            Classification::Transfers(found) => transfers += found.len(),
            Classification::Rejected(reject) => {
                rejection.get_or_insert((reject.kind(), reject.reason()));
            }
        }
    }

    if transfers > 0 {
        FetchOutcome::Transfers(transfers)
    } else if let Some((kind, reason)) = rejection {
        FetchOutcome::Rejected { kind, reason }
    } else {
        FetchOutcome::Rejected {
            kind: "not_transfer",
            reason: "no_adapter",
        }
    }
}

fn timestamp(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0).map_or_else(
        || secs.to_string(),
        |time| time.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    )
}

fn secs(value: Option<i64>) -> String {
    value.map_or_else(|| "-".to_string(), |value| format!("{value}s"))
}

fn print_header(args: &Args, window: Window, elapsed: Duration) {
    println!(
        "reconcile window {} .. {} ({:.2}h, ending {}s ago), ran in {:.1}s",
        timestamp(window.start),
        timestamp(window.end),
        args.window_hours,
        args.end_offset_secs,
        elapsed.as_secs_f64()
    );
}

fn print_report(report: &ProgramReport, args: &Args) {
    let protocols: Vec<&str> = report
        .protocols
        .iter()
        .map(|protocol| protocol.as_str())
        .collect();
    println!("\nprogram {} ({})", report.program, protocols.join(", "));
    println!(
        "  signatures: {} successful in window ({} failed, {} too recent, {} unplaceable)",
        report.scan.signatures.len(),
        report.scan.failed,
        report.scan.too_recent,
        report.scan.unplaceable
    );
    println!(
        "  scan stop marker: {} (one signature older than the window ended the scan; not a count)",
        if report.scan.scan_stop_marker {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "  persisted: {} transfer(s) in {} transaction(s)",
        report.diff.persisted.len(),
        report.recall.persisted
    );
    match report.recall.missed_pct() {
        Some(pct) => println!("  missed: {} ({pct:.2}%)", report.recall.missed),
        None => println!("  missed: {}", report.recall.missed),
    }
    if let Some(pct) = report.recall.percent {
        println!("  recall: {pct:.2}%");
    } else if let Some(lower) = report.recall.lower_bound {
        println!(
            "  recall: >= {lower:.2}% (cap hit: {} signature(s) fetchable within --max-fetch {})",
            report.recall.unfetched, args.max_fetch
        );
    } else {
        println!("  recall: n/a (no transfers in window)");
    }

    let rejected = report.tally.rejection_total();
    println!("  rejected: {rejected}");
    for (reason, count) in &report.tally.rejected {
        println!("      {reason:<44} {count}");
    }
    let failed = report.tally.fetch_failed_total();
    println!("  fetch failed: {failed}");
    for (kind, count) in &report.tally.fetch_failed {
        println!("      {kind:<44} {count}");
    }

    match report.lag {
        Some(lag) => println!(
            "  ingestion lag: p50 {}s, p99 {}s, max {}s (n={})",
            lag.p50_secs, lag.p99_secs, lag.max_secs, lag.samples
        ),
        None => println!("  ingestion lag: n/a"),
    }

    if !report.misses.is_empty() {
        println!("  misses:");
        for miss in &report.misses {
            println!(
                "    {:<10} slot {}  {}  (nearest persisted {} before, {} after)",
                miss.suspect.as_str(),
                miss.signature.slot,
                miss.signature.signature,
                secs(miss.nearest_before_secs),
                secs(miss.nearest_after_secs)
            );
        }
    }

    if let Some(precision) = &report.precision {
        println!(
            "  precision sample: {} persisted transaction(s), {} compared",
            precision.sampled, precision.summary.settled.total
        );
        match precision.summary.settled.agreement_pct() {
            Some(pct) => println!(
                "    agreement: {pct:.1}% ({}/{})",
                precision.summary.settled.agree, precision.summary.settled.total
            ),
            None => println!("    agreement: n/a (nothing compared)"),
        }
        for (verdict, count) in &precision.summary.settled.by_verdict {
            println!("      {verdict:<18} {count}");
        }
        if precision.summary.no_transfer > 0 {
            println!(
                "    no transfer on re-analysis: {}",
                precision.summary.no_transfer
            );
        }
        for (kind, count) in &precision.summary.errors {
            println!("    error {kind:<28} {count}");
        }
    }
}

fn report_json(report: &ProgramReport) -> Value {
    json!({
        "program": report.program,
        "protocols": report
            .protocols
            .iter()
            .map(|protocol| protocol.as_str())
            .collect::<Vec<_>>(),
        "signatures": report.scan.signatures.len(),
        "failed": report.scan.failed,
        "too_recent": report.scan.too_recent,
        "scan_stop_marker": report.scan.scan_stop_marker,
        "unplaceable": report.scan.unplaceable,
        "persisted": report.diff.persisted.len(),
        "persisted_transactions": report.recall.persisted,
        "missed": report.recall.missed,
        "missed_percent": report.recall.missed_pct(),
        "rejected": report.tally.rejected,
        "rejected_total": report.tally.rejection_total(),
        "fetch_failed": report.tally.fetch_failed,
        "fetch_failed_total": report.tally.fetch_failed_total(),
        "unfetched": report.recall.unfetched,
        "recall_percent": report.recall.percent,
        "recall_lower_bound": report.recall.lower_bound,
        "capped": report.recall.capped,
        "lag": report.lag.map(|lag| json!({
            "samples": lag.samples,
            "p50_secs": lag.p50_secs,
            "p99_secs": lag.p99_secs,
            "max_secs": lag.max_secs,
        })),
        "misses": report
            .misses
            .iter()
            .map(|miss| json!({
                "signature": miss.signature.signature,
                "slot": miss.signature.slot,
                "block_time": miss.signature.block_time,
                "suspect": miss.suspect.as_str(),
                "nearest_before_secs": miss.nearest_before_secs,
                "nearest_after_secs": miss.nearest_after_secs,
            }))
            .collect::<Vec<_>>(),
        "precision": report.precision.as_ref().map(|precision| json!({
            "sampled": precision.sampled,
            "compared": precision.summary.settled.total,
            "agreement_percent": precision.summary.settled.agreement_pct(),
            "by_verdict": precision.summary.settled.by_verdict,
            "no_transfer": precision.summary.no_transfer,
            "errors": precision.summary.errors,
        })),
    })
}
