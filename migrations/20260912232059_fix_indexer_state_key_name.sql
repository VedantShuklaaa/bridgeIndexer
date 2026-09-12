

INSERT INTO indexer_state (key, value)
SELECT 'solana_last_ingested_slot', value
FROM indexer_state
WHERE key = 'solana_last_processed_slot'
ON CONFLICT (key) DO NOTHING;


INSERT INTO indexer_state (key, value)
VALUES ('solana_last_ingested_slot', '0')
ON CONFLICT (key) DO NOTHING;


DELETE FROM indexer_state WHERE key = 'solana_last_processed_slot';