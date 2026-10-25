//! Replays the last N signatures of the Token Bridge program through the real
//! pipeline (`analyse_tx`) and compares our status with WormholeScan's `global-tx`.
//!
//!   cargo run --bin replay -- --limit 200
//!
//! Needs the same env as the server. It reads the chain registry from
//! `DATABASE_URL` and writes nothing: no migrations, no persisted transfers.
//! Exits 1 when agreement on settled transfers is below `--min-agreement`,
//! and 2 when there was nothing settled to compare.

use bridge::chain_adapters::setup::build_registry;
use bridge::chains::ChainRegistry;
use bridge::clients::limiter::call_upstream;
use bridge::clients::{helius, wormhole};
use bridge::config::AppConfig;
use bridge::db;
use bridge::services::analyzer::analyse_tx;
use bridge::services::replay::{Outcome, Summary, Verdict, compare};
use bridge::state::AppState;
use futures_util::{StreamExt, stream};
use reqwest::Client;
use rustls::crypto::ring;
use std::time::{Duration, Instant};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: replay [--limit N] [--min-age-secs S] [--min-agreement PCT]
              [--concurrency N] [--before SIGNATURE] [--program ID]

  --limit          signatures to replay, newest first (default 100)
  --min-age-secs   a transfer younger than this is reported but does not count
                   towards the agreement (default 900, i.e. 15 minutes)
  --min-agreement  exit 1 when agreement on settled transfers is below this
                   percentage (default 99)
  --concurrency    signatures analysed at once (default 4; the Helius limiter
                   still caps the real rate)
  --before         start from the signatures older than this one
  --program        program to list signatures for (default SOLANA_TOKEN_BRIDGE_PROGRAM)";

struct Args {
    limit: usize,
    min_age_secs: i64,
    min_agreement: f64,
    concurrency: usize,
    before: Option<String>,
    program: Option<String>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        limit: 100,
        min_age_secs: 900,
        min_agreement: 99.0,
        concurrency: 4,
        before: None,
        program: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--help" || flag == "-h" {
            println!("{USAGE}");
            std::process::exit(0);
        }
        let mut value = || {
            it.next()
                .ok_or_else(|| anyhow::anyhow!("{flag} needs a value\n\n{USAGE}"))
        };
        match flag.as_str() {
            "--limit" => args.limit = value()?.parse()?,
            "--min-age-secs" => args.min_age_secs = value()?.parse()?,
            "--min-agreement" => args.min_agreement = value()?.parse()?,
            "--concurrency" => args.concurrency = value()?.parse::<usize>()?.max(1),
            "--before" => args.before = Some(value()?),
            "--program" => args.program = Some(value()?),
            other => anyhow::bail!("unknown argument {other}\n\n{USAGE}"),
        }
    }
    anyhow::ensure!(args.limit > 0, "--limit must be above 0");
    Ok(args)
}

struct Sig {
    signature: String,
    block_time: Option<i64>,
}

/// The newest `limit` successful signatures of `program`, in pages of 1000.
async fn collect_signatures(
    state: &AppState,
    program: &str,
    limit: usize,
    mut before: Option<String>,
) -> anyhow::Result<Vec<Sig>> {
    let mut out: Vec<Sig> = Vec::with_capacity(limit);

    'pages: loop {
        let page = call_upstream(&state.helius_limiter, || {
            helius::get_signatures_for_address(
                &state.http_client,
                &state.config.helius_url,
                program,
                before.clone(),
            )
        })
        .await?;

        let Some(last) = page.last().and_then(|e| e["signature"].as_str()) else {
            break;
        };
        before = Some(last.to_string());

        for entry in &page {
            // a failed tx reverts its effects: there is nothing to compare
            if entry.get("err").is_some_and(|e| !e.is_null()) {
                continue;
            }
            let Some(signature) = entry["signature"].as_str() else {
                continue;
            };
            out.push(Sig {
                signature: signature.to_string(),
                block_time: entry["blockTime"].as_i64(),
            });
            if out.len() >= limit {
                break 'pages;
            }
        }
    }
    Ok(out)
}

struct Report {
    signature: String,
    outcome: Outcome,
    latency_ms: u64,
    /// One line for anything that is not an agreement.
    detail: Option<String>,
}

