# 3f: write-path throughput of the stream consumer

## Workload

`src/bin/bench_write_path.rs` pre-fills `bridge:transactions` with 5000
signatures, then starts the same consumer tasks the stream role runs and waits
until the consumer group has no lag and no pending entries **and** the
`transactions` table grew by exactly 5000. A dead letter, a missing row or a
nonzero pending count makes the run `INVALID`; a fast run that drops a message
is not reported as a result.

* Signatures are selected from the fake upstream's `/debug/fixture` so each one
  replays a fixture the app resolves to `Completed`. Each message therefore
  spends one Helius `getTransaction`, about half a `getMultipleAccounts`
  preload, and one WormholeScan `operations` call (config 1; config 2's retry
  overhead is below).
* Five workers, `XREADGROUP COUNT 10`, unbounded join over each read batch, one
  Postgres transaction per message, `DB_POOL_MAX 20` — the production defaults.
  A recovery worker runs too, as it does in production.
* Per-message time is publish-to-persist: the wall clock when the message was
  XADDed versus `transactions.created_at`. It includes time spent queued in the
  stream, so p50 is roughly half the run for a pre-filled stream.
* The fake upstream runs with `FAKE_LATENCY_MS=0`, `FAKE_429_RATE=0`. Redis,
  Postgres and the consumer share the laptop with the fake.

**Fixture collision caveat.** The fake has six fixture pairs, and every one of
the 5000 signatures maps to the same Completed pair by hash. `transactions.hash`
is the requested signature, so 5000 rows are inserted, but the decoded
`(protocol, message_key)` is identical, so all 5000 messages upsert the **same**
`bridge_transfers` row. A message is therefore one insert plus one update of a
hot row. Real traffic has distinct message keys and would not serialise on that
row, so the absolute rates here are a lower bound and the batched-upsert verdict
below is specific to this row shape.

## Configurations

**(1) Isolated write path.** `HELIUS_LIVE_RPS=100000`, `WORMHOLE_RPS=100000`,
so the provider quota is out of the way and Redis, Postgres and worker
concurrency are the only limits.

A first pass at `HELIUS_LIVE_RPS=1000` is included because it shows the quota
still binds: each message spends ~1.5 Helius tokens (1 `getTransaction` + 0.5
preload), and the run's 7459 token acquisitions over 6.6 s exhaust both the
limiter's initial burst and its refill. The measured ~760 messages/s over this short backlog is therefore a
little above the ~667 messages/s a 1000/s refill would sustain indefinitely;
either way it is a quota effect, not a write-path effect.

**(2) Real limits.** `HELIUS_LIVE_RPS=4` (free plan), `WORMHOLE_RPS=20`.
`HELIUS_LIVE_TOKEN_WAIT_MS=60000` for this run only: with the default 20 s
patience the 50-message round needs ~19 s just for first Helius tokens, so a
large fraction of messages shed and re-enter `analyse_with_retry`, spending
another token each time. With 60 s the Helius first-token wait stops shedding.
The WormholeScan client keeps its own 2 s token wait, so a small fraction
(~11%) still sheds and retries once; that overhead is included in the measured
rate. The default remains 20 s.

## Results

Laptop: Apple M5 (10 cores), macOS 26.6.1. Postgres 18 and Redis 7.4.9 in
Docker. These are iteration numbers, not EC2 numbers.

### Baseline, production defaults

| Config | Run | msgs/s | p50 (ms) | p99 (ms) | rows | dead letters |
| --- | --- | --- | --- | --- | --- | --- |
| 1, `HELIUS_LIVE_RPS=1000` | 1 (final tree) | 764.7 | 2743.78 | 6395.10 | 5000 | 0 |
| 1, `HELIUS_LIVE_RPS=1000` | 2 (final tree) | 760.4 | 2765.89 | 6434.43 | 5000 | 0 |
| 1, `HELIUS_LIVE_RPS=100000` | 1 (final tree) | 2356.9 | 1030.64 | 1950.80 | 5000 | 0 |
| 1, `HELIUS_LIVE_RPS=100000` | 2 (final tree) | 2348.4 | 965.03 | 1910.79 | 5000 | 0 |
| 2, `HELIUS_LIVE_RPS=4` | 1 | 2.3 | 1007744 | 1994711 | 5000 | 0 |
| 2, `HELIUS_LIVE_RPS=4` | 2 (final tree) | 2.4 | 1009526 | 2011026 | 5000 | 0 |

