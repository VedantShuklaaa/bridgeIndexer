# Scaling log

Every performance claim in this repository needs measured before/after numbers
(CLAUDE.md). Add an entry only after the runs exist; never fill a number in from
an estimate.

Results go in `bench-results/<timestamp>/` and are kept out of git through
`.git/info/exclude`.

## How to run R-hit

Run the steps in this order. The first 50-connection run fills the cache, spends
Helius quota, and is excluded from the summary table.

1. Start the API server with ingestion off and warning-level logs. This is the
   deployed API server for the test, not a shared or production URL.

   ```bash
   INGEST_ENABLED=0 RUST_LOG=warn ./target/release/bridge
   ```

   `INGEST_ENABLED` is only disabled by the exact value `0`. Any other value
   (including `false`) leaves ingestion on.

2. On the load generator, build the URL list from the deployed database.

   ```bash
   export DATABASE_URL='postgres://user:password@host:5432/bridge'
   export BASE_URL='http://172.31.38.190:8080'

   scripts/make_urls.sh --print-sql          # inspect the query first
   scripts/make_urls.sh v1 urls.txt          # 769 URLs, newest first
   ```

   Use `scripts/make_urls.sh v2 urls-v2.txt` or
   `--api-version v2` to target `/api/v2`.

3. Start CPU sampling on both the API server and the load generator, before the
   series starts.

   ```bash
   ssh api-host 'scripts/cpu_sample.sh /tmp/rhit-server-cpu.log'
   scripts/cpu_sample.sh /tmp/rhit-loadgen-cpu.log
   ```

4. Run the series. The optional second positional argument is recorded with the
   date and oha version in `bench-results/<timestamp>/meta.txt`.

   ```bash
   BASE_URL="$BASE_URL" \
     scripts/bench_rhit.sh urls.txt "$(git rev-parse HEAD)"
   ```

5. Ctrl+C both `cpu_sample.sh` processes. Read the table printed by
   `bench_rhit.sh`, then fill in one entry below.

R-hit uses oha with `-z 30s`, `--urls-from-file`, and `--output-format json`.
The script raises the open-file limit to 65535 before the first run and stops
with a clear message if that fails.

## Entry template

Copy this block for each experiment. Leave every result row blank until the runs
have been made.

```markdown
## <experiment> (<date>)

### Hypothesis

### Workload

### Setup

### Before

### After

### Delta

### Runs

| Run | Connections | req/s | median latency (ms) | p99 latency (ms) | success rate | non-200 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
|  |  |  |  |  |  |  |
|  |  |  |  |  |  |  |
|  |  |  |  |  |  |  |
|  |  |  |  |  |  |  |

Median and min–max across the repeated runs:

| Metric | Before (median, min–max) | After (median, min–max) | Delta |
| --- | --- | --- | --- |
|  |  |  |  |

### Limited by

### Notes
```

## Arc AppState, 35d3e1b vs 4ffb783 (2026-10-26)

### Setup

m7i-flex.large server, c7i.xlarge load generator. Limited by server CPU. Load
generator about 72% idle. Zero non-200s in every run.

### Workload

`R-hit`, single warm key set. Says nothing about the cold path or writes.

### Reason for change

Profile showed `AppState::clone` plus drops at about 4% and `String::clone` at
1.35%.

### Control

Old commit 4ffb783 gave 77,275 and 77,302 req/s at 08:40 and 76,954 and 77,329
at 11:24. Within about 0.5%, so there was no drift.

### Runs

req/s:

| Connections | 4ffb783 (08:40) | 4ffb783 (11:24) | 35d3e1b (11:11) | Change vs same-day old |
| --- | ---: | ---: | ---: | ---: |
| 50, run 1 | 77,275 | 76,954 | 84,995 | |
| 50, run 2 | 77,302 | 77,329 | 84,391 | |
| 50, mean | 77,289 | 77,141 | 84,693 | +9.8% |
| 50, median / p99 (ms) | 0.64 / 1.05 | 0.645 / 1.055 | 0.59 / 0.955 | -8.5% / -9.5% |
| 200 | 75,118 | 74,729 | 80,754 | +8.1% |
| 400 | 74,431 | 73,604 | 81,173 | +10.3% |

### Caveats

200 and 400 are single runs per commit. Runs were not interleaved (new at 11:11,
old at 11:24). One workload only.

