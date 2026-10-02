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

pub struct ReadThrough {
    inflight: DashMap<String, Arc<Mutex<()>>>,
    upstream_permits: Semaphore,
    completed: Cache<String, NormalisedTransaction>,
    open: Cache<String, NormalisedTransaction>,
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
                .time_to_live(Duration::from_secs(5))
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
    if let Some(hit) = rt.completed.get(hash).await {
        return Ok(hit);
    } // 1: memory first
    if let Some(hit) = rt.open.get(hash).await {
        return Ok(hit);
    }

    if let Some(hit) = lookup(state, hash).await {
        remember(rt, hash, &hit).await; // 2: fill on DB hit
        return Ok(hit);
    }

    let key = format!("{chain}:{hash}");
    let lock = rt
        .inflight
        .entry(key.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;

    if let Some(hit) = lookup(state, hash).await {
        remember(rt, hash, &hit).await;
        rt.inflight.remove(&key);
        return Ok(hit);
    }

    let result = {
        let _permit = rt
            .upstream_permits
            .acquire()
            .await
            .expect("semaphore closed");
        analyse_tx(state, chain, hash).await
    };

    if let Ok(tx) = &result {
        if tx.bridge_transfer.is_some() {
            if let Err(e) = persist_transaction(&state.db, tx).await {
                tracing::warn!(error = %e, hash, "write-back failed");
            }
            remember(rt, hash, tx).await; // 3: fill after fetch
        }
    }

    rt.inflight.remove(&key);
    result
}