Config 2 run 2 also recorded the Helius ingest-lane histogram: **4.0 s average
wait over 8742 token acquisitions**, so the workers spend almost all of their
time parked on the 4/s quota. Both config 2 runs spent 2130 s and 2046 s on the
5000-message backlog, and p50 is about half of that because the stream was
pre-filled. `bridge_transfers` grew by 0 in both runs because its single shared
row already existed from earlier runs; `transactions` grew by exactly 5000.

Repeated runs differ by 0.6% (config 1 at 1000/s) and 0.4% (config 1 at
100000/s). The database was already warm from earlier runs, so neither repeat is
a true cold-database number; they are labelled so a warm/cold difference is not
hidden inside an average. Config 2 run 1 used a bench binary built before the
metrics endpoint was added; its consumer path is identical and it simply does
not print the limiter line.

An earlier config 1 pass on the pre-final bench binary measured 2376.9
messages/s at 100000/s; that is the baseline row in the experiment table. The
experiment runs all used that same binary, so their deltas are internally
consistent, and the final-tree repeats above are 0.8–1.2% lower.

### Experiments (config 1, 100000 tokens/s unless noted)

| Change | Setting | msgs/s | vs baseline |
| --- | --- | --- | --- |
| baseline | `COUNT 10`, unbounded, batch 1, pool 20 | 2376.9 | — |
| (a) read count | `COUNT 50` | 2263.7 | −4.8% |
| (a) read count | `COUNT 100` | 2329.2 | −2.0% |
| (b) bounded concurrency | `buffer_unordered(10)`, COUNT 100 | 2317.5 | −2.5% |
| (b) bounded concurrency | `buffer_unordered(40)`, COUNT 100 | 2041.2 | −14.1% |
| (c) batched upserts | batch 50 | 277.7 | −88.3% |
| (c) batched upserts | batch 10 (at 1000 tokens/s) | 28.1 | −96.3% |
| (c) batched upserts | batch 50 (at 1000 tokens/s) | 244.9 | −67.9% |
| (d) pool size | `DB_POOL_MAX 10` | 2191.4 | −7.8% |
| (d) pool size | `DB_POOL_MAX 40` | 1838.8 | −22.6% |
| worker count | 1 | 1493.3 | −37.2% |
| worker count | 2 | 1917.3 | −19.3% |
| worker count | 10 | 2263.2 | −4.8% |

Every setting other than the baseline is a single run; the spread between
repeated baseline runs at the same limiter was 0.6%, so differences under ~3%
are noise. The 1000-token/s numbers are flat across every experiment because
the quota, not the write path, is the limit there.

### What limits the write path

* **Config (1): Postgres work per message.** Throughput is flat at ~2.35k
  messages/s from 10 to 500 in-flight messages, from a 10- to a 40-connection
  pool, and from `COUNT 10` to `COUNT 100`. More concurrency queues work
  without adding throughput, and a bigger pool makes it slightly worse. During
  a run Postgres sat at ~1.3 cores (Docker stats) while the consumer process
  and the fake upstream were not saturated. Per-message work is one transaction
  containing an insert plus the hot-row upsert described above. Raising the
  pool cannot help a serialised row update; batched transactions make it worse
  because they hold that row lock for the whole batch.
* **Config (2): provider quota.** 4 Helius tokens/s and about 1.6 tokens per
  message (1.11 `getTransaction` + 0.51 `getMultipleAccounts`), so the ceiling
  is near 2.5 messages/s. Measured 2.3 and 2.4 messages/s, and the ingest-lane
  histogram averages 4.0 s of wait per acquisition: workers are parked on the
  quota, not on Redis or Postgres. The extra 0.11 call does not appear in
  config 1 (1.00 per message) on the same fixtures, so it is the WormholeScan
  shed described above, not a fixture property.