async fn replay_one(state: &AppState, sig: &Sig, min_age_secs: i64, now: i64) -> Report {
    let started = Instant::now();
    let result = analyse_tx(state, "solana", &sig.signature).await;
    let latency_ms = started.elapsed().as_millis() as u64;

    let report = |outcome: Outcome, detail: Option<String>| Report {
        signature: sig.signature.clone(),
        outcome,
        latency_ms,
        detail,
    };

    let tx = match result {
        Ok(analysed) => analysed.tx,
        Err(e) => {
            let kind = e.code().to_string();
            return report(Outcome::Error(kind.clone()), Some(format!("{kind}: {e}")));
        }
    };
    let Some(transfer) = tx.bridge_transfer else {
        return report(Outcome::NoTransfer, None);
    };

    let theirs = call_upstream(&state.wormhole_limiter, || {
        wormhole::get_destination_tx(
            &state.http_client,
            &state.config.wormhole_url,
            &transfer.message_id,
        )
    })
    .await;
    let theirs = match theirs {
        Ok(theirs) => theirs,
        Err(e) => {
            let kind = format!("reference:{}", e.code());
            return report(Outcome::Error(kind.clone()), Some(format!("{kind}: {e}")));
        }
    };

    let verdict = compare(
        &transfer.status,
        transfer.destination_tx_hash.as_deref(),
        theirs.as_deref(),
    );
    let settled = sig.block_time.is_some_and(|t| now - t >= min_age_secs);
    let detail = (!verdict.agrees()).then(|| {
        format!(
            "{:<13} ours={:?} dest={} theirs={}",
            verdict.name(),
            transfer.status,
            transfer.destination_tx_hash.as_deref().unwrap_or("-"),
            theirs.as_deref().unwrap_or("-"),
        )
    });
    report(
        Outcome::Transfer {
            status: transfer.status,
            verdict,
            settled,
        },
        detail,
    )
}

fn pct(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |v| format!("{v:.1}%"))
}

fn print_tally(label: &str, tally: &bridge::services::replay::Tally) {
    println!(
        "{label}: {} transfers, agreement {} ({}/{})",
        tally.total,
        pct(tally.agreement_pct()),
        tally.agree,
        tally.total
    );
    for (verdict, n) in &tally.by_verdict {
        println!("    {verdict:<14} {n}");
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();

    let args = parse_args()?;
    let config = AppConfig::from_env()?;
    let program = args
        .program
        .clone()
        .unwrap_or_else(|| config.solana_token_bridge_program.clone());

    let db = db::connection::connect(&config.database_url).await?;
    let http_client = Client::builder().timeout(Duration::from_secs(15)).build()?;
    let registry = build_registry(&config, &http_client);
    let chains = ChainRegistry::load(&db).await?;
    let state = AppState::new(db, config, http_client, registry, chains)?;

    eprintln!(
        "listing the last {} signatures of {program} ...",
        args.limit
    );
    let sigs = collect_signatures(&state, &program, args.limit, args.before.clone()).await?;
    eprintln!("replaying {} signatures ...", sigs.len());

    let now = chrono::Utc::now().timestamp();
    let reports: Vec<Report> = stream::iter(&sigs)
        .map(|sig| replay_one(&state, sig, args.min_age_secs, now))
        .buffer_unordered(args.concurrency)
        .collect()
        .await;

    let mut summary = Summary::default();
    let mut problems: Vec<&Report> = Vec::new();
    for report in &reports {
        summary.record(&report.outcome, report.latency_ms);
        if report.detail.is_some() {
            problems.push(report);
        }
    }

    println!(
        "replayed {} signatures of {program} (failed txs skipped), settled = older than {}s",
        summary.signatures, args.min_age_secs
    );
    print_tally("settled", &summary.settled);
    print_tally("recent ", &summary.recent);
    println!(
        "status mix: detected {}, pending {} ({}), completed {}",
        summary.detected,
        summary.pending,
        pct(summary.pending_pct()),
        summary.completed
    );
    println!("no transfer: {}", summary.no_transfer);
    if summary.errors.is_empty() {
        println!("errors: none");
    } else {
        println!("errors by kind:");
        for (kind, n) in &summary.errors {
            println!("    {kind:<28} {n}");
        }
    }
    println!(
        "analyse_tx latency: p50 {} ms, p99 {} ms",
        summary.latency_percentile(50.0).unwrap_or(0),
        summary.latency_percentile(99.0).unwrap_or(0)
    );

    if !problems.is_empty() {
        println!("\nnot agreeing or failed:");
        for report in problems {
            println!(
                "  {}  {}",
                report.signature,
                report.detail.as_deref().unwrap_or("")
            );
        }
    }

    // `Behind` is the criterion's hard rule, so call it out on its own.
    let behind = summary.settled.by_verdict.get(Verdict::Behind.name());
    if let Some(n) = behind {
        println!(
            "\n{n} settled transfer(s) are Pending although WormholeScan has the destination tx"
        );
    }

    match summary.settled.agreement_pct() {
        None => {
            eprintln!("no settled transfers to compare");
            std::process::exit(2);
        }
        Some(p) if p < args.min_agreement => {
            eprintln!(
                "agreement {p:.1}% is below the required {:.1}%",
                args.min_agreement
            );
            std::process::exit(1);
        }
        Some(_) => Ok(()),
    }
}
