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
