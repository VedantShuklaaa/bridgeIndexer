# 4g — Helius streaming options (evaluated 2026-11-07)

Context: hermit ingests via standard `logsSubscribe` on the Free plan (1M credits/month, 10 RPC req/s). Goal: recall ≥ 99.9% per program over 24 h.

| Option | Plan needed | Fit |
| --- | --- | --- |
| Standard WebSocket (`logsSubscribe`), current | Free | In use. Metered at 2 credits per 0.1 MB streamed since 2026-05-01 |
| Enhanced WebSockets (`transactionSubscribe`) | Developer and up | Not on Free |
| LaserStream gRPC (mainnet) | Business ($499/mo) and up | Has `fromSlot` replay (~24 h per the replay page; one overview page says ~3000 slots, unverified), SDK auto-reconnect |
| Webhooks | Free has 1 webhook | Needs a public HTTPS endpoint; auto-disabled at 95%+ failure rate; single extra path, not a replacement |

## Decision

Keep `logsSubscribe` as the low-latency path. Build 4f (periodic reconciliation poll) as the completeness guarantee. Revisit LaserStream only if the plan is upgraded; its replay would replace most of 4b/4c.

## Open items

* Measure streamed MB/day on the ingester to see how much of the 1M credits it uses.
* Verify the LaserStream replay window (24 h vs 3000 slots) against the current docs before any migration.

Sources: helius.dev/docs/billing/plans, /docs/billing/credits, /docs/laserstream/historical-replay, /docs/laserstream, helius.dev/webhooks
