use crate::domain::bridge_transfer::BridgeStatus;
use axum::body::Bytes;
use dashmap::DashMap;
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::db::repository::{
    get_analysis_any_age, get_transaction_with_transfers, persist_analysis,
};
use crate::domain::api::TransactionResponse;
use crate::domain::bridge::BridgeEvent;
use crate::domain::transaction::NormalisedTransaction;
use crate::error::AppError;
use crate::routes::v1::compat;
use crate::services::analyzer::analyse_tx;
use crate::services::api_v2;
use crate::state::AppState;

const FRESH_SECS: f64 = 60.0;
const MAX_BACKGROUND_REFRESHES: usize = 8;

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
                retryable: true,
            },
        }
    }
}

/// A `/api/v2` response and the legacy `bridge_event` `/api/v1` still exposes. The
/// event is not part of the generalised model; keeping it here lets the compatibility
/// mapper reproduce the frozen v1 response exactly.
#[derive(Clone)]
pub struct V2Analysis {
    response: TransactionResponse,
    bridge_event: Option<BridgeEvent>,
    v1_body: Option<Bytes>,
    v2_body: Option<Bytes>,
}

impl V2Analysis {
    /// Fills the cache entry. The `/api/v1` body is serialized once here, so a hit
    /// clones a `Bytes` handle instead of remapping and re-serializing.
    // v2-only misses pay this v1 serialization once too; that cost is cold-path only.
    pub fn new(response: TransactionResponse, bridge_event: Option<BridgeEvent>) -> Self {
        let v1_body = compat::to_v1(&response, bridge_event.clone())
            .ok()
            .and_then(|tx| serde_json::to_vec(&tx).ok())
            .map(Bytes::from);
        // Same bytes axum's `Json` would produce per request; cold-path cost only.
        let v2_body = serde_json::to_vec(&response).ok().map(Bytes::from);
        Self {
            response,
            bridge_event,
            v1_body,
            v2_body,
        }
    }

    pub fn response(&self) -> &TransactionResponse {
        &self.response
    }

    pub fn bridge_event(&self) -> Option<&BridgeEvent> {
        self.bridge_event.as_ref()
    }

    pub fn v1_body(&self) -> Option<&Bytes> {
        self.v1_body.as_ref()
    }

    pub fn v2_body(&self) -> Option<&Bytes> {
        self.v2_body.as_ref()
    }
}

pub struct ReadThrough {
    inflight: DashMap<String, Arc<Mutex<()>>>,
    upstream_permits: Semaphore,
    completed: Cache<String, NormalisedTransaction>,
    open: Cache<String, NormalisedTransaction>,
    v2_completed: Cache<String, Arc<V2Analysis>>,
    v2_open: Cache<String, Arc<V2Analysis>>,
    // negative caches
    not_found: Cache<String, Neg>, // tx doesn't exist: longer TTL
    failed: Cache<String, Neg>,    // transient upstream failure: very short TTL
    // stale-while-revalidate
    refresh_permits: Arc<Semaphore>,
    refresh_cooldown: Cache<String, ()>, // at most one refresh attempt per key per TTL
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
                .time_to_live(Duration::from_secs(15))
                .build(),
            v2_completed: Cache::builder()
                .max_capacity(50_000)
                .time_to_live(Duration::from_secs(600))
                .build(),
            v2_open: Cache::builder()
                .max_capacity(10_000)
                .time_to_live(Duration::from_secs(15))
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
            refresh_permits: Arc::new(Semaphore::new(MAX_BACKGROUND_REFRESHES)),
            refresh_cooldown: Cache::builder()
                .max_capacity(50_000)
                .time_to_live(Duration::from_secs(15))
                .build(),
        }
    }
}

