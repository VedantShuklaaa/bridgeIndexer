//! Pure parts of the reconcile tool (`src/bin/reconcile.rs`): planning the
//! signature window, diffing it against the database, bucketing what the adapters
//! say about the remainder, and turning that into recall, miss clustering and lag.
//! No IO, so it is unit tested.
//!
//! Ground truth is one program: every successful signature that touched it inside
//! the window. A signature is *persisted* when `bridge_transfers` has a row with
//! that `observed_tx_hash`, and *missed* when the adapters decode a transfer out
//! of it but no row exists. Recall is measured per transaction, not per decoded
//! transfer, so a transaction with several messages cannot weigh more than one.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Unix seconds, inclusive at both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub start: i64,
    pub end: i64,
}

impl Window {
    /// The window ends `end_offset_secs` ago, so it only covers signatures the
    /// ingester had a chance to see. Whatever is newer than `end` is out.
    pub fn plan(now: i64, window_secs: i64, end_offset_secs: i64) -> Self {
        let end = now.saturating_sub(end_offset_secs);
        Self {
            start: end.saturating_sub(window_secs),
            end,
        }
    }
}

/// One successful `getSignaturesForAddress` entry inside the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub signature: String,
    pub slot: u64,
    pub block_time: i64,
}

/// Accumulates `getSignaturesForAddress` pages, newest first.
#[derive(Debug, Default)]
pub struct SignatureScan {
    pub signatures: Vec<Signature>,
    /// Failed transactions in the window; their effects were reverted, so they
    /// are not ground truth.
    pub failed: u64,
    /// Too new: the ingester could not have seen them before `end`.
    pub too_recent: u64,
    /// Older than `start`. Reaching one means the next page cannot help either.
    pub too_old: u64,
    /// Entries without a signature, slot or block time.
    pub unplaceable: u64,
    seen: BTreeSet<String>,
    done: bool,
}

impl SignatureScan {
    /// Feeds one page. Returns true when the scan can stop: a signature older
    /// than the window was reached, or the page was empty.
    pub fn push_page(&mut self, page: &[Value], window: Window) -> bool {
        if page.is_empty() {
            self.done = true;
            return true;
        }

        for entry in page {
            let signature = entry.get("signature").and_then(Value::as_str);
            let slot = entry.get("slot").and_then(Value::as_u64);
            let block_time = entry.get("blockTime").and_then(Value::as_i64);
            let (Some(signature), Some(slot), Some(block_time)) = (signature, slot, block_time)
            else {
                self.unplaceable += 1;
                continue;
            };

            if block_time > window.end {
                self.too_recent += 1;
                continue;
            }
            if block_time < window.start {
                self.too_old += 1;
                self.done = true;
                break;
            }
            if entry.get("err").is_some_and(|err| !err.is_null()) {
                self.failed += 1;
                continue;
            }
            if self.seen.insert(signature.to_string()) {
                self.signatures.push(Signature {
                    signature: signature.to_string(),
                    slot,
                    block_time,
                });
            }
        }

        self.done
    }

    pub fn is_done(&self) -> bool {
        self.done
    }
}

/// One `bridge_transfers` row already in the database for the window. Several
/// rows can share an `observed_tx_hash`: one transaction can hold several
/// transfers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedTransaction {
    pub observed_tx_hash: String,
    pub slot: i64,
    pub block_time: i64,
    pub stored_at: i64,
}

/// The ground-truth signatures split into persisted and missing.
#[derive(Debug, Default)]
pub struct Diff {
    /// Rows whose transaction is one of the ground-truth signatures.
    pub persisted: Vec<PersistedTransaction>,
    /// Distinct signatures with at least one persisted row.
    pub persisted_hashes: BTreeSet<String>,
    /// Signatures with no persisted row. Only these need `getTransaction`.
    pub missing: Vec<Signature>,
}

impl Diff {
    pub fn persisted_transactions(&self) -> usize {
        self.persisted_hashes.len()
    }
}

pub fn diff(signatures: &[Signature], persisted: &[PersistedTransaction]) -> Diff {
    let in_window: BTreeSet<&str> = signatures
        .iter()
        .map(|signature| signature.signature.as_str())
        .collect();

    let rows: Vec<PersistedTransaction> = persisted
        .iter()
        .filter(|row| in_window.contains(row.observed_tx_hash.as_str()))
        .cloned()
        .collect();
    let persisted_hashes: BTreeSet<String> = rows
        .iter()
        .map(|row| row.observed_tx_hash.clone())
        .collect();
    let missing = signatures
        .iter()
        .filter(|signature| !persisted_hashes.contains(&signature.signature))
        .cloned()
        .collect();

    Diff {
        persisted: rows,
        persisted_hashes,
        missing,
    }
}