The fake-upstream fixture set means the config (1) ceiling is pessimistic for
real traffic: distinct message keys would replace the serialised upsert with
independent inserts, and batched transactions could then amortise commit cost.
That shape cannot be produced from six fixture pairs, so batching was not kept
on the strength of a speculative benefit.

## Kept

* `CONSUMER_COUNT` (default 5): worker tasks. Five is already on the plateau
  (1 → 1493, 2 → 1917, 5 → 2357, 10 → 2263 messages/s); the knob exists so a
  larger instance can be tuned, not because a different value was measured
  better.
* `CONSUMER_READ_COUNT` (default 10): `XREADGROUP COUNT`. No measured gain at
  50 or 100; default unchanged.
* `DB_POOL_MAX` (default 20): Postgres pool per process. No measured gain at 10
  or 40; default unchanged.

All three default to the previous hardcoded values, so a deployment that sets
nothing behaves exactly as before.

## Dropped

* **Bounded per-worker concurrency** (`CONSUMER_CONCURRENCY`): neutral at 10
  and worse at 40. The existing unbounded join over a `COUNT`-sized read batch
  is already the concurrency control. Code removed.
* **Batched upserts** (`CONSUMER_BATCH`): 3–27× slower on this workload because
  every batch transaction serialises on the one shared `bridge_transfers` row.
  A per-message fallback preserved ack/retry/dead-letter semantics, and the
  consumer reliability tests stayed green, but the change cannot be justified
  by measurement. Code removed.
* **Larger `XREADGROUP COUNT`**: flat. The knob stays at the old default.

## Reliability

`cargo test --test consumer_reliability` (4 tests) passed after the final tree:
immediate dead letter for a permanent failure, delivery-budget dead letter,
a burst through a slow limiter fully persisted, and a reclaimed `Overloaded`
message staying pending. No change touches ack-after-persist, retry or
`XAUTOCLAIM` semantics.

## RDS connection assumption

`DB_POOL_MAX` bounds one process. The API and stream roles can run as separate
instances or one `ROLE=all`; at the default of 20 the worst case is 40
connections (plus a second API instance, 60). A `db.t3.micro` has about 112
connections available by RDS's default formula (`DBInstanceClassMemory /
9531392`, 1 GiB), and `db.t3.small` about 225, so the default fits with room
for admin sessions. Raise `DB_POOL_MAX` only after checking the instance class:
the sweep showed no throughput gain from doing so.

## EC2 results

Run on the main EC2 box (`<instance type>`), Postgres on RDS (`<instance class>`,
`max_connections` = `<n>`), a throwaway local Redis on port 6380, the fake upstream
on loopback, `bridge-api` and `bridge-stream` stopped, git SHA `<sha>`. The
production Redis is Upstash, so Redis round trips are not in these numbers and the
real ceiling is lower. Config (1), 5000 messages, every run valid (5000/5000 rows,
0 dead letters, 0 pending, limiter wait 0 ms).

| Setting | msgs/s | p50 | p99 |
|---|---|---|---|
| baseline (5 workers, pool 20, COUNT 10), run 1 | 919 | 2.69 s | 5.23 s |
| baseline, run 2 | 892 | 2.78 s | 5.40 s |
| baseline, run 3 | 903 | 2.72 s | 5.35 s |
| `DB_POOL_MAX=10`, run 1 | 998 | 2.43 s | 4.82 s |
| `DB_POOL_MAX=10`, run 2 | 1022 | 2.36 s | 4.75 s |
| `DB_POOL_MAX=10`, run 3 | 997 | 2.47 s | 4.93 s |
| `DB_POOL_MAX=40` | 732 | 3.29 s | 6.57 s |
| `BENCH_WORKERS=1` | 548 | 4.15 s | 8.97 s |
| `BENCH_WORKERS=2` | 757 | 3.34 s | 6.49 s |
| `BENCH_WORKERS=10` | 848 | 2.81 s | 5.54 s |
| `BENCH_WORKERS=20` | 839 | 3.06 s | 5.86 s |
| `CONSUMER_READ_COUNT=50` | 836 | 3.06 s | 5.76 s |

A first run with the services still up scored 1016 msgs/s, so run-to-run noise
on this box is about 10%.

* The EC2 ceiling is about 0.9k messages/s against 2.35k on the laptop. Limiter
  wait is 0, so the limit is RDS round trips plus the hot-row upsert, not the
  provider quota or the consumer CPU.
* Worker count plateaus at 5 as on the laptop. 10 and 20 do not help.
* A pool of 10 is about 11% faster than 20 on this fixture set, and 40 is 19%
  slower. The fixtures collapse onto one hot `bridge_transfers` row, so this is
  not carried over to real traffic. Defaults stay at 5 / 10 / 20; a smaller pool
  for `bridge-stream` is an option to try with real traffic.

## Reproduce on EC2

Run on the app host with the fake upstream on the same host (loopback, no
provider calls), Postgres and Redis reachable through the environment:

```sh
cargo build --release --bin fake_upstream --bin bench_write_path
FAKE_LATENCY_MS=0 FAKE_PORT=9090 ./target/release/fake_upstream &

