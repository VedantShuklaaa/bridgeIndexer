//! Pure parts of the replay tool (`src/bin/replay.rs`): comparing our status with
//! WormholeScan's and summarising a run. No IO, so it is unit tested.
//!
//! WormholeScan's `global-tx` is the reference. It says a transfer is complete
//! when it knows the destination tx, which is the same rule our `Completed` uses.

use crate::domain::bridge_transfer::BridgeStatus;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// Both know the destination tx, and it is the same one.
    BothCompleted,
    /// Neither knows a destination tx yet.
    BothOpen,
    /// WormholeScan knows the destination tx and we say `Pending`. The one case the
    /// exit criterion forbids: `Pending` only when WormholeScan has no destination tx.
    Behind,
    /// WormholeScan knows the destination tx and we say `Detected`.
    Missed,
    /// We say `Completed` and WormholeScan knows no destination tx.
    Ahead,
    /// Both say completed, with different destination txs.
    HashMismatch,
}

impl Verdict {
    pub fn agrees(self) -> bool {
        matches!(self, Verdict::BothCompleted | Verdict::BothOpen)
    }

    pub fn name(self) -> &'static str {
        match self {
            Verdict::BothCompleted => "both_completed",
            Verdict::BothOpen => "both_open",
            Verdict::Behind => "behind",
            Verdict::Missed => "missed",
            Verdict::Ahead => "ahead",
            Verdict::HashMismatch => "hash_mismatch",
        }
    }
}

