use crate::domain::bridge_transfer::BridgeStatus;
use dashmap::DashMap;
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};

use crate::db::repository::{get_cached_analysis, persist_transaction};
use crate::domain::transaction::NormalisedTransaction;
use crate::error::AppError;
use crate::services::analyzer::analyse_tx;
use crate::state::AppState;

/// A failure worth remembering briefly, so repeated requests for the same
/// hash don't each redo the full upstream attempt (and queue on the lock).
#[derive(Clone)]
enum Neg {
    NotFound,
    VaaNotAvailable,
    RateLimited,
    Timeout,
    Provider,
}

impl Neg {
    fn from_error(e: &AppError) -> Option<Self> {
        match e {
            AppError::TransactionNotFound(_) => Some(Neg::NotFound),
            AppError::VaaNotAvailable(_) => Some(Neg::VaaNotAvailable),
            AppError::UpstreamRateLimited(_) => Some(Neg::RateLimited),
            AppError::UpstreamTimeout(_) => Some(Neg::Timeout),
            AppError::UpstreamProvider { .. } => Some(Neg::Provider),
            // Overloaded is local backpressure, bad input is never reached,
            // Normalisation/Internal may be bugs: don't cache those.
            _ => None,
        }
    }

    fn into_error(self, hash: &str) -> AppError {
        match self {
            Neg::NotFound => AppError::TransactionNotFound(hash.to_string()),
            Neg::VaaNotAvailable => AppError::VaaNotAvailable(hash.to_string()),
            Neg::RateLimited => AppError::UpstreamRateLimited("cached"),
            Neg::Timeout => AppError::UpstreamTimeout("cached"),
            Neg::Provider => AppError::UpstreamProvider {
                provider: "cached",
                message: "recent upstream failure".to_string(),
            },
        }
    }
}

pub struct ReadThrough {
    inflight: DashMap<String, Arc<Mutex<()>>>,
    upstream_permits: Semaphore,
    completed: Cache<String, NormalisedTransaction>,
    open: Cache<String, NormalisedTransaction>,
    // negative caches
    not_found: Cache<String, Neg>, // tx doesn't exist: longer TTL
    failed: Cache<String, Neg>,    // transient upstream failure: very short TTL
}

impl ReadThrough {
    pub fn new(max_concurrent_upstream: usize) -> Self {
        Self {
            inflight: DashMap::new(),
            upstream_permits: Semaphore::new(max_concurrent_upstream),
            completed: Cache::builder()
                .max_capacity(50_000)
                .time_to_live(Duration::from_secs(600))
                .build(),
            open: Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(30))
                .build(),
            // lower this (e.g. 5-10s) if freshly sent txs must show up quickly
            not_found: Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(30))
                .build(),
            failed: Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(2))
                .build(),
        }
    }
}

async fn lookup(state: &AppState, hash: &str) -> Option<NormalisedTransaction> {
    match get_cached_analysis(&state.db, hash).await {
        Ok(Some(value)) => serde_json::from_value(value)
            .map_err(|e| tracing::warn!(error = %e, "cached analysis has stale shape, ignoring"))
            .ok(),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, "analysis lookup failed");
            None
        }
    }
}

async fn remember(rt: &ReadThrough, hash: &str, tx: &NormalisedTransaction) {
    let done = matches!(
        tx.bridge_transfer.as_ref().map(|b| &b.status),
        Some(BridgeStatus::Completed)
    );
    let cache = if done { &rt.completed } else { &rt.open };
    cache.insert(hash.to_string(), tx.clone()).await;
}

/// Memory-only check: positive caches first, then negative caches.
async fn check_memory(
    rt: &ReadThrough,
    hash: &str,
) -> Option<Result<NormalisedTransaction, AppError>> {
    if let Some(hit) = rt.completed.get(hash).await {
        return Some(Ok(hit));
    }
    if let Some(hit) = rt.open.get(hash).await {
        return Some(Ok(hit));
    }
    if let Some(n) = rt.not_found.get(hash).await {
        return Some(Err(n.into_error(hash)));
    }
    if let Some(n) = rt.failed.get(hash).await {
        return Some(Err(n.into_error(hash)));
    }
    None
}

fn is_valid_solana_signature(s: &str) -> bool {
    bs58::decode(s)
        .into_vec()
        .map(|v| v.len() == 64)
        .unwrap_or(false)
}

pub async fn analyse_cached(
    state: &AppState,
    chain: &str,
    hash: &str,
) -> Result<NormalisedTransaction, AppError> {
    if chain == "solana" && !is_valid_solana_signature(hash) {
        return Err(AppError::InvalidTransactionHash(hash.to_string()));
    }

    let rt = &state.read_through;

    // 1: fast path, memory only (hits and remembered failures)
    if let Some(r) = check_memory(rt, hash).await {
        return r;
    }

    // 2: single-flight per key. Everything below (DB lookup + upstream)
    //    runs once per key; waiters re-check memory when they get the lock.
    let key = format!("{chain}:{hash}");
    let lock = rt
        .inflight
        .entry(key.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;

    let result = async {
        // someone ahead of us may have filled a cache (or recorded a failure)
        if let Some(r) = check_memory(rt, hash).await {
            return r;
        }

        // DB hit: fill memory
        if let Some(hit) = lookup(state, hash).await {
            remember(rt, hash, &hit).await;
            return Ok(hit);
        }

        // upstream, bounded wait for a permit so overload fails fast
        let fetched = async {
            let _permit =
                tokio::time::timeout(Duration::from_secs(2), rt.upstream_permits.acquire())
                    .await
                    .map_err(|_| AppError::Overloaded)?
                    .expect("semaphore closed");
            analyse_tx(state, chain, hash).await
        }
        .await;

        match &fetched {
            Ok(tx) => {
                if tx.bridge_transfer.is_some() {
                    if let Err(e) = persist_transaction(&state.db, tx).await {
                        tracing::warn!(error = %e, hash, "write-back failed");
                    }
                    remember(rt, hash, tx).await; // 3: fill after fetch
                }
            }
            Err(e) => {
                if let Some(neg) = Neg::from_error(e) {
                    let cache = if matches!(neg, Neg::NotFound) {
                        &rt.not_found
                    } else {
                        &rt.failed
                    };
                    cache.insert(hash.to_string(), neg).await;
                }
            }
        }
        fetched
    }
    .await;

    // only remove the entry if it's still ours
    rt.inflight.remove_if(&key, |_, v| Arc::ptr_eq(v, &lock));
    result
}