## 3b — pre-serialized `/api/v1` cache bodies (2026-10-26)

### Hypothesis

A warm `/api/v1` hit does three avoidable things per request today: moka
returns a deep clone of the cached entry, the handler rebuilds a
`NormalisedTransaction` through the v1 compatibility mapper, and axum
serializes it. Storing the serialized body in the cache and returning it
directly should raise `R-hit` throughput and cut latency.

### Workload

`R-hit` (v1), warm cache, 30 s runs at 50, 200 and 400 connections, run per the
procedure above. Says nothing about the cold path or writes.

### Setup

Same server (`m7i-flex.large`) and load generator (`c7i.xlarge`) as the Arc
AppState entry. The benchmark ran against the deployed API instance with
ingestion off, using the 769-URL list from the deployed database; the warm-up
run is excluded. Old build `35d3e1b`, optimized build `03bbb0d` (merged as
`05d3192`, PR #29). Four rounds: old, new, old again, new again. The repeats
check for drift; the comparison uses the mean of the initial and repeat rounds
for each build (four samples at 50 connections, two at 200 and 400).

### Before

`35d3e1b`: 86,115 req/s at 50 connections (mean of four runs), 0.58 ms median,
0.94 ms p99; 83,293 req/s at 200; 81,711 req/s at 400.

### After

`03bbb0d`: 132,082 req/s at 50 connections (mean of four runs), 0.38 ms median,
0.59 ms p99; 124,969 req/s at 200; 124,706 req/s at 400.

### Delta

+53.4% at 50 connections, +50.0% at 200 and +52.6% at 400. At 50 connections
the median fell from 0.58 to 0.38 ms and p99 from 0.94 to 0.59 ms. Every run
reported 100% success and zero non-200 responses.

### Caveat

The change bundles three effects: the deep clone of the cached entry is gone,
the v1 remap (and its string allocations) is gone, and per-hit serialization is
gone. Any gain is not attributable to serialization alone. The baseline
already contains the Arc AppState change, so the delta is relative to
`35d3e1b`, not `4ffb783` or `v0`.

### Runs

All runs reported 100% success and zero non-200s.

| Run | Commit | Connections | req/s | median latency (ms) | p99 latency (ms) | success rate | non-200 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline, initial | 35d3e1b | 50 | 85,346.6 | 0.59 | 0.94 | 100% | 0 |
| baseline, initial | 35d3e1b | 50 | 85,026.4 | 0.59 | 0.96 | 100% | 0 |
| baseline, initial | 35d3e1b | 200 | 83,189.4 | 2.38 | 4.01 | 100% | 0 |
| baseline, initial | 35d3e1b | 400 | 81,274.6 | 4.40 | 13.96 | 100% | 0 |
| optimized, initial | 03bbb0d | 50 | 132,588.2 | 0.38 | 0.59 | 100% | 0 |
| optimized, initial | 03bbb0d | 50 | 130,422.7 | 0.39 | 0.60 | 100% | 0 |
| optimized, initial | 03bbb0d | 200 | 125,208.5 | 1.57 | 2.74 | 100% | 0 |
| optimized, initial | 03bbb0d | 400 | 125,441.1 | 2.72 | 12.36 | 100% | 0 |
| baseline, repeat | 35d3e1b | 50 | 87,300.9 | 0.57 | 0.92 | 100% | 0 |
| baseline, repeat | 35d3e1b | 50 | 86,784.9 | 0.58 | 0.93 | 100% | 0 |
| baseline, repeat | 35d3e1b | 200 | 83,396.9 | 2.37 | 4.04 | 100% | 0 |
| baseline, repeat | 35d3e1b | 400 | 82,146.5 | 4.38 | 14.25 | 100% | 0 |
| optimized, repeat | 03bbb0d | 50 | 132,899.5 | 0.38 | 0.59 | 100% | 0 |
| optimized, repeat | 03bbb0d | 50 | 132,417.7 | 0.38 | 0.59 | 100% | 0 |
| optimized, repeat | 03bbb0d | 200 | 124,729.0 | 1.59 | 2.69 | 100% | 0 |
| optimized, repeat | 03bbb0d | 400 | 123,970.3 | 2.76 | 12.33 | 100% | 0 |

Mean and min–max across the repeated rounds of each build. These means are the
comparison:

| Metric | Before, `35d3e1b` (mean, min–max) | After, `03bbb0d` (mean, min–max) | Delta |
| --- | --- | --- | --- |
| 50 connections, req/s | 86,115 (85,026–87,301) | 132,082 (130,423–132,900) | +53.4% |
| 50 connections, median latency (ms) | 0.58 (0.57–0.59) | 0.38 (0.38–0.39) | −34.5% |
| 50 connections, p99 latency (ms) | 0.94 (0.92–0.96) | 0.59 (0.59–0.60) | −37.2% |
| 200 connections, req/s | 83,293 (83,189–83,397) | 124,969 (124,729–125,209) | +50.0% |
| 200 connections, median latency (ms) | 2.38 (2.37–2.38) | 1.58 (1.57–1.59) | −33.6% |
| 200 connections, p99 latency (ms) | 4.03 (4.01–4.04) | 2.72 (2.69–2.74) | −32.5% |
| 400 connections, req/s | 81,711 (81,275–82,147) | 124,706 (123,970–125,441) | +52.6% |
| 400 connections, median latency (ms) | 4.39 (4.38–4.40) | 2.74 (2.72–2.76) | −37.6% |
| 400 connections, p99 latency (ms) | 14.11 (13.96–14.25) | 12.35 (12.33–12.36) | −12.5% |

### Limited by

Server CPU. The load generator was about 66.5–78.6% idle in the old runs and
66.5–69.5% idle in the optimized runs, so it was not the bottleneck. Throughput
depends on this hardware and this workload; these numbers are not capacity
guarantees.

### Notes

The repeat rounds agree with the initial round within about 2% for the old
build and 1% for the new one, so no drift was measured. One deployed API
instance and one warm key set only.

## perf-1 — mimalloc global allocator (2026-10-27)

### Hypothesis

Warm `R-hit` throughput is server-CPU-bound. Replacing the default system
allocator with mimalloc (`mimalloc = "0.1.52"`, `#[global_allocator]` in
`src/main.rs`) may reduce allocator time and raise throughput.

### Workload

`R-hit` (v1), warm cache, 30 s runs at 50, 200 and 400 connections, run per the
procedure above. Says nothing about the cold path or writes.

### Setup

Baseline `16018712` (glibc `malloc`), after `59fe708` (mimalloc, full SHA
`59fe7086145948617e395b425e605108bdd07129`), on the same `m7i-flex.large`
server with a `c7i.xlarge` load generator. The runs were not interleaved: the
glibc runs were made before the mimalloc runs. The only change is the global
allocator. The allocator is declared in the `bridge` binary only, so
integration-test binaries still use the system allocator and benchmarks must
run the `bridge` binary.

### Before

`16018712` (glibc `malloc`): 131,033 and 130,601 req/s at 50 connections (mean
130,817), median 0.38 ms and p99 0.59 ms; 125,318 req/s at 200 connections and
123,239 at 400.

### After

`59fe708` (mimalloc): 139,755 and 138,936 req/s at 50 connections (mean
139,345), median 0.35 ms and p99 0.57 ms (runs 0.56 and 0.58); 134,973 req/s at
200 connections and 131,172 at 400.

### Delta

| Connections | `16018712` (glibc), req/s | `59fe708` (mimalloc), req/s | Change |
| ---: | ---: | ---: | ---: |
| 50 | 130,817 (runs 131,033 and 130,601) | 139,345 (runs 139,755 and 138,936) | +6.5% |
| 200 | 125,318 | 134,973 | +7.7% |
| 400 | 123,239 | 131,172 | +6.4% |

At 50 connections median latency went from 0.38 to 0.35 ms and p99 from 0.59 to
0.57 ms.

### Runs

| Run | Connections | req/s | median latency (ms) | p99 latency (ms) | success rate | non-200 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `16018712` (glibc), run 1 | 50 | 131,033 | — | — | 100% | 0 |
| `16018712` (glibc), run 2 | 50 | 130,601 | — | — | 100% | 0 |
| `16018712` (glibc) | 200 | 125,318 | — | — | 100% | 0 |
| `16018712` (glibc) | 400 | 123,239 | — | — | 100% | 0 |
| `59fe708` (mimalloc), run 1 | 50 | 139,755 | — | 0.56 | 100% | 0 |
| `59fe708` (mimalloc), run 2 | 50 | 138,936 | — | 0.58 | 100% | 0 |
| `59fe708` (mimalloc) | 200 | 134,973 | — | — | 100% | 0 |
| `59fe708` (mimalloc) | 400 | 131,172 | — | — | 100% | 0 |

`—` means not recorded. The 50-connection medians above are per-build figures
(0.38 ms for `16018712`, 0.35 ms for `59fe708`), not per-run values.

### Limited by

Server CPU. During the mimalloc run the server reported us 50.1, sy 23.1 and
id 27.1; the load generator was 70.4% idle, so it was not the bottleneck. With
glibc, server user CPU was about 57%.

### Notes

A profile at `16018712` put about 12.5% of server time in glibc allocation
(`malloc`, `_int_malloc`, `_int_free`, `malloc_consolidate`, `realloc`,
`cfree`), which is why the allocator was swapped. Caveats: one comparison run
per connection count (two at 50 connections); the glibc runs were not
interleaved with the mimalloc runs; `R-hit` only, so nothing is known about the
cold path or writes; memory use under mimalloc was not measured. Zero non-200
responses in every run.

## perf-2 — `AppState` clone behind one `Arc` (2026-10-27)

### Hypothesis

`AppState` is cloned for every axum state extraction and for each spawned
worker task. All of its fields were already cheap to clone, but each clone
still copied the whole struct and took one refcount per `Arc`-backed field.
Holding the fields in one `Inner` behind a single `Arc` makes a clone one
refcount bump.

### Workload

`R-hit` (v1), warm cache, same procedure and connection counts as perf-1. Says
nothing about the cold path or writes.

### Setup

Baseline `06b891ac4c157e4cff4881174dd84b884988b3bf` (includes mimalloc), after
`0144ea90ea3088bbcba76a0eecb6da0ebe17ebe0` (`AppState { inner: Arc<Inner> }`),
on the same `m7i-flex.large` server with a `c7i.xlarge` load generator. The
runs were interleaved in the order old, new, old, new. The only change is how
`AppState` is cloned.

### Before

`06b891a`: 142,095 req/s at 50 connections (mean of four runs: 141,846,
140,579, 142,260 and 143,697), median about 0.35 ms and p99 about 0.55 ms;
134,784 req/s at 200 connections (mean of two: 133,581 and 135,986) and 130,376
at 400 (mean of two: 129,347 and 131,404).

### After

`0144ea9`: 156,819 req/s at 50 connections (mean of four runs: 155,643,
156,057, 156,984 and 158,590), median about 0.31 ms and p99 about 0.50 ms;
147,438 req/s at 200 connections (mean of two: 145,878 and 148,998) and 145,689
at 400 (mean of two: 146,457 and 144,921).

### Delta

| Connections | `06b891a`, req/s | `0144ea9`, req/s | Change |
| ---: | ---: | ---: | ---: |
| 50 | 142,095 (141,846, 140,579, 142,260 and 143,697) | 156,819 (155,643, 156,057, 156,984 and 158,590) | +10.4% |
| 200 | 134,784 (133,581 and 135,986) | 147,438 (145,878 and 148,998) | +9.4% |
| 400 | 130,376 (129,347 and 131,404) | 145,689 (146,457 and 144,921) | +11.7% |

### Runs

| Run | Connections | req/s | median latency (ms) | p99 latency (ms) | success rate | non-200 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `06b891a`, run 1 | 50 | 141,846 | — | — | 100% | 0 |
| `0144ea9`, run 1 | 50 | 155,643 | — | — | 100% | 0 |
| `06b891a`, run 2 | 50 | 140,579 | — | — | 100% | 0 |
| `0144ea9`, run 2 | 50 | 156,057 | — | — | 100% | 0 |
| `06b891a`, run 3 | 50 | 142,260 | — | — | 100% | 0 |
| `0144ea9`, run 3 | 50 | 156,984 | — | — | 100% | 0 |
| `06b891a`, run 4 | 50 | 143,697 | — | — | 100% | 0 |
| `0144ea9`, run 4 | 50 | 158,590 | — | — | 100% | 0 |
| `06b891a`, run 1 | 200 | 133,581 | — | — | 100% | 0 |
| `0144ea9`, run 1 | 200 | 145,878 | — | — | 100% | 0 |
| `06b891a`, run 2 | 200 | 135,986 | — | — | 100% | 0 |
| `0144ea9`, run 2 | 200 | 148,998 | — | — | 100% | 0 |
| `06b891a`, run 1 | 400 | 129,347 | — | — | 100% | 0 |
| `0144ea9`, run 1 | 400 | 146,457 | — | — | 100% | 0 |
| `06b891a`, run 2 | 400 | 131,404 | — | — | 100% | 0 |
| `0144ea9`, run 2 | 400 | 144,921 | — | — | 100% | 0 |

`—` means not recorded. The 50-connection medians and p99s above are per-build
approximate values (about 0.35 and 0.55 ms for `06b891a`; about 0.31 and
0.50 ms for `0144ea9`), not per-run values.

### Limited by

Server CPU. Server and load generator CPU percentages were not recorded for
these rounds.

### Notes

The reason for the change: a profile at `16018712` showed about 4% of server
time in `drop_glue<AppState>` and `FromRef::from_ref`. The old `AppState` clone
did about 8 refcount increments per request; the new one does 1.

The measured gain (+9.4% to +11.7%) is larger than that 4%. A likely
explanation, stated as a hypothesis rather than a measured fact: the removed
atomic refcount operations probably also cause cache-line contention between
cores, which a CPU profile does not attribute cleanly.

Caveats: the runs were interleaved old/new/old/new; 200 and 400 connections
have two runs per version; `R-hit` only, so nothing is known about the cold
path or writes; server and load generator CPU were not recorded for these
rounds; the old baseline already includes mimalloc; the tradeoff is one extra
pointer hop per field access through `Deref`. At 50 connections the old and new
ranges do not overlap (140.6k–143.7k against 155.6k–158.6k), and the old runs
agree with each other within about 1.3%. Zero non-200 responses in every run.

## 3c — pre-serialized `/api/v2` response bodies (2026-10-27)

### Hypothesis

A warm `/api/v2` hit already costs one cache lookup (`v2_completed`, then
`v2_open` on a miss), so the roadmap's "one lookup instead of four" no longer
applies. The remaining per-hit cost was the deep clone of the cached
`TransactionResponse` and its JSON serialization on every request. Storing the
serialized body in the cache and returning it directly, as 3b did for
`/api/v1`, should raise `R-hit` throughput and cut latency.

### Workload

`R-hit` (`v2`), warm cache, 30 s runs at 50, 200 and 400 connections, run per
the procedure above. One `/api/v1` pass on each build, at 50 connections,
served as a control. Says nothing about the cold path or writes.

### Setup

Old build `5a2fce3`, new build `0ed3edb`. The runs were interleaved in the
order old, new, old, new.

### Before

`5a2fce3`: 86,660 req/s at 50 connections (mean of four runs), 0.58 ms median,
0.82 ms p99; 82,852 req/s at 200 connections and 80,290 at 400.

### After

`0ed3edb`: 128,872 req/s at 50 connections (mean of four runs), 0.39 ms median,
0.53 ms p99; 127,498 req/s at 200 connections and 121,888 at 400.

### Delta

| Connections | `5a2fce3`, req/s | `0ed3edb`, req/s | Change |
| ---: | ---: | ---: | ---: |
| 50 | 86,660 (mean of 4) | 128,872 (mean of 4) | +48.7% |
| 200 | 82,852 | 127,498 | +53.9% |
| 400 | 80,290 | 121,888 | +51.8% |

At 50 connections the median fell from 0.58 to 0.39 ms and p99 from 0.82 to
0.53 ms.

### Control

A `/api/v1` pass on each build on the same day, at 50 connections, gave
127.9k req/s on `5a2fce3` and 128.2k on `0ed3edb`, so there is no `v1`
regression from this change. The control is one pass per build.

### Limited by

CPU is not comparable: the old1 sampler ran too long. No CPU numbers are
reported.

### Notes

Caveats: the runs were interleaved old/new/old/new; 200 and 400 connections
have two runs per version; `R-hit` only, so nothing is known about the cold
path or writes; the `/api/v1` control is one pass per build. The tradeoff is
one more serialized body per cached entry in memory (up to 50k completed
entries), plus one more serialization on a v2-only miss.
