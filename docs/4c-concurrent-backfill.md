# 4c — backfill concurrent with live reads

Before 4c the ingester subscribed, ran the whole backfill (paced at
`INGEST_BACKFILL_PACE_MS` per signature) and only then entered the read loop. With the
defaults a 5000-signature replay takes about 62 minutes, and for that whole time live
notifications sat in the socket. 4c moves the backfill into its own task next to the
read loop.

## What runs

After `logsSubscribe` is confirmed, `run_connection`:

1. reads the persisted checkpoint (`solana_last_ingested_slot`);
2. if `LIVE_BACKFILL` is on and the checkpoint is non-zero, spawns the backfill task and
   builds a `CheckpointGate` closed until that task reports completion;
3. enters the read loop immediately.

The task is a child of the connection's cancellation token. When the connection ends for
any reason — read error, close frame, watchdog, shutdown — `run_connection` cancels the
child token and awaits the task before returning, so a reconnect never overlaps two
backfills and the `ingest_backfill_inflight` gauge returns to zero. The next connection
starts a fresh backfill from the persisted checkpoint.

Live notifications and backfilled signatures use the same publish path
(`RedisProducer::publish_transaction`). The Redis `SET NX EX` dedupe and the idempotent
Postgres upserts already make an overlap harmless. `INGEST_BACKFILL_MAX` and
`INGEST_BACKFILL_PACE_MS` keep their meaning: the cap counts published signatures in one
run and the pace sleeps between them. The task uses the same ingest Helius lane as before.

## The checkpoint invariant

The persisted checkpoint is also the resume point of the next backfill, so it must never
pass an unreplayed gap:

> While the backfill for this connection is unfinished, a live notification publishes its
> signature but does not advance the persisted checkpoint. The checkpoint therefore
> always sits at or before the oldest slot the backfill has yet to replay.

`src/ingestion/checkpoint.rs` is the pure state machine (`CheckpointGate`) that enforces
it:

| Event | Gate state | Result |
| --- | --- | --- |
| No backfill (fresh DB, `LIVE_BACKFILL=0`, checkpoint read error) | open | live message persists its slot immediately |
| Live message while the backfill runs | closed | slot remembered, nothing persisted |
| Backfill completes | opens | persists the newest live slot seen while it ran (GREATEST keeps the larger value) |
| Backfill fails or panics | stays closed | nothing is persisted for the rest of the connection; the next reconnect retries from the unchanged checkpoint |
| Connection drops mid-backfill | task cancelled | deferred live slots are never persisted; the next backfill starts from the same slot |

While the gate is closed, live messages still refresh `updated_at` on the checkpoint row
(`touch_last_ingested_slot`) without changing its value. `/health` therefore keeps seeing
live ingestion during a long backfill, but its `ingest_age_seconds` cannot be satisfied by
advancing past the gap.

## Failure handling

The task catches its own panic (`catch_unwind`). A publish or Helius error, or a panic,
is logged and marks the gate failed; the live read loop is unaffected. The checkpoint
stays where it was and the next reconnect re-runs the backfill. The task never tears down
the connection.

## Metrics

| Metric | Type | Meaning |
| --- | --- | --- |
| `ingest_backfill_inflight` | gauge | Backfill tasks currently running (0 or 1), held by a drop guard so a panic cannot leak it |
| `ingest_backfill_duration_seconds` | histogram | Wall-clock duration of one run, including paced publishes and cancellation |
| `ingest_backfill_published_total` | counter | Signatures the backfill published; dedupe suppressions are not counted |

## Tests

`src/ingestion/checkpoint.rs` has the pure unit tests for the invariant: live slots are
deferred during the backfill, a connection dropped mid-backfill leaves the checkpoint
unchanged (and the next gate starts from that same slot), completion releases the newest
live slot and opens the gate, completion without live messages persists nothing, a failed
backfill keeps the gate closed, and no-backfill connections persist immediately. No
network, Redis, Postgres or Docker.

## Verification plan (EC2)

1. Start `bridge-stream` on this build with `LIVE_BACKFILL=1` and a checkpoint far enough
   behind that the next connect starts a backfill with several signatures (or lower
   `INGEST_BACKFILL_PACE_MS` in a test environment so the run is long enough to interrupt).
2. Confirm the log line `Starting Solana transaction backfill` (with `last_ingested_slot`,
   `rewind_slots` and `max`) and that `ingest_backfill_inflight` is `1`.
3. While `ingest_backfill_inflight` is still `1`, break the connection: `nftables`/`iptables`
   DROP both directions to the WebSocket host for longer than `INGEST_WATCHDOG_SECS`.
4. Expected: the watchdog fires, `ingest_reconnects_total{reason="watchdog"}` increments,
   the backfill task is cancelled (`Solana transaction backfill cancelled with its
   connection`), and `ingest_backfill_inflight` returns to `0`.
5. Confirm `solana_last_ingested_slot` is unchanged from before the drop. The checkpoint
   key's `updated_at` may be newer (heartbeat only); the value is not.
6. After the reconnect, confirm a new backfill starts from the same value
   (`last_ingested_slot` in the start log) and `ingest_backfill_inflight` is `1` again.
7. Live path: during the first run, a bridge transaction confirmed while the backfill was
   running must appear in the stream and database without waiting for the backfill to
   finish. Its log line ends `checkpoint advance deferred to the backfill`.
