-- A7 origin backfill: record every enrichment attempt on a missing-origin row, whatever
-- the outcome, so the retry window rotates the queue even when the attempt writes nothing
-- back to the row. NULL means never attempted, which is where the rows repaired by the A7
-- migrations start (their cached analysis was dropped).
ALTER TABLE bridge_transfers
    ADD COLUMN last_enrich_attempt_at TIMESTAMPTZ;

-- The backfill orders by the attempt marker, never-attempted rows first, then by
-- updated_at so rows written together still rotate deterministically.
DROP INDEX IF EXISTS idx_bt_missing_origin;
CREATE INDEX idx_bt_missing_origin
ON bridge_transfers (last_enrich_attempt_at ASC NULLS FIRST, updated_at ASC)
WHERE status = 'Completed'
  AND direction = 'Inbound'
  AND source_tx_hash IS NULL;
