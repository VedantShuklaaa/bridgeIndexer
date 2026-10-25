-- A7: redeems into Solana are stored as inbound rows, with the redeem tx as the
-- destination tx and the origin chain's tx unknown until enrichment finds it.
--
-- Before A7 the WormholeScan-driven path stored a redeem like a send: direction
-- 'Outbound' and the redeem's own signature as source_tx_hash. A message emitted by any
-- chain other than Solana (Wormhole chain 1) and observed on Solana can only be a
-- redeem, so those rows are turned around. The cached analysis is dropped because it
-- still describes the old shape: the next read re-analyses the tx and rewrites it.
UPDATE bridge_transfers bt
SET direction                = 'Inbound',
    source_tx_hash           = NULL,
    source_explorer_url      = NULL,
    destination_explorer_url = CASE
        WHEN bt.destination_tx_hash = bt.observed_tx_hash THEN bt.destination_explorer_url
    END,
    destination_tx_hash      = bt.observed_tx_hash,
    status                   = 'Completed',
    analysis                 = NULL,
    updated_at               = NOW()
FROM wormhole_messages wm
WHERE wm.transfer_id = bt.id
  AND bt.protocol = 'wormhole'
  AND bt.direction = 'Outbound'
  AND wm.emitter_chain <> 1
  AND bt.destination_ref_protocol = 'wormhole'
  AND bt.destination_ref_id = '1';