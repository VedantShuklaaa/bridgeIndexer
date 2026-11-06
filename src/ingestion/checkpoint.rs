//! The per-connection checkpoint gate: decides when a live notification may advance the
//! persisted ingestion checkpoint. Pure state, no IO.
//!
//! Invariant: while the backfill for the current connection is unfinished, a live
//! notification never advances the persisted checkpoint. The persisted checkpoint
//! therefore always sits at or before the oldest slot the backfill has yet to replay, so
//! a connection that dies mid-backfill leaves it unchanged and the next connection
//! re-backfills the same gap. Once the backfill has completed, the gate opens and the
//! live path persists the newest slot it saw while the backfill ran.

/// What the live path should do with the slot of a notification it just published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointUpdate {
    /// Persist this slot now; the store keeps the maximum.
    Persist(u64),
    /// A backfill is still running. The slot is remembered and persisted when the
    /// backfill completes, so the checkpoint cannot jump over the unfinished gap.
    Deferred,
}

/// Tracks the checkpoint advance for one WebSocket connection.
#[derive(Debug)]
pub struct CheckpointGate {
    start_checkpoint: u64,
    backfill_running: bool,
    live_max: Option<u64>,
}

impl CheckpointGate {
    /// `start_checkpoint` is the slot the backfill resumes from, and
    /// `backfill_running` is true exactly when that backfill task was spawned. Without a
    /// backfill there is no gap to protect, so live messages persist immediately.
    pub fn new(start_checkpoint: u64, backfill_running: bool) -> Self {
        Self {
            start_checkpoint,
            backfill_running,
            live_max: None,
        }
    }

    /// The persisted checkpoint this connection started from.
    pub fn start_checkpoint(&self) -> u64 {
        self.start_checkpoint
    }

    /// Records a live notification. While the backfill runs nothing may be persisted;
    /// afterwards the caller persists the slot directly.
    pub fn on_live_message(&mut self, slot: u64) -> CheckpointUpdate {
        self.live_max = Some(self.live_max.map_or(slot, |current| current.max(slot)));

        if self.backfill_running {
            CheckpointUpdate::Deferred
        } else {
            CheckpointUpdate::Persist(slot)
        }
    }

    /// The backfill completed successfully: open the gate and return the newest live
    /// slot seen while it ran, which is now safe to persist. `None` means no live
    /// notification arrived during the backfill, so the stored checkpoint already
    /// covers everything.
    pub fn on_backfill_finished(&mut self) -> Option<u64> {
        self.backfill_running = false;
        self.live_max
    }

    /// The backfill failed or panicked. The gate intentionally stays closed for the rest
    /// of this connection: any live slot persisted now could move the checkpoint past
    /// the unreplayed gap, and the next reconnect must re-backfill it.
    pub fn on_backfill_failed(&mut self) {
        self.backfill_running = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the `GREATEST` upsert in `db::repository::set_last_ingested_slot`.
    fn persist(checkpoint: &mut u64, update: CheckpointUpdate) {
        if let CheckpointUpdate::Persist(slot) = update {
            *checkpoint = (*checkpoint).max(slot);
        }
    }

    #[test]
    fn live_slots_are_deferred_while_the_backfill_runs() {
        let mut gate = CheckpointGate::new(100, true);

        assert_eq!(gate.on_live_message(110), CheckpointUpdate::Deferred);
        assert_eq!(gate.on_live_message(120), CheckpointUpdate::Deferred);
    }

    #[test]
    fn a_connection_dropped_mid_backfill_leaves_the_checkpoint_unchanged() {
        let start = 1_000;
        let mut persisted = start;
        let mut gate = CheckpointGate::new(start, true);

        for slot in [1_010, 1_020, 1_030] {
            persist(&mut persisted, gate.on_live_message(slot));
        }

        // The connection ends here: the backfill is cancelled, so it never reports
        // completion and the deferred live slots are never persisted.
        assert_eq!(persisted, start);

        // The next connection backfills from the same slot.
        let next = CheckpointGate::new(persisted, true);
        assert_eq!(next.start_checkpoint(), start);
    }

    #[test]
    fn a_completed_backfill_opens_the_gate_and_releases_the_newest_live_slot() {
        let mut persisted = 100;
        let mut gate = CheckpointGate::new(100, true);

        for slot in [110, 105, 120] {
            persist(&mut persisted, gate.on_live_message(slot));
        }
        assert_eq!(persisted, 100, "nothing was persisted during the backfill");

        let released = gate.on_backfill_finished();
        assert_eq!(
            released,
            Some(120),
            "the newest live slot, not the last one"
        );
        persist(&mut persisted, CheckpointUpdate::Persist(released.unwrap()));
        assert_eq!(persisted, 120);

        assert_eq!(gate.on_live_message(130), CheckpointUpdate::Persist(130));
        persist(&mut persisted, CheckpointUpdate::Persist(130));
        assert_eq!(persisted, 130);
    }

    #[test]
    fn a_completed_backfill_without_live_messages_has_nothing_to_persist() {
        let mut gate = CheckpointGate::new(100, true);

        assert_eq!(gate.on_backfill_finished(), None);
        assert_eq!(gate.on_live_message(110), CheckpointUpdate::Persist(110));
    }

    #[test]
    fn a_failed_backfill_keeps_the_gate_closed_for_the_connection() {
        let start = 500;
        let mut persisted = start;
        let mut gate = CheckpointGate::new(start, true);

        persist(&mut persisted, gate.on_live_message(510));
        gate.on_backfill_failed();
        persist(&mut persisted, gate.on_live_message(520));

        assert_eq!(persisted, start, "a failed backfill must not open the gate");
    }

    #[test]
    fn without_a_backfill_live_messages_persist_immediately() {
        let mut gate = CheckpointGate::new(0, false);

        assert_eq!(gate.on_live_message(10), CheckpointUpdate::Persist(10));
    }

    #[test]
    fn an_older_live_slot_still_persists_after_the_backfill() {
        let mut gate = CheckpointGate::new(1_000, true);

        gate.on_live_message(1_001);
        assert_eq!(gate.on_backfill_finished(), Some(1_001));

        // Out-of-order notifications are safe too: the store's GREATEST keeps 1_001.
        assert_eq!(gate.on_live_message(900), CheckpointUpdate::Persist(900));
    }
}