/// DB lookup of any stored analysis. Returns (analysis, is_fresh).
async fn lookup(state: &AppState, hash: &str) -> Option<(NormalisedTransaction, bool)> {
    match get_analysis_any_age(&state.db, hash, FRESH_SECS).await {
        Ok(Some(c)) => serde_json::from_value(c.analysis)
            .map(|tx| (tx, c.fresh))
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

/// Background refresh of a stale entry. Failures are only logged: the stale
/// copy keeps being served and the cooldown spaces out the next attempt.
async fn refresh(state: AppState, chain: String, hash: String, _slot: OwnedSemaphorePermit) {
    let rt = &state.read_through;

    let fetched = async {
        let _permit = rt
            .upstream_permits
            .acquire()
            .await
            .expect("semaphore closed");
        analyse_tx(&state, &chain, &hash).await
    }
    .await;

    match fetched {
        Ok(analysed) if analysed.tx.bridge_transfer.is_some() => {
            if let Err(e) = persist_analysis(&state.db, &analysed.tx, &analysed.transfers).await {
                tracing::warn!(error = %e, hash = %hash, "refresh write-back failed");
            }
            remember(rt, &hash, &analysed.tx).await;
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %e, hash = %hash, "background refresh failed, still serving stale");
        }
    }
}

async fn maybe_spawn_refresh(state: &AppState, chain: &str, hash: &str) {
    let rt = &state.read_through;

    if rt.refresh_cooldown.contains_key(hash) {
        return;
    }
    // bounded number of concurrent background refreshes; skip if all busy
    let Ok(slot) = rt.refresh_permits.clone().try_acquire_owned() else {
        return;
    };
    rt.refresh_cooldown.insert(hash.to_string(), ()).await;

    let state = state.clone();
    let chain = chain.to_string();
    let hash = hash.to_string();
    tokio::spawn(async move {
        refresh(state, chain, hash, slot).await;
    });
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

        if let Some((hit, fresh)) = lookup(state, hash).await {
            remember(rt, hash, &hit).await;
            if !fresh {
                maybe_spawn_refresh(state, chain, hash).await;
            }
            return Ok(hit);
        }

        // nothing stored anywhere: real cold fetch, bounded wait for a permit
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
            Ok(analysed) => {
                if analysed.tx.bridge_transfer.is_some() {
                    if let Err(e) =
                        persist_analysis(&state.db, &analysed.tx, &analysed.transfers).await
                    {
                        tracing::warn!(error = %e, hash, "write-back failed");
                    }
                    remember(rt, hash, &analysed.tx).await; // 3: fill after fetch
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
        // v1 only needs the transaction body here; every decoded transfer has been
        // written by `persist_analysis` above.
        fetched.map(|analysed| analysed.tx)
    }
    .await;

    // only remove the entry if it's still ours
    rt.inflight.remove_if(&key, |_, v| Arc::ptr_eq(v, &lock));
    result
}

/// The `/api/v2` read path. It reuses the v1 freshness, single-flight and negative-cache
/// machinery (so a cold analysis is done once), then serves every stored transfer of the
/// transaction from Postgres.
pub async fn analyse_cached_v2(
    state: &AppState,
    chain: &str,
    hash: &str,
) -> Result<Arc<V2Analysis>, AppError> {
    let rt = &state.read_through;

    if let Some(hit) = rt.v2_completed.get(hash).await {
        return Ok(hit);
    }
    if let Some(hit) = rt.v2_open.get(hash).await {
        return Ok(hit);
    }

    // Ensures the transaction has been analysed and persisted, with the same caching,
    // single-flight and upstream protection as v1.
    let legacy = analyse_cached(state, chain, hash).await?;

    let snapshot = state.chains.snapshot();
    let response = match get_transaction_with_transfers(&state.db, hash).await {
        Ok(Some(stored)) => api_v2::assemble(stored, &snapshot)?,
        // A transaction with no stored transfer rows: the analysis is the only source.
        Ok(None) => api_v2::from_v1(&legacy, &snapshot),
        Err(error) => return Err(AppError::Internal(error)),
    };

    let entry = Arc::new(V2Analysis::new(response, legacy.bridge_event));
    let cache = if entry.response().is_complete() {
        &rt.v2_completed
    } else {
        &rt.v2_open
    };
    cache.insert(hash.to_string(), entry.clone()).await;
    Ok(entry)
}
