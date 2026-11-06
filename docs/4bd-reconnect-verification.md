# 4b + 4d reconnect verification (EC2)

Build: `2c98dde` (merge of PR #72, `fix/4b-4d-stream-reconnect`). `bridge-stream` has run this build since 2026-11-05 17:57:25 UTC. Host: the main app host, RDS Postgres, Wormhole only.

## Results

| Test | Trigger | Outcome |
|---|---|---|
| Real drop | `failed to read Solana WebSocket message` at 19:45:53Z | Reconnect after 5.0 s backoff, connected 19:45:59, backfill from slot 465827819 with `rewind_slots=20`, `published=0 skipped=1`. 3 h reconcile: 97/97 persisted, 0 missed, lag max 11 s. |
| Silent stall (4d) | nftables drop of all traffic to `mainnet.helius-rpc.com`, both directions, 20:23:45Z to 20:25:35Z | Watchdog fired 20:25:00.011Z (`no frame received for 90s`), connection ended `reason="watchdog"`, reconnect attempt 20:25:05Z, connected 20:25:41Z (6 s after unblock), backfill from slot 465837802 with `rewind_slots=20`, `published=0 skipped=1`. |
| Clean 24 h window | stream on this build from 17:57:25Z | `reconcile --window-hours 24 --end-offset-secs 180 --precision-sample 20`, window 2026-11-05T18:01:19Z to 2026-11-06T18:01:19Z: 594/594 persisted, 0 missed (recall 100.00%), 0 rejected, 0 fetch failed. Lag p50 1 s, p99 3 s, max 11 s (n=594). Precision 20/20 (18 both\_completed, 2 both\_open). 39 failed signatures excluded. |

## Notes

* Recovery order is subscribe, then backfill from the checkpoint rewound by 20 slots. The rewind re-sees already-ingested signatures and dedupes them (`skipped=1` in both tests).
* The stall test was ~2 min and no bridge tx landed in it, so it proves the watchdog, reconnect and rewind path, not recovery of a missed transaction. The real-drop reconcile (97/97) covers recall across a reconnect.
* The earlier 24 h reconcile (706/706, lag p99 7796 s, max 8953 s) mostly ran on the old build. The lag tail is the 8984 s `bridge-stream` downtime from 10:42:39Z to 13:12:23Z on 2026-11-05, not a reconnect.
* The connect at 20:25:05Z took 36 s only because the host was blackholed. Recovery came 6 s after the block was removed.