The bench runs `DEL bridge:transactions bridge:transactions:failed` and writes
5000 rows. Never point it at the production Redis (Upstash has one logical DB)
or the production database. Use a local throwaway Redis and a scratch database:

    redis6-server --port 6380 --bind 127.0.0.1 --save "" --appendonly no --daemonize yes
    psql "$DB" -c "CREATE DATABASE bench_3f;"

Use `REDIS_URL=redis://127.0.0.1:6380`, `DATABASE_URL` pointing at `bench_3f`, and
also set `HELIUS_API_URL`, `HELIUS_STREAM_URL` and `HELIUS_LIVE_URL` to the
fake so a real key in `.env` cannot be used. The worker count in the bench is
`BENCH_WORKERS`; `CONSUMER_COUNT` only affects the real binary.

# config (1) isolated
DATABASE_URL=... REDIS_URL=... HELIUS_URL=http://127.0.0.1:9090 \
WORMHOLE_URL=http://127.0.0.1:9090 SOLANA_WS_URL=ws://127.0.0.1:9090 \
SOLANA_TOKEN_BRIDGE_PROGRAM=wormDTUJ6AWPNvk59vGQbDvGJmqbdc3c7R3P5LxV5hY \
HELIUS_LIVE_RPS=100000 WORMHOLE_RPS=100000 METRICS_PORT=9101 \
BENCH_N=5000 BENCH_PREFIX=ec2-cfg1-$(date +%s) RUST_LOG=warn \
./target/release/bench_write_path

# config (2) real quota
DATABASE_URL=... REDIS_URL=... HELIUS_URL=http://127.0.0.1:9090 \
WORMHOLE_URL=http://127.0.0.1:9090 SOLANA_WS_URL=ws://127.0.0.1:9090 \
SOLANA_TOKEN_BRIDGE_PROGRAM=wormDTUJ6AWPNvk59vGQbDvGJmqbdc3c7R3P5LxV5hY \
HELIUS_LIVE_RPS=4 WORMHOLE_RPS=20 HELIUS_LIVE_TOKEN_WAIT_MS=60000 \
METRICS_PORT=9101 BENCH_N=5000 BENCH_PREFIX=ec2-cfg2-$(date +%s) RUST_LOG=warn \
./target/release/bench_write_path
```

Each run prints one block. `INVALID` means the row count, pending count or
dead-letter count failed; do not compare an `INVALID` run.

## Metrics

The merged Prometheus metrics are installed by the bench (`METRICS_PORT`,
default 9100) and it prints the Helius ingest-lane `limiter_wait_seconds`
average at the end. That is the 4.0 s figure quoted for config 2. Cache metrics
do not appear on the write path, and no other merged metric changed the
conclusions, so they are not listed.
