//! The reconcile tool's optional precision pass: re-runs persisted transactions
//! through `analyse_tx` and compares their status with WormholeScan, reusing the
//! replay tool's comparison.
//!
//! Like `replay`, this only reads. `analyse_tx` fetches, decodes and enriches but
//! never persists; every write in the pipeline happens in the Redis consumer and
//! the read-through writer, and neither is reachable from here.

use crate::clients::limiter::call_upstream;
use crate::clients::wormhole;
use crate::services::analyzer::analyse_tx;
use crate::services::replay::{Outcome, Summary, compare};
use crate::state::AppState;
use std::time::Instant;

pub struct PrecisionReport {
    pub sampled: usize,
    pub summary: Summary,
}

pub async fn precision_pass(state: &AppState, hashes: &[&str]) -> PrecisionReport {
    let mut summary = Summary::default();

    for hash in hashes {
        let started = Instant::now();
        let result = analyse_tx(state, "solana", hash).await;
        let elapsed_ms = started.elapsed().as_millis() as u64;

        match result {
            Ok(analysed) => match analysed.tx.bridge_transfer {
                Some(transfer) => {
                    let theirs = call_upstream(&state.wormhole_limiter, || {
                        wormhole::get_destination_tx(
                            &state.http_client,
                            &state.config.wormhole_url,
                            &transfer.message_id,
                        )
                    })
                    .await;
                    match theirs {
                        Ok(destination) => {
                            let verdict = compare(
                                &transfer.status,
                                transfer.destination_tx_hash.as_deref(),
                                destination.as_deref(),
                            );
                            summary.record(
                                &Outcome::Transfer {
                                    status: transfer.status,
                                    verdict,
                                    settled: true,
                                },
                                elapsed_ms,
                            );
                        }
                        Err(error) => {
                            let kind = format!("reference:{}", error.code());
                            summary.record(&Outcome::Error(kind), elapsed_ms);
                        }
                    }
                }
                None => summary.record(&Outcome::NoTransfer, elapsed_ms),
            },
            Err(error) => summary.record(&Outcome::Error(error.code().to_string()), elapsed_ms),
        }
    }

    PrecisionReport {
        sampled: hashes.len(),
        summary,
    }
}
