use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::{Mutex, Semaphore};

use crate::db::repository::{get_cached_analysis, persist_transaction};
use crate::domain::transaction::NormalisedTransaction;
use crate::error::AppError;
use crate::services::analyzer::analyse_tx;
use crate::state::AppState;

pub struct ReadThrough {
    inflight: DashMap<String, Arc<Mutex<()>>>,
    upstream_permits: Semaphore,
}

impl ReadThrough {
    pub fn new(max_concurrent_upstream: usize) -> Self {
        Self {
            inflight: DashMap::new(),
            upstream_permits: Semaphore::new(max_concurrent_upstream),
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

    if let Some(hit) = lookup(state, hash).await {
        return Ok(hit);
    }

    // single-flight: one upstream fetch per key, concurrent callers wait then re-read the DB
    let key = format!("{chain}:{hash}");
    let rt = &state.read_through;
    let lock = rt
        .inflight
        .entry(key.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;

    if let Some(hit) = lookup(state, hash).await {
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
        }
    }

    rt.inflight.remove(&key);
    result
}
