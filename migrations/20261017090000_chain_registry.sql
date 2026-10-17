-- Canonical chains, plus a mapping from each protocol's own chain-ID namespace
-- (Wormhole chain IDs, EVM chain IDs, later CCTP domains, LayerZero endpoint IDs, ...)
-- to a canonical chain. Only `active` rows resolve; anything else is kept for review.

CREATE TABLE chains (
    id              SMALLINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    slug            TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL CHECK (kind IN ('solana', 'evm', 'other')),
    -- URL template with a {hash} placeholder
    explorer_tx_url TEXT,
    status          TEXT NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active', 'pending_review', 'rejected')),
    origin          TEXT NOT NULL DEFAULT 'seed'
                    CHECK (origin IN ('seed', 'manual', 'llm')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE protocol_chain_ids (
    protocol          TEXT NOT NULL,
    -- decimal string: the namespaces differ in width and some are not numeric
    protocol_chain_id TEXT NOT NULL,
    chain_id          SMALLINT NOT NULL REFERENCES chains (id),
    status            TEXT NOT NULL DEFAULT 'active'
                      CHECK (status IN ('active', 'pending_review', 'rejected')),
    origin            TEXT NOT NULL DEFAULT 'seed'
                      CHECK (origin IN ('seed', 'manual', 'llm')),
    -- where the mapping was checked (URL or doc), required before auto-activation
    evidence          TEXT,
    verified_at       TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (protocol, protocol_chain_id)
);

CREATE INDEX idx_protocol_chain_ids_chain ON protocol_chain_ids (chain_id);

-- Seed: the 10 chains the indexer already knew about.
INSERT INTO chains (slug, name, kind, explorer_tx_url) VALUES
    ('solana',    'Solana',          'solana', 'https://explorer.solana.com/tx/{hash}'),
    ('ethereum',  'Ethereum',        'evm',    'https://etherscan.io/tx/{hash}'),
    ('bsc',       'BNB Smart Chain', 'evm',    'https://bscscan.com/tx/{hash}'),
    ('polygon',   'Polygon',         'evm',    'https://polygonscan.com/tx/{hash}'),
    ('avalanche', 'Avalanche',       'evm',    'https://snowtrace.io/tx/{hash}'),
    ('near',      'NEAR',            'other',  'https://nearblocks.io/txns/{hash}'),
    ('arbitrum',  'Arbitrum',        'evm',    'https://arbiscan.io/tx/{hash}'),
    ('optimism',  'Optimism',        'evm',    'https://optimistic.etherscan.io/tx/{hash}'),
    ('gnosis',    'Gnosis',          'evm',    'https://gnosisscan.io/tx/{hash}'),
    ('base',      'Base',            'evm',    'https://basescan.org/tx/{hash}');

-- Wormhole chain IDs, checked against sdk/vaa/structs.go in wormhole-foundation/wormhole.
INSERT INTO protocol_chain_ids (protocol, protocol_chain_id, chain_id, evidence, verified_at)
SELECT 'wormhole', m.id, c.id,
       'https://github.com/wormhole-foundation/wormhole/blob/main/sdk/vaa/structs.go',
       NOW()
FROM (VALUES
    ('1',  'solana'),
    ('2',  'ethereum'),
    ('4',  'bsc'),
    ('5',  'polygon'),
    ('6',  'avalanche'),
    ('15', 'near'),
    ('23', 'arbitrum'),
    ('24', 'optimism'),
    ('25', 'gnosis'),
    ('30', 'base')
) AS m (id, slug)
JOIN chains c ON c.slug = m.slug;

-- EVM chain IDs, checked against github.com/ethereum-lists/chains.
INSERT INTO protocol_chain_ids (protocol, protocol_chain_id, chain_id, evidence, verified_at)
SELECT 'evm', m.id, c.id,
       'https://github.com/ethereum-lists/chains',
       NOW()
FROM (VALUES
    ('1',     'ethereum'),
    ('56',    'bsc'),
    ('137',   'polygon'),
    ('43114', 'avalanche'),
    ('42161', 'arbitrum'),
    ('10',    'optimism'),
    ('100',   'gnosis'),
    ('8453',  'base')
) AS m (id, slug)
JOIN chains c ON c.slug = m.slug;