/// What happened to one missing signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// The adapters decoded at least one transfer: a miss.
    Transfers(usize),
    /// The adapters ran and said this is not a transfer.
    Rejected {
        kind: &'static str,
        reason: &'static str,
    },
    /// The transaction could not be read (or viewed). Never a miss.
    FetchFailed { kind: String },
    /// The `--max-fetch` cap stopped the run before this signature.
    Unfetched,
}

/// Miss counts and rejection reasons for one program.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ClassificationTally {
    pub missed: u64,
    /// `kind: reason` -> count.
    pub rejected: BTreeMap<String, u64>,
    /// Error kind -> count.
    pub fetch_failed: BTreeMap<String, u64>,
    pub unfetched: u64,
}

impl ClassificationTally {
    pub fn record(&mut self, outcome: &FetchOutcome) {
        match outcome {
            FetchOutcome::Transfers(_) => self.missed += 1,
            FetchOutcome::Rejected { kind, reason } => {
                *self
                    .rejected
                    .entry(format!("{kind}: {reason}"))
                    .or_default() += 1;
            }
            FetchOutcome::FetchFailed { kind } => {
                *self.fetch_failed.entry(kind.clone()).or_default() += 1;
            }
            FetchOutcome::Unfetched => self.unfetched += 1,
        }
    }

    pub fn rejection_total(&self) -> u64 {
        self.rejected.values().sum()
    }

    pub fn fetch_failed_total(&self) -> u64 {
        self.fetch_failed.values().sum()
    }
}

/// Recall over the fetched part of the window. When the cap was hit, the
/// unfetched remainder is unclassified, so `percent` is withheld and only a
/// lower bound is valid.
#[derive(Debug, Clone, PartialEq)]
pub struct Recall {
    pub persisted: u64,
    pub missed: u64,
    pub unfetched: u64,
    /// Exact recall. `None` when the cap was hit or the window held no transfers.
    pub percent: Option<f64>,
    /// Lowest recall consistent with the run: every unfetched signature counted
    /// as a miss. Always `None` for an empty window.
    pub lower_bound: Option<f64>,
    pub capped: bool,
}

pub fn recall(persisted: u64, missed: u64, unfetched: u64) -> Recall {
    let capped = unfetched > 0;
    let known = persisted + missed;
    let total = known + unfetched;
    Recall {
        persisted,
        missed,
        unfetched,
        percent: (!capped && known > 0).then(|| persisted as f64 * 100.0 / known as f64),
        lower_bound: (total > 0).then(|| persisted as f64 * 100.0 / total as f64),
        capped,
    }
}

impl Recall {
    /// Share of the known transactions that were missed. Still meaningful when
    /// the cap was hit, because both counts cover the fetched part.
    pub fn missed_pct(&self) -> Option<f64> {
        let known = self.persisted + self.missed;
        (known > 0).then(|| self.missed as f64 * 100.0 / known as f64)
    }
}

/// Block time (`transactions.timestamp`) to stored time
/// (`bridge_transfers.created_at`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LagStats {
    pub samples: usize,
    pub p50_secs: i64,
    pub p99_secs: i64,
    pub max_secs: i64,
}

pub fn lag_stats(persisted: &[PersistedTransaction]) -> Option<LagStats> {
    if persisted.is_empty() {
        return None;
    }
    // A negative lag can only be clock skew between the chain and Postgres.
    let mut lags: Vec<i64> = persisted
        .iter()
        .map(|row| (row.stored_at - row.block_time).max(0))
        .collect();
    lags.sort_unstable();
    Some(LagStats {
        samples: lags.len(),
        p50_secs: percentile(&lags, 50.0),
        p99_secs: percentile(&lags, 99.0),
        max_secs: *lags.last().unwrap_or(&0),
    })
}

/// Nearest-rank percentile over a sorted slice.
fn percentile(sorted: &[i64], p: f64) -> i64 {
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Which known suspect a miss points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suspect {
    /// A persisted transfer is in the same slot: suspect 1, the slot-granular
    /// checkpoint drops the rest of the slot on reconnect.
    SameSlot,
    /// No persisted transfer within `--gap-secs` either side: suspects 3, 4 or 5,
    /// a dead socket, a swallowed subscribe error or a stopped ingester.
    Gap,
    /// Something else, not explained by the two cluster rules.
    Other,
}

impl Suspect {
    pub fn as_str(self) -> &'static str {
        match self {
            Suspect::SameSlot => "same_slot",
            Suspect::Gap => "gap",
            Suspect::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissCluster {
    pub signature: Signature,
    pub suspect: Suspect,
    /// Seconds to the nearest persisted transfer at or before this miss.
    pub nearest_before_secs: Option<i64>,
    /// Seconds to the nearest persisted transfer at or after this miss.
    pub nearest_after_secs: Option<i64>,
}

pub fn cluster(miss: &Signature, persisted: &[PersistedTransaction], gap_secs: i64) -> MissCluster {
    let same_slot = persisted.iter().any(|row| row.slot == miss.slot as i64);
    let nearest_before_secs = persisted
        .iter()
        .filter(|row| row.block_time <= miss.block_time)
        .map(|row| miss.block_time - row.block_time)
        .min();
    let nearest_after_secs = persisted
        .iter()
        .filter(|row| row.block_time >= miss.block_time)
        .map(|row| row.block_time - miss.block_time)
        .min();

    let in_gap = nearest_before_secs.is_none_or(|secs| secs > gap_secs)
        && nearest_after_secs.is_none_or(|secs| secs > gap_secs);
    let suspect = if same_slot {
        Suspect::SameSlot
    } else if in_gap {
        Suspect::Gap
    } else {
        Suspect::Other
    };

    MissCluster {
        signature: miss.clone(),
        suspect,
        nearest_before_secs,
        nearest_after_secs,
    }
}

/// Picks up to `n` items spread evenly over `items`, deterministically. Used to
/// sample persisted transactions for the precision pass without needing a random
/// number generator, so two runs over the same data compare the same sample.
pub fn sample_evenly<T>(items: &[T], n: usize) -> Vec<&T> {
    if n == 0 || items.is_empty() {
        return Vec::new();
    }
    if n >= items.len() {
        return items.iter().collect();
    }
    (0..n).map(|i| &items[i * items.len() / n]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(signature: &str, slot: u64, block_time: i64, failed: bool) -> Value {
        json!({
            "signature": signature,
            "slot": slot,
            "blockTime": block_time,
            "err": if failed { json!({"InstructionError": [0, "x"]}) } else { Value::Null },
        })
    }

    fn signature(signature: &str, slot: u64, block_time: i64) -> Signature {
        Signature {
            signature: signature.into(),
            slot,
            block_time,
        }
    }

    fn row(hash: &str, slot: i64, block_time: i64, stored_at: i64) -> PersistedTransaction {
        PersistedTransaction {
            observed_tx_hash: hash.into(),
            slot,
            block_time,
            stored_at,
        }
    }

    #[test]
    fn the_window_ends_before_the_offset() {
        let window = Window::plan(10_000, 3600, 600);
        assert_eq!(window.end, 9400);
        assert_eq!(window.start, 5800);
    }

    #[test]
    fn a_page_drops_failed_and_too_recent_and_stops_at_too_old() {
        let window = Window {
            start: 6000,
            end: 9400,
        };
        let page = vec![
            entry("new", 30, 9500, false),
            entry("failed", 29, 9000, true),
            entry("ok", 28, 8000, false),
            entry("ok", 28, 8000, false),
            entry("old", 27, 5000, false),
            entry("older", 26, 4000, false),
        ];

        let mut scan = SignatureScan::default();
        let done = scan.push_page(&page, window);

        assert!(done);
        assert!(scan.is_done());
        assert_eq!(scan.signatures, vec![signature("ok", 28, 8000)]);
        assert_eq!(scan.too_recent, 1);
        assert_eq!(scan.too_old, 1);
        assert_eq!(scan.failed, 1);
        assert_eq!(scan.unplaceable, 0);
    }

    #[test]
    fn an_empty_page_ends_the_scan() {
        let mut scan = SignatureScan::default();
        assert!(scan.push_page(&[], Window { start: 0, end: 10 }));
    }

    #[test]
    fn the_diff_only_returns_signatures_without_a_row() {
        let signatures = vec![
            signature("a", 1, 100),
            signature("b", 2, 200),
            signature("c", 3, 300),
        ];
        let persisted = vec![
            row("a", 1, 100, 105),
            // two transfers in one tx: one persisted transaction, two rows
            row("a", 1, 100, 106),
            // a row whose transaction is not ground truth is ignored
            row("elsewhere", 9, 900, 905),
        ];

        let diff = diff(&signatures, &persisted);

        assert_eq!(diff.persisted.len(), 2);
        assert_eq!(diff.persisted_transactions(), 1);
        let missing: Vec<&str> = diff.missing.iter().map(|s| s.signature.as_str()).collect();
        assert_eq!(missing, vec!["b", "c"]);
    }

    #[test]
    fn outcomes_land_in_their_own_buckets() {
        let mut tally = ClassificationTally::default();
        tally.record(&FetchOutcome::Transfers(2));
        tally.record(&FetchOutcome::Rejected {
            kind: "not_transfer",
            reason: "admin",
        });
        tally.record(&FetchOutcome::Rejected {
            kind: "not_transfer",
            reason: "admin",
        });
        tally.record(&FetchOutcome::Rejected {
            kind: "malformed",
            reason: "layout",
        });
        tally.record(&FetchOutcome::FetchFailed {
            kind: "UPSTREAM_TIMEOUT".into(),
        });
        tally.record(&FetchOutcome::Unfetched);

        assert_eq!(tally.missed, 1);
        assert_eq!(tally.rejected.get("not_transfer: admin"), Some(&2));
        assert_eq!(tally.rejected.get("malformed: layout"), Some(&1));
        assert_eq!(tally.rejection_total(), 3);
        // a failed fetch is not a miss
        assert_eq!(tally.fetch_failed_total(), 1);
        assert_eq!(tally.unfetched, 1);
    }

    #[test]
    fn recall_is_undefined_for_an_empty_window() {
        let recall = recall(0, 0, 0);
        assert_eq!(recall.percent, None);
        assert_eq!(recall.lower_bound, None);
        assert!(!recall.capped);
        assert_eq!(recall.missed_pct(), None);
    }

    #[test]
    fn a_rejected_only_remainder_is_full_recall() {
        // 10 persisted and 0 missed; 7 rejected signatures are correctly absent.
        let recall = recall(10, 0, 0);
        assert_eq!(recall.percent, Some(100.0));
        assert_eq!(recall.lower_bound, Some(100.0));
        assert_eq!(recall.missed_pct(), Some(0.0));
    }

    #[test]
    fn recall_is_the_persisted_share_of_known_transactions() {
        let recall = recall(99, 1, 0);
        assert_eq!(recall.percent, Some(99.0));
        assert_eq!(recall.lower_bound, Some(99.0));
        assert!(!recall.capped);
        assert!((recall.missed_pct().unwrap() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_capped_run_reports_a_lower_bound_instead_of_recall() {
        let recall = recall(10, 2, 8);
        assert!(recall.capped);
        assert_eq!(recall.percent, None);
        let lower = recall.lower_bound.unwrap();
        assert!((lower - 50.0).abs() < f64::EPSILON, "{lower}");
    }

    #[test]
    fn a_miss_in_a_persisted_slot_is_the_checkpoint_suspect() {
        let persisted = vec![row("a", 42, 1000, 1005)];
        let miss = signature("m", 42, 1004);

        let cluster = cluster(&miss, &persisted, 300);

        assert_eq!(cluster.suspect, Suspect::SameSlot);
    }

    #[test]
    fn a_miss_far_from_every_persisted_transfer_is_a_gap() {
        let persisted = vec![row("a", 1, 1000, 1005), row("b", 2, 2000, 2005)];
        let miss = signature("m", 10, 1600);

        let cluster = cluster(&miss, &persisted, 300);

        assert_eq!(cluster.suspect, Suspect::Gap);
        assert_eq!(cluster.nearest_before_secs, Some(600));
        assert_eq!(cluster.nearest_after_secs, Some(400));
    }

    #[test]
    fn an_empty_database_puts_every_miss_in_a_gap() {
        let cluster = cluster(&signature("m", 10, 1600), &[], 300);
        assert_eq!(cluster.suspect, Suspect::Gap);
        assert_eq!(cluster.nearest_before_secs, None);
        assert_eq!(cluster.nearest_after_secs, None);
    }

    #[test]
    fn a_miss_next_to_a_persisted_transfer_is_unexplained() {
        let persisted = vec![row("a", 1, 1900, 1905)];
        let miss = signature("m", 10, 1600);

        let cluster = cluster(&miss, &persisted, 300);

        assert_eq!(cluster.suspect, Suspect::Other);
        assert_eq!(cluster.nearest_before_secs, None);
        assert_eq!(cluster.nearest_after_secs, Some(300));
    }

    #[test]
    fn lag_percentiles_use_nearest_rank() {
        let rows: Vec<PersistedTransaction> =
            (1..=100).map(|lag| row("a", 1, 1000, 1000 + lag)).collect();

        let lag = lag_stats(&rows).unwrap();

        assert_eq!(lag.samples, 100);
        assert_eq!(lag.p50_secs, 50);
        assert_eq!(lag.p99_secs, 99);
        assert_eq!(lag.max_secs, 100);
    }

    #[test]
    fn lag_is_undefined_without_persisted_transfers() {
        assert_eq!(lag_stats(&[]), None);
    }

    #[test]
    fn sampling_is_even_and_bounded() {
        let items: Vec<u32> = (0..10).collect();
        assert_eq!(sample_evenly(&items, 3), vec![&0, &3, &6]);
        assert!(sample_evenly(&items, 0).is_empty());
        assert_eq!(sample_evenly(&items, 99).len(), 10);
        assert!(sample_evenly::<u32>(&[], 5).is_empty());
    }
}
