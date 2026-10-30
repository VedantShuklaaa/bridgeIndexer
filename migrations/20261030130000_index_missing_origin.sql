-- Origin-tx backfill (A7): completed inbound rows whose source tx is still unknown.
-- Serves list_missing_origin_transfers, which filters on these columns and orders by
-- updated_at (the last-attempt marker).
CREATE INDEX idx_bt_missing_origin
ON bridge_transfers (updated_at)
WHERE status = 'Completed'
  AND direction = 'Inbound'
  AND source_tx_hash IS NULL;
