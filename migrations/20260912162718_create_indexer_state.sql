-- Add migration script here
CREATE TABLE indexer_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO indexer_state (key, value)
VALUES ('solana_last_processed_slot', '0');