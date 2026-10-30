-- Data repair: the WormholeScan fallback stored the Solana redeemer, the signer of the
-- redeem tx, as source_wallet on inbound rows. A redeem's sender is on the origin chain,
-- and an origin-chain sender is not a Solana address, so chain-first inbound rows store
-- NULL. Those rows are put back to that shape and their cached analysis is dropped: it
-- still holds the old transfer, wallet included, and the next read rebuilds it.
--
-- A second run matches nothing, because every row it changed no longer has a source_wallet.
UPDATE bridge_transfers
SET source_wallet = NULL,
    analysis      = NULL,
    updated_at    = NOW()
WHERE direction = 'Inbound'
  AND source_wallet IS NOT NULL;
