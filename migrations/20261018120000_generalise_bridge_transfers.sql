CREATE FUNCTION resolve_chain(p_protocol TEXT, p_id TEXT) RETURNS SMALLINT
LANGUAGE sql STABLE AS $$
    SELECT m.chain_id
    FROM protocol_chain_ids m
    JOIN chains c ON c.id = m.chain_id
    WHERE m.protocol = p_protocol
      AND m.protocol_chain_id = p_id
      AND m.status = 'active'
      AND c.status = 'active'
$$;

ALTER TABLE bridge_transfers
    ADD COLUMN protocol                 TEXT,
    ADD COLUMN message_key              TEXT,
    ADD COLUMN direction                TEXT,
    -- position among the transfers decoded from one Solana tx
    ADD COLUMN event_index              SMALLINT NOT NULL DEFAULT 0,
    -- the Solana signature this transfer was decoded from
    ADD COLUMN observed_tx_hash         TEXT,
    -- outer router or protocol, when there is one
    ADD COLUMN via                      TEXT,
    ADD COLUMN via_ref                  TEXT,
    ADD COLUMN source_ref_protocol      TEXT,
    ADD COLUMN source_ref_id            TEXT,
    ADD COLUMN source_chain_id          SMALLINT REFERENCES chains (id),
    ADD COLUMN destination_ref_protocol TEXT,
    ADD COLUMN destination_ref_id       TEXT,
    ADD COLUMN destination_chain_id     SMALLINT REFERENCES chains (id),
    -- where the token lives; unknown for rows written before this migration
    ADD COLUMN token_ref_protocol       TEXT,
    ADD COLUMN token_ref_id             TEXT,
    ADD COLUMN token_chain_id           SMALLINT REFERENCES chains (id);

UPDATE bridge_transfers SET
    protocol                 = 'wormhole',
    message_key              = emitter_chain::text || '/' || emitter_address || '/' || sequence::text,
    direction                = 'Outbound',
    observed_tx_hash         = source_tx_hash,
    source_ref_protocol      = 'wormhole',
    source_ref_id            = source_chain::text,
    source_chain_id          = resolve_chain('wormhole', source_chain::text),
    destination_ref_protocol = 'wormhole',
    destination_ref_id       = destination_chain::text,
    destination_chain_id     = resolve_chain('wormhole', destination_chain::text);

CREATE TABLE wormhole_messages (
    transfer_id     BIGINT PRIMARY KEY REFERENCES bridge_transfers (id) ON DELETE CASCADE,
    -- INTEGER, not SMALLINT: a Wormhole chain ID is a u16
    emitter_chain   INTEGER NOT NULL,
    emitter_address TEXT NOT NULL,
    -- a u64 has at most 20 digits
    sequence        NUMERIC(20, 0) NOT NULL,
    UNIQUE (emitter_chain, emitter_address, sequence)
);

INSERT INTO wormhole_messages (transfer_id, emitter_chain, emitter_address, sequence)
SELECT id, emitter_chain, emitter_address, sequence
FROM bridge_transfers;

-- TEXT -> NUMERIC(78,0). A value that is not a plain integer becomes NULL instead of
-- failing the migration. Nothing is lost: the full analysis JSON still holds it.
DO $$
DECLARE
    skipped BIGINT;
BEGIN
    SELECT COUNT(*) INTO skipped
    FROM bridge_transfers
    WHERE amount IS NOT NULL AND amount !~ '^[0-9]{1,78}$';
    IF skipped > 0 THEN
        RAISE NOTICE 'A2: % row(s) had a non-integer amount, set to NULL (see analysis JSON)', skipped;
    END IF;
END $$;

ALTER TABLE bridge_transfers
    ALTER COLUMN amount TYPE NUMERIC(78, 0)
    USING (CASE WHEN amount ~ '^[0-9]{1,78}$' THEN amount::numeric END);

ALTER TABLE bridge_transfers
    DROP COLUMN emitter_chain,
    DROP COLUMN emitter_address,
    DROP COLUMN sequence,
    DROP COLUMN source_chain,
    DROP COLUMN destination_chain;

-- One Solana tx can now hold several transfers, and for an inbound redeem the tx on
-- the source chain is only known later, from enrichment.
ALTER TABLE bridge_transfers DROP CONSTRAINT IF EXISTS bridge_transfers_source_tx_hash_key;
ALTER TABLE bridge_transfers ALTER COLUMN source_tx_hash DROP NOT NULL;

ALTER TABLE bridge_transfers
    ALTER COLUMN protocol                 SET NOT NULL,
    ALTER COLUMN message_key              SET NOT NULL,
    ALTER COLUMN direction                SET NOT NULL,
    ALTER COLUMN observed_tx_hash         SET NOT NULL,
    ALTER COLUMN source_ref_protocol      SET NOT NULL,
    ALTER COLUMN source_ref_id            SET NOT NULL,
    ALTER COLUMN destination_ref_protocol SET NOT NULL,
    ALTER COLUMN destination_ref_id       SET NOT NULL,
    ADD CONSTRAINT bridge_transfers_direction_check
        CHECK (direction IN ('Outbound', 'Inbound')),
    ADD CONSTRAINT bridge_transfers_status_check
        CHECK (status IN ('Detected', 'Pending', 'Completed', 'Failed', 'Refunded')),
    ADD CONSTRAINT bridge_transfers_message_unique
        UNIQUE (protocol, message_key);

-- idx_bridge_transfers_destination_chain went with the dropped column.
CREATE INDEX idx_bt_observed_tx ON bridge_transfers (observed_tx_hash, event_index);
CREATE INDEX idx_bt_destination_chain ON bridge_transfers (destination_chain_id);

-- Unresolved chain references, so activating a registry mapping can find the rows
-- it now resolves.
CREATE INDEX idx_bt_unresolved_source
    ON bridge_transfers (source_ref_protocol, source_ref_id)
    WHERE source_chain_id IS NULL;
CREATE INDEX idx_bt_unresolved_destination
    ON bridge_transfers (destination_ref_protocol, destination_ref_id)
    WHERE destination_chain_id IS NULL;