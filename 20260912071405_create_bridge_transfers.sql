CREATE TABLE bridge_transfers (
    id BIGSERIAL PRIMARY KEY,

    -- Source transaction
    source_tx_hash TEXT NOT NULL UNIQUE,
    source_chain SMALLINT NOT NULL,
    source_wallet TEXT,
    source_explorer_url TEXT,

    -- Destination
    destination_chain SMALLINT NOT NULL,
    destination_wallet TEXT,
    destination_tx_hash TEXT,
    destination_explorer_url TEXT,

    -- Token
    token TEXT,
    token_symbol TEXT,

    -- Amount
    amount TEXT,
    amount_formatted TEXT,

    -- Wormhole message identity
    emitter_chain SMALLINT NOT NULL,
    emitter_address TEXT NOT NULL,
    sequence NUMERIC(39, 0) NOT NULL,

    -- Processing state
    status TEXT NOT NULL,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- A Wormhole message is globally identified by these fields
    UNIQUE (emitter_chain, emitter_address, sequence)
);
