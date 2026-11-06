//! Pure planning for the startup backfill: no network, no state.

use serde_json::Value;

use super::types::CandidateTransaction;

/// What one `getSignaturesForAddress` page contributes to the backfill.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BackfillPage {
    /// Successful transactions newer than the checkpoint, newest first.
    pub candidates: Vec<CandidateTransaction>,
    /// The page reached the checkpoint (or the `limit`), so there is nothing older to fetch.
    pub done: bool,
}

/// What one connection's backfill did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct BackfillStats {
    /// Signatures published to the stream.
    pub published: usize,
    /// Signatures the Redis dedupe suppressed because they were already published.
    pub skipped: usize,
}

/// A fresh database has checkpoint 0, and walking the whole history of a busy bridge program
/// from there would burn the Helius quota for nothing. Backfill only runs once there is a
/// checkpoint to resume from; live ingestion starts the record from "now".
pub fn should_backfill(checkpoint_slot: u64) -> bool {
    checkpoint_slot > 0
}

/// Picks the candidates out of one page, newest first, as Helius returns it.
///
/// Stops at the first signature at or below the cutoff, which is `checkpoint_slot`
/// rewound by `rewind_slots` (floored at 0). Rewinding re-reads the checkpoint's own
/// slot and the slots just before it, where transactions can be missed on a reconnect;
/// the Redis dedupe drops whatever was already published. Everything after the cutoff
/// is older, so the page is done. Failed transactions and entries without a slot or
/// signature are skipped, and no more than `remaining` candidates are returned.
pub fn plan_page(
    signatures: &[Value],
    checkpoint_slot: u64,
    rewind_slots: u64,
    remaining: usize,
) -> BackfillPage {
    let cutoff = checkpoint_slot.saturating_sub(rewind_slots);
    let mut page = BackfillPage::default();

    for info in signatures {
        let Some(slot) = info.get("slot").and_then(Value::as_u64) else {
            continue;
        };

        if slot <= cutoff {
            page.done = true;
            break;
        }

        if info.get("err").is_some_and(|err| !err.is_null()) {
            continue;
        }

        let Some(signature) = info.get("signature").and_then(Value::as_str) else {
            continue;
        };

        if page.candidates.len() >= remaining {
            page.done = true;
            break;
        }

        page.candidates.push(CandidateTransaction {
            chain: "solana".to_string(),
            tx_hash: signature.to_string(),
            slot,
        });
    }

    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sig(signature: &str, slot: u64) -> Value {
        json!({ "signature": signature, "slot": slot, "err": null })
    }

    fn hashes(page: &BackfillPage) -> Vec<&str> {
        page.candidates.iter().map(|c| c.tx_hash.as_str()).collect()
    }

    #[test]
    fn fresh_database_does_not_backfill() {
        assert!(!should_backfill(0));
        assert!(should_backfill(1));
    }

    #[test]
    fn takes_everything_newer_than_the_checkpoint() {
        let page = plan_page(&[sig("c", 30), sig("b", 20), sig("a", 10)], 5, 0, 100);

        assert_eq!(hashes(&page), ["c", "b", "a"]);
        assert!(!page.done);
    }

    #[test]
    fn stops_at_the_checkpoint_slot_itself_without_a_rewind() {
        let page = plan_page(&[sig("c", 30), sig("b", 20), sig("a", 10)], 20, 0, 100);

        assert_eq!(hashes(&page), ["c"]);
        assert!(page.done);
    }

    #[test]
    fn includes_transactions_in_the_checkpoints_own_slot() {
        let page = plan_page(
            &[sig("e", 105), sig("d", 100), sig("c", 95), sig("b", 80)],
            100,
            20,
            100,
        );

        assert_eq!(hashes(&page), ["e", "d", "c"]);
        assert!(page.done);
    }

    #[test]
    fn includes_the_whole_rewind_window_and_stops_at_its_edge() {
        let page = plan_page(
            &[sig("f", 100), sig("e", 99), sig("d", 90), sig("c", 81)],
            100,
            20,
            100,
        );

        assert_eq!(hashes(&page), ["f", "e", "d", "c"]);
        assert!(!page.done);

        // Slot 80 is the rewound cutoff itself, one slot too old.
        let page = plan_page(&[sig("d", 90), sig("c", 80), sig("b", 79)], 100, 20, 100);

        assert_eq!(hashes(&page), ["d"]);
        assert!(page.done);
    }

    #[test]
    fn a_checkpoint_below_the_rewind_floors_at_slot_zero() {
        let page = plan_page(&[sig("c", 10), sig("b", 5), sig("a", 0)], 5, 20, 100);

        assert_eq!(hashes(&page), ["c", "b"]);
        assert!(page.done);
    }

    #[test]
    fn the_rewind_never_exceeds_the_remaining_cap() {
        let page = plan_page(
            &[sig("d", 100), sig("c", 99), sig("b", 90), sig("a", 81)],
            100,
            20,
            2,
        );

        assert_eq!(hashes(&page), ["d", "c"]);
        assert!(page.done);
    }

    #[test]
    fn skips_failed_transactions() {
        let failed =
            json!({ "signature": "bad", "slot": 25, "err": { "InstructionError": [0, "Custom"] } });
        let page = plan_page(&[sig("c", 30), failed, sig("a", 10)], 5, 0, 100);

        assert_eq!(hashes(&page), ["c", "a"]);
    }

    #[test]
    fn skips_entries_missing_a_slot_or_signature() {
        let no_slot = json!({ "signature": "x", "err": null });
        let no_signature = json!({ "slot": 40, "err": null });
        let page = plan_page(&[no_slot, no_signature, sig("a", 10)], 5, 0, 100);

        assert_eq!(hashes(&page), ["a"]);
    }

    #[test]
    fn never_returns_more_than_remaining() {
        let page = plan_page(&[sig("c", 30), sig("b", 20), sig("a", 10)], 5, 0, 2);

        assert_eq!(hashes(&page), ["c", "b"]);
        assert!(page.done);
    }

    #[test]
    fn exactly_remaining_candidates_is_not_yet_done() {
        let page = plan_page(&[sig("b", 20), sig("a", 10)], 5, 0, 2);

        assert_eq!(hashes(&page), ["b", "a"]);
        assert!(!page.done);
    }

    #[test]
    fn nothing_remaining_returns_nothing() {
        let page = plan_page(&[sig("a", 10)], 5, 0, 0);

        assert!(page.candidates.is_empty());
        assert!(page.done);
    }

    #[test]
    fn an_empty_page_is_not_done_by_itself() {
        let page = plan_page(&[], 5, 20, 10);

        assert_eq!(page, BackfillPage::default());
    }

    #[test]
    fn candidates_carry_chain_and_slot() {
        let page = plan_page(&[sig("a", 10)], 5, 0, 10);

        assert_eq!(page.candidates[0].chain, "solana");
        assert_eq!(page.candidates[0].slot, 10);
    }
}
