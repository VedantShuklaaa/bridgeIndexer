CREATE TABLE transactions (
    id BIGSERIAL PRIMARY KEY,

    hash TEXT NOT NULL UNIQUE,
    chain SMALLINT NOT NULL,

    status TEXT NOT NULL,

    slot BIGINT NOT NULL,
    timestamp TIMESTAMPTZ,

    fee_lamports BIGINT NOT NULL,

    signer TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE bridge_transfers (
    id BIGSERIAL PRIMARY KEY,

    transaction_id BIGINT NOT NULL
        REFERENCES transactions(id)
        ON DELETE CASCADE,

    source_tx_hash TEXT NOT NULL UNIQUE,
    source_chain SMALLINT NOT NULL,

    source_wallet TEXT,
    source_explorer_url TEXT,

    emitter_chain SMALLINT NOT NULL,
    emitter_address TEXT NOT NULL,
    sequence NUMERIC(39, 0) NOT NULL,

    destination_chain SMALLINT NOT NULL,
    destination_wallet TEXT,
    destination_tx_hash TEXT,
    destination_explorer_url TEXT,

    token TEXT,
    token_symbol TEXT,

    amount TEXT,
    amount_formatted TEXT,

    status TEXT NOT NULL,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE (emitter_chain, emitter_address, sequence)
);

CREATE INDEX idx_bridge_transfers_destination_tx
    ON bridge_transfers(destination_tx_hash);

CREATE INDEX idx_bridge_transfers_status
    ON bridge_transfers(status);

CREATE INDEX idx_bridge_transfers_destination_chain
    ON bridge_transfers(destination_chain);