/// EVM hashes are case-insensitive hex. Everything else (Solana base58) is exact.
fn same_hash(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    if a.starts_with("0x") && b.starts_with("0x") {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// Compares our status (and destination tx) with WormholeScan's destination tx.
pub fn compare(
    ours: &BridgeStatus,
    ours_destination_tx: Option<&str>,
    theirs_destination_tx: Option<&str>,
) -> Verdict {
    let theirs = theirs_destination_tx
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let ours_done = matches!(ours, BridgeStatus::Completed);

    match (ours_done, theirs) {
        (true, Some(t)) => match ours_destination_tx {
            Some(o) if !same_hash(o, t) => Verdict::HashMismatch,
            _ => Verdict::BothCompleted,
        },
        (true, None) => Verdict::Ahead,
        (false, Some(_)) if matches!(ours, BridgeStatus::Pending) => Verdict::Behind,
        (false, Some(_)) => Verdict::Missed,
        (false, None) => Verdict::BothOpen,
    }
}

/// What happened to one signature.
#[derive(Debug, Clone)]
pub enum Outcome {
    /// The pipeline produced a transfer and the reference answered.
    Transfer {
        status: BridgeStatus,
        verdict: Verdict,
        /// Old enough that WormholeScan should know the destination tx by now.
        settled: bool,
    },
    /// The pipeline ran and found no transfer (failed tx, nothing to decode).
    NoTransfer,
    /// The pipeline or the reference lookup failed. The key is the error kind.
    Error(String),
}

#[derive(Debug, Default)]
pub struct Tally {
    pub total: u64,
    pub agree: u64,
    pub by_verdict: BTreeMap<&'static str, u64>,
}

impl Tally {
    fn add(&mut self, verdict: Verdict) {
        self.total += 1;
        if verdict.agrees() {
            self.agree += 1;
        }
        *self.by_verdict.entry(verdict.name()).or_default() += 1;
    }

    pub fn agreement_pct(&self) -> Option<f64> {
        (self.total > 0).then(|| self.agree as f64 * 100.0 / self.total as f64)
    }
}

#[derive(Debug, Default)]
pub struct Summary {
    pub signatures: u64,
    pub no_transfer: u64,
    pub errors: BTreeMap<String, u64>,
    pub detected: u64,
    pub pending: u64,
    pub completed: u64,
    /// Transfers older than the minimum age. The exit criterion is measured here.
    pub settled: Tally,
    /// Younger transfers: reported, never counted against the criterion.
    pub recent: Tally,
    latencies_ms: Vec<u64>,
}

impl Summary {
    pub fn record(&mut self, outcome: &Outcome, latency_ms: u64) {
        self.signatures += 1;
        self.latencies_ms.push(latency_ms);

        match outcome {
            Outcome::Transfer {
                status,
                verdict,
                settled,
            } => {
                match status {
                    BridgeStatus::Detected => self.detected += 1,
                    BridgeStatus::Pending => self.pending += 1,
                    BridgeStatus::Completed => self.completed += 1,
                }
                if *settled {
                    self.settled.add(*verdict);
                } else {
                    self.recent.add(*verdict);
                }
            }
            Outcome::NoTransfer => self.no_transfer += 1,
            Outcome::Error(kind) => *self.errors.entry(kind.clone()).or_default() += 1,
        }
    }

    pub fn transfers(&self) -> u64 {
        self.detected + self.pending + self.completed
    }

    /// Share of transfers still `Pending`.
    pub fn pending_pct(&self) -> Option<f64> {
        (self.transfers() > 0).then(|| self.pending as f64 * 100.0 / self.transfers() as f64)
    }

    /// Nearest-rank percentile of the pipeline latency, in milliseconds.
    pub fn latency_percentile(&self, p: f64) -> Option<u64> {
        if self.latencies_ms.is_empty() {
            return None;
        }
        let mut sorted = self.latencies_ms.clone();
        sorted.sort_unstable();
        let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
        Some(sorted[rank.clamp(1, sorted.len()) - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEST: &str = "0xAbC123";

    #[test]
    fn matching_destination_txs_agree() {
        let v = compare(&BridgeStatus::Completed, Some(DEST), Some("0xabc123"));
        assert_eq!(v, Verdict::BothCompleted);
        assert!(v.agrees());
    }

    #[test]
    fn neither_knowing_a_destination_tx_agrees() {
        for status in [BridgeStatus::Detected, BridgeStatus::Pending] {
            let v = compare(&status, None, None);
            assert_eq!(v, Verdict::BothOpen);
        }
        // an empty answer from the reference is no answer
        assert_eq!(
            compare(&BridgeStatus::Pending, None, Some("  ")),
            Verdict::BothOpen
        );
    }

    #[test]
    fn pending_while_the_reference_has_the_destination_tx_is_behind() {
        let v = compare(&BridgeStatus::Pending, None, Some(DEST));
        assert_eq!(v, Verdict::Behind);
        assert!(!v.agrees());
    }

    #[test]
    fn detected_while_the_reference_has_the_destination_tx_is_missed() {
        assert_eq!(
            compare(&BridgeStatus::Detected, None, Some(DEST)),
            Verdict::Missed
        );
    }

    #[test]
    fn completed_without_a_reference_destination_tx_is_ahead() {
        assert_eq!(
            compare(&BridgeStatus::Completed, Some(DEST), None),
            Verdict::Ahead
        );
    }

    #[test]
    fn different_destination_txs_are_a_mismatch() {
        assert_eq!(
            compare(&BridgeStatus::Completed, Some("0xaaa"), Some("0xbbb")),
            Verdict::HashMismatch
        );
    }

    #[test]
    fn solana_hashes_are_compared_exactly() {
        assert_eq!(
            compare(&BridgeStatus::Completed, Some("AbCd"), Some("abcd")),
            Verdict::HashMismatch
        );
        assert_eq!(
            compare(&BridgeStatus::Completed, Some("AbCd"), Some("AbCd")),
            Verdict::BothCompleted
        );
    }

    #[test]
    fn a_completed_transfer_without_our_hash_still_agrees() {
        assert_eq!(
            compare(&BridgeStatus::Completed, None, Some(DEST)),
            Verdict::BothCompleted
        );
    }

    fn transfer(status: BridgeStatus, verdict: Verdict, settled: bool) -> Outcome {
        Outcome::Transfer {
            status,
            verdict,
            settled,
        }
    }

    #[test]
    fn the_summary_separates_settled_from_recent_transfers() {
        let mut s = Summary::default();
        s.record(
            &transfer(BridgeStatus::Completed, Verdict::BothCompleted, true),
            10,
        );
        s.record(&transfer(BridgeStatus::Pending, Verdict::Behind, true), 20);
        s.record(&transfer(BridgeStatus::Pending, Verdict::Behind, false), 30);
        s.record(&Outcome::NoTransfer, 40);
        s.record(&Outcome::Error("TX_NOT_FOUND".into()), 50);
        s.record(&Outcome::Error("TX_NOT_FOUND".into()), 60);

        assert_eq!(s.signatures, 6);
        assert_eq!(s.transfers(), 3);
        assert_eq!(s.settled.total, 2);
        assert_eq!(s.settled.agreement_pct(), Some(50.0));
        // the recent transfer is reported but not part of the criterion
        assert_eq!(s.recent.total, 1);
        assert_eq!(s.recent.by_verdict.get("behind"), Some(&1));
        assert_eq!(s.no_transfer, 1);
        assert_eq!(s.errors.get("TX_NOT_FOUND"), Some(&2));
        let pending = s.pending_pct().unwrap();
        assert!((pending - 66.666).abs() < 0.01, "{pending}");
    }

    #[test]
    fn agreement_is_undefined_without_transfers() {
        assert_eq!(Summary::default().settled.agreement_pct(), None);
        assert_eq!(Summary::default().pending_pct(), None);
        assert_eq!(Summary::default().latency_percentile(50.0), None);
    }

    #[test]
    fn latency_percentiles_use_nearest_rank() {
        let mut s = Summary::default();
        for ms in (1..=100).rev() {
            s.record(&Outcome::NoTransfer, ms);
        }
        assert_eq!(s.latency_percentile(50.0), Some(50));
        assert_eq!(s.latency_percentile(99.0), Some(99));
        assert_eq!(s.latency_percentile(100.0), Some(100));
        assert_eq!(s.latency_percentile(0.0), Some(1));
    }
}
