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

`R-hit` (v1), warm cache, 50 connections, 30 s, run per the procedure above.
Says nothing about the cold path or writes.

### Setup

Same server and load generator as the Arc AppState entry. Fill in the commit
and run metadata from `bench-results/<timestamp>/`.

### Before

### After

### Delta

### Caveat

The change bundles three effects: the deep clone of the cached entry is gone,
the v1 remap (and its string allocations) is gone, and per-hit serialization is
gone. Any gain is not attributable to serialization alone.

### Runs

| Run | Connections | req/s | median latency (ms) | p99 latency (ms) | success rate | non-200 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
|  |  |  |  |  |  |  |

Median and min–max across the repeated runs:

| Metric | Before (median, min–max) | After (median, min–max) | Delta |
| --- | --- | --- | --- |
|  |  |  |  |

### Limited by

### Notes
