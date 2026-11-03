#!/usr/bin/env bash
# Benchmark GET /api/v2/transfers (roadmap 3g) against the deployed list server.
#
# Workloads (all three must meet 300 req/s and p99 < 150 ms on the EC2 box;
# only 2xx counts as success):
#   first     /api/v2/transfers?limit=50
#   deep      the same with DEEP_CURSOR, at roughly 80% depth
#   filtered  direction=Inbound&status=Completed&chain=ethereum, limit=50
#
# Each workload is run twice: run 1 is the cold run, run 2 the warm run. The
# JSON for each is kept separately under bench-results/<timestamp>/ and is never
# averaged together. "Cold" here means the server and database have not served
# this query since they started; to make run 1 genuinely cold, restart the API
# and Postgres (or drop the database page cache) first. The script cannot do
# that for you, and it records whether you said the run was cold.
#
# oha's JSON output is kept under bench-results/<timestamp>/ (ignored through
# .git/info/exclude) so the numbers can be checked later.
set -euo pipefail

readonly CONNS=50
readonly LIST_DURATION=30s

BASE_URL=${BASE_URL:-}
SERVER_SHA=${SERVER_SHA:-}
INSTANCE_TYPE=${INSTANCE_TYPE:-}
LOADGEN_INSTANCE=${LOADGEN_INSTANCE:-}
DEEP_CURSOR=${DEEP_CURSOR:-}
FILTERED_QUERY=${FILTERED_QUERY:-direction=Inbound\&status=Completed\&chain=ethereum\&limit=50}
RESULTS_ROOT=${BENCH_RESULTS_DIR:-bench-results}
COLD_START=${COLD_START:-no}
DRY_RUN=0
DURATION=$LIST_DURATION

usage() {
  cat <<'EOF'
Usage: scripts/bench_list.sh [options]

Runs first/deep/filtered page against GET /api/v2/transfers with oha and
prints a markdown summary table, one row per run. Never merges the cold and
warm runs.

Environment (required unless an option is given):
  BASE_URL          API origin, e.g. http://172.31.38.190:8080
  SERVER_SHA        server git revision
  INSTANCE_TYPE     EC2 instance type of the API server
  DEEP_CURSOR       value from the first page's next_cursor for the deep page

Environment (optional):
  LOADGEN_INSTANCE  instance type of the load generator (recorded in meta)
  FILTERED_QUERY    filter query string (default:
                    direction=Inbound&status=Completed&chain=ethereum&limit=50)
  COLD_START        yes|no (default no): whether run 1 was made cold by
                    restarting the API and Postgres or dropping caches
  BENCH_RESULTS_DIR output root (default: bench-results)

Options:
  --base-url URL        override BASE_URL
  --server-sha SHA      override SERVER_SHA
  --instance-type TYPE  override INSTANCE_TYPE
  --deep-cursor VALUE   override DEEP_CURSOR
  --filtered-query Q    override FILTERED_QUERY (with or without '?')
  --cold-start          mark run 1 as cold (and check you meant it)
  --dry-run             use 2 s per run and write under bench-results/dry-run-*
  -h, --help            show this help

The script first sends one curl request per workload and stops unless each
answers 200 with a JSON body containing an items array. It requires oha 1.16.0
(or any oha with --output-format json), jq and curl.

WARNING: point this only at the deployed API server for this test, never at a
shared or production URL.
EOF
}

