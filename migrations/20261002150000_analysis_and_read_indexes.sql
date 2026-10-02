ALTER TABLE bridge_transfers
ADD COLUMN analysis JSONB;

CREATE INDEX idx_bt_created
ON bridge_transfers (created_at DESC, id DESC);

CREATE INDEX idx_bt_status_created
ON bridge_transfers (status, created_at DESC, id DESC);

CREATE INDEX idx_bt_open
ON bridge_transfers (updated_at)
WHERE status IN ('Pending', 'Detected');