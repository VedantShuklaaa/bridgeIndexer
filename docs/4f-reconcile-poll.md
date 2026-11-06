# 4f — periodic reconciliation poll

The WebSocket (`logsSubscribe`) is the low-latency path; the poll is the completeness
safety net next to it. 4a measured recall, 4b/4d/4e removed the known reconnect losses,
and 4g established that Helius' paid streaming options are not available on the current
plan. The poll asks the chain directly what touched each program and repairs anything
the store is missing.

## What runs

The task is `reconcile-poll`, supervised next to `solana-ingester`. It starts only when
`INGEST_ENABLED` is on, in the roles that run the pipeline (`all` and `stream`). Every
`INGEST_POLL_SECS` (default 300, minimum 30), for each registered program:

1. read the program's checkpoint from `indexer_state`
   (`reconcile_poll_<program>_last_end`);
2. plan the window `[last_poll_end - INGEST_POLL_OVERLAP_SECS, now -
   INGEST_POLL_OFFSET_SECS]`; on the first run there is no checkpoint, so the anchor
   is `now` and the window starts `INGEST_POLL_OVERLAP_SECS` before it;
3. page `getSignaturesForAddress` through the shared reconcile scanner, newest first,
   until the window's start (same `SignatureScan` the `reconcile` bin uses);
4. drop failed transactions and anything newer than the window's end;
5. query `bridge_transfers` for the same window, per protocol, and subtract the
   signatures that already have a row (`services::reconcile::diff`);
6. publish every remaining signature as a normal `CandidateTransaction` on
   `bridge:transactions`, up to `INGEST_POLL_MAX_PUBLISH` (default 500);
7. advance the checkpoint to the window's end with a `GREATEST` upsert.

The consumers classify and persist the candidates exactly as they do a live
notification. A republication is safe: the existing Redis `SET NX EX` dedupe
(`INGEST_DEDUPE_TTL_SECS`, default two hours) suppresses a signature published
recently, and the PostgreSQL upserts are idempotent.

The checkpoint uses the existing `indexer_state` table, so no migration was added.

## Isolation and limits

* **Own Helius lane.** The poll calls `HELIUS_RECONCILE_URL` (falling back to
  `HELIUS_URL`) at `RECONCILE_RPS` (default 3/s) and labels its upstream metrics
  `lane="reconcile"`. The limiter is built separately from the API lane's and the
  ingest lane's, so a cycle cannot starve either. In `ROLE=all` the poll and a manual
  `reconcile` bin run each build their own limiter; they share the configured quota,
  not the token bucket.
* **Publish cap.** `INGEST_POLL_MAX_PUBLISH` bounds the Helius spend after a long gap.
  A hit is logged, and the checkpoint is *not* advanced (on a first run it is pinned to
  the window's start), so the next cycle re-scans the remainder instead of moving past
  it. Signatures already published are suppressed by the Redis dedupe, so each capped
  cycle makes progress through the backlog.
* **Overlap and offset.** The offset keeps the window behind the ingester, so a
  notification in flight is not treated as missing. The overlap re-covers the part the
  previous window ended before seeing, so a signature near a window edge is not lost.
* **Failure handling.** A scan, database or Redis error fails only that program's
  cycle: it is logged, the checkpoint is not advanced, and the next cycle retries.
  Nothing in the poll panics. A failed cycle is counted with
  `poll_cycles_total{result="error"}`.

## Metrics

| Metric | Type | Labels | Meaning |
| --- | --- | --- | --- |
| `poll_cycles_total` | counter | `program`, `result` (`ok`, `error`) | One per program per cycle |
| `poll_missed_total` | counter | `program` | Signatures the poll had to publish because no row existed and no recent publish suppressed them |
| `poll_last_success_timestamp_seconds` | gauge | `program` | Unix time of the last successful cycle; alert when it goes stale |

A successful cycle that published anything logs a warning with the count and the
window. `poll_missed_total` is not the same as the `reconcile` bin's miss count: the
poll cannot tell a transfer from an admin/attest call without fetching and decoding
it, so an admin call from a period the WebSocket did not cover can be counted once.
The consumer rejects it and the Redis dedupe prevents a repeat. Classification
precision remains the `reconcile` bin's job.

## Tests

`src/services/reconcile.rs` has the pure unit tests: first-run window planning,
overlap against the previous end, the effect of the end offset, an empty window when
the overlap is shorter than the offset, a checkpoint ahead of the wall clock, the
monotonic checkpoint helper, and missing-signature selection (persisted rows are
dropped, including when an overlapping window scans them again). No network, no
database, no Docker.

The IO path (scan, database query, Redis publish) is the same code the `reconcile`
bin already used; the container-backed integration suite was not extended for it.