die() {
  printf 'bench_list.sh: %s\n' "$*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base-url)
      [[ $# -ge 2 ]] || die "--base-url needs a value"
      BASE_URL=$2
      shift 2
      ;;
    --server-sha)
      [[ $# -ge 2 ]] || die "--server-sha needs a value"
      SERVER_SHA=$2
      shift 2
      ;;
    --instance-type)
      [[ $# -ge 2 ]] || die "--instance-type needs a value"
      INSTANCE_TYPE=$2
      shift 2
      ;;
    --deep-cursor)
      [[ $# -ge 2 ]] || die "--deep-cursor needs a value"
      DEEP_CURSOR=$2
      shift 2
      ;;
    --filtered-query)
      [[ $# -ge 2 ]] || die "--filtered-query needs a value"
      FILTERED_QUERY=$2
      shift 2
      ;;
    --cold-start)
      COLD_START=yes
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      DURATION=2s
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    -*)
      die "unknown option: $1 (try --help)"
      ;;
    *)
      die "unexpected argument: $1 (try --help)"
      ;;
  esac
done

command -v oha >/dev/null 2>&1 || die "oha is required and was not found in PATH"
command -v jq >/dev/null 2>&1 || die "jq is required to read oha's JSON output"
command -v curl >/dev/null 2>&1 || die "curl is required for the pre-run status check"

[[ -n "$BASE_URL" ]] || die "BASE_URL is not set (use --base-url or the environment)"
[[ -n "$SERVER_SHA" ]] || die "SERVER_SHA is not set: every row must record the server revision"
[[ -n "$INSTANCE_TYPE" ]] || die "INSTANCE_TYPE is not set: every row must record the server instance type"
[[ -n "$DEEP_CURSOR" ]] || die "DEEP_CURSOR is not set: record the deep page's cursor from the first page first (see README)"
case "$COLD_START" in
  yes | no) ;;
  *) die "COLD_START must be yes or no, got: $COLD_START" ;;
esac

while [[ "$BASE_URL" == */ ]]; do
  BASE_URL=${BASE_URL%/}
done
case "$BASE_URL" in
  http://* | https://*) ;;
  *) die "BASE_URL must start with http:// or https://, got: $BASE_URL" ;;
esac

OHA_VERSION=$(oha --version 2>&1 || true)
OHA_VERSION=${OHA_VERSION%%$'\n'*}
[[ -n "$OHA_VERSION" ]] || die "could not read the oha version"
OHA_HELP=$(oha --help 2>&1 || true)
grep -q -- '--output-format' <<<"$OHA_HELP" ||
  die "$OHA_VERSION does not support --output-format json"

# 50 connections plus keep-alive still needs a raised descriptor limit on some
# hosts; match the other benchmark scripts.
ulimit -n 65535 2>/dev/null || true

query_string() {
  local q=$1
  if [[ "$q" == \?* ]]; then
    printf '%s' "$q"
  else
    printf '?%s' "$q"
  fi
}

readonly FIRST_URL="$BASE_URL/api/v2/transfers?limit=50"
readonly DEEP_URL="$BASE_URL/api/v2/transfers?limit=50&cursor=$DEEP_CURSOR"
readonly FILTERED_URL="$BASE_URL/api/v2/transfers$(query_string "$FILTERED_QUERY")"

check_url() {
  local label=$1
  local url=$2
  local body status
  body=$(mktemp)
  status=$(curl -sS -m 10 -o "$body" -w '%{http_code}' "$url" || true)
  if [[ "$status" != "200" ]]; then
    printf 'bench_list.sh: %s pre-check returned HTTP %s\n' "$label" "$status" >&2
    head -c 500 "$body" >&2 || true
    printf '\n' >&2
    rm -f -- "$body"
    exit 1
  fi
  if ! jq -e '.items | type == "array"' "$body" >/dev/null 2>&1; then
    printf 'bench_list.sh: %s pre-check returned 200 without an items array\n' "$label" >&2
    head -c 500 "$body" >&2 || true
    printf '\n' >&2
    rm -f -- "$body"
    exit 1
  fi
  LAST_ITEMS=$(jq -r '.items | length' "$body")
  printf 'bench_list.sh: %s pre-check ok (%s items)\n' "$label" "$LAST_ITEMS" >&2
  rm -f -- "$body"
}

check_url first "$FIRST_URL"
[[ "$LAST_ITEMS" -gt 0 ]] ||
  die "the first page is empty on $BASE_URL; seed the database before benchmarking"
check_url deep "$DEEP_URL"
[[ "$LAST_ITEMS" -gt 0 ]] ||
  die "the deep cursor page is empty; choose a cursor nearer the newest rows"
check_url filtered "$FILTERED_URL"
[[ "$LAST_ITEMS" -gt 0 ]] ||
  die "the filtered page is empty; choose a filter present in the seeded data"

stamp=$(date -u +%Y%m%dT%H%M%SZ)
if [[ "$DRY_RUN" -eq 1 ]]; then
  run_dir="$RESULTS_ROOT/dry-run-$stamp"
else
  run_dir="$RESULTS_ROOT/$stamp"
fi
[[ ! -e "$run_dir" ]] || run_dir="${run_dir}-$$"
mkdir -p -- "$run_dir"

{
  printf 'date_utc: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'base_url: %s\n' "$BASE_URL"
  printf 'server_sha: %s\n' "$SERVER_SHA"
  printf 'instance_type: %s\n' "$INSTANCE_TYPE"
  printf 'loadgen_instance: %s\n' "${LOADGEN_INSTANCE:-not provided}"
  printf 'cold_start: %s\n' "$COLD_START"
  printf 'oha: %s\n' "$OHA_VERSION"
  printf 'duration_per_run: %s\n' "$DURATION"
  printf 'connections: %s\n' "$CONNS"
  printf 'first_url: %s\n' "$FIRST_URL"
  printf 'deep_url: %s\n' "$DEEP_URL"
  printf 'filtered_url: %s\n' "$FILTERED_URL"
  printf 'warmup: none; run 1 and run 2 are both measured, cold and warm kept separate\n'
  printf 'host: %s\n' "$(uname -n)"
  printf 'uname: %s\n' "$(uname -a)"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf 'note: DRY RUN, not a 3g result\n'
  fi
} >"$run_dir/meta.txt"

cat >&2 <<EOF
WARNING: point this only at the deployed API server for this test.
Target:      $BASE_URL
Server SHA:  $SERVER_SHA
Instance:    $INSTANCE_TYPE
Cold start:  $COLD_START
Duration:    $DURATION per run
Results:     $run_dir
EOF

run_oha() {
  local label=$1
  local url=$2
  local filename=$3

  printf 'bench_list.sh: %s (%s connections, %s)\n' "$label" "$CONNS" "$DURATION" >&2
  printf 'oha -z %q -c %q --no-tui --output-format json %q -o %q\n' \
    "$DURATION" "$CONNS" "$url" "$run_dir/$filename" >>"$run_dir/commands.txt"

  oha -z "$DURATION" \
    -c "$CONNS" \
    --no-tui \
    --output-format json \
    "$url" \
    -o "$run_dir/$filename"
}

run_oha "first/cold" "$FIRST_URL" "01-first-cold.json"
run_oha "first/warm" "$FIRST_URL" "02-first-warm.json"
run_oha "deep/cold" "$DEEP_URL" "03-deep-cold.json"
run_oha "deep/warm" "$DEEP_URL" "04-deep-warm.json"
run_oha "filtered/cold" "$FILTERED_URL" "05-filtered-cold.json"
run_oha "filtered/warm" "$FILTERED_URL" "06-filtered-warm.json"

# One row of the summary. Success is 2xx only: a 4xx or 5xx is not a success.
extract_row() {
  local file=$1
  jq -r '
    def rps: (.metrics.requests_per_sec // .summary.requestsPerSec // 0);
    def p50: (.metrics.latency_ms.p50 // ((.latencyPercentiles.p50 // 0) * 1000));
    def p99: (.metrics.latency_ms.p99 // ((.latencyPercentiles.p99 // 0) * 1000));
    def dist: (.statusCodeDistribution // {});
    def ok: ([dist | to_entries[]? | select((.key | tonumber) >= 200 and (.key | tonumber) < 300) | .value] | add // 0);
    def four: ([dist | to_entries[]? | select((.key | tonumber) >= 400 and (.key | tonumber) < 500) | .value] | add // 0);
    def five: ([dist | to_entries[]? | select((.key | tonumber) >= 500) | .value] | add // 0);
    def errors: ([dist | to_entries[]? | select((.key | tonumber) < 200 or (.key | tonumber) >= 300) | .value] | add // 0);
    [rps, p50, p99, ok, four, five, errors] | @tsv
  ' "$file"
}

printf '\n## 3g list endpoint results\n\n'
printf '| Workload | Run | Connections | req/s | median (ms) | p99 (ms) | 2xx | 4xx | 5xx | non-2xx |\n'
printf '| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |\n'

labels=("first" "first" "deep" "deep" "filtered" "filtered")
runs=("cold" "warm" "cold" "warm" "cold" "warm")
files=("01-first-cold.json" "02-first-warm.json" "03-deep-cold.json" \
  "04-deep-warm.json" "05-filtered-cold.json" "06-filtered-warm.json")

for i in "${!labels[@]}"; do
  row=$(extract_row "$run_dir/${files[$i]}") ||
    die "could not parse $run_dir/${files[$i]}"
  IFS=$'\t' read -r rps p50 p99 ok four five errors <<<"$row"
  printf '| %s | %s | %s | %.1f | %.2f | %.2f | %s | %s | %s | %s |\n' \
    "${labels[$i]}" "${runs[$i]}" "$CONNS" "$rps" "$p50" "$p99" \
    "$ok" "$four" "$five" "$errors"
done

printf '\n'
printf 'Target: 300 req/s and p99 < 150 ms on the EC2 box, 2xx only counted as success.\n'
printf 'Cold and warm runs are listed separately; do not average them together.\n'
printf 'Server SHA: %s   instance: %s   cold_start: %s\n' \
  "$SERVER_SHA" "$INSTANCE_TYPE" "$COLD_START"
printf 'JSON and meta kept at %s/\n' "$run_dir"
