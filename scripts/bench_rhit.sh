#!/usr/bin/env bash
# Run the R-hit benchmark series against the deployed API server.
#
# Series (README "Workloads"):
#   1. 50 connections, 30 s   warm-up, excluded from the summary
#   2. 50 connections, 30 s
#   3. 50 connections, 30 s
#   4. 200 connections, 30 s
#   5. 400 connections, 30 s
#
# oha's JSON output is kept under bench-results/<timestamp>/ (ignored through
# .git/info/exclude) so the numbers can be checked later.
set -euo pipefail

readonly WARMUP_CONNS=50
readonly HOT_CONNS=50
readonly MID_CONNS=200
readonly MAX_CONNS=400
readonly R_HIT_DURATION=30s
readonly EXPECTED_URLS=769

BASE_URL=${BASE_URL:-}
URLS_FILE=${URLS_FILE:-}
SERVER_SHA=${SERVER_SHA:-}
RESULTS_ROOT=${BENCH_RESULTS_DIR:-bench-results}
DRY_RUN=0
DURATION=$R_HIT_DURATION

usage() {
  cat <<'EOF'
Usage: scripts/bench_rhit.sh [options] [URLS_FILE] [SERVER_SHA]

Runs the R-hit series with oha and prints a markdown summary table.
If --urls (or URLS_FILE in the environment) already names the file, a single
bare argument is SERVER_SHA.

Environment:
  BASE_URL            API origin the URLs were built for, e.g.
                      http://172.31.38.190:8080 (required)
  URLS_FILE           file from make_urls.sh (required unless positional)
  SERVER_SHA          server revision to record (optional; positional allowed)
  BENCH_RESULTS_DIR   output root (default: bench-results)

Options:
  --base-url URL      override BASE_URL
  --urls FILE         override URLS_FILE
  --server-sha SHA    record the server revision
  --dry-run           use 2 s per run and write under bench-results/dry-run-*
                      (only for testing against a local fake server)
  -h, --help          show this help

oha 1.16.0 writes JSON with --output-format json. The script checks for that
flag and stops with a clear message if the installed oha differs.

WARNING: the first run fills the server cache and spends Helius quota. Point
this only at the deployed API server for this test, never at a shared or
production URL.
EOF
}

die() {
  printf 'bench_rhit.sh: %s\n' "$*" >&2
  exit 1
}

positionals=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --base-url)
      [[ $# -ge 2 ]] || die "--base-url needs a value"
      BASE_URL=$2
      shift 2
      ;;
    --urls | --urls-file)
      [[ $# -ge 2 ]] || die "$1 needs a value"
      URLS_FILE=$2
      shift 2
      ;;
    --server-sha)
      [[ $# -ge 2 ]] || die "--server-sha needs a value"
      SERVER_SHA=$2
      shift 2
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
      positionals+=("$1")
      shift
      ;;
  esac
done

# Bare arguments are [URLS_FILE] [SERVER_SHA]. When --urls (or URLS_FILE in the
# environment) already named the file, a single bare argument is SERVER_SHA.
if [[ ${#positionals[@]} -gt 0 ]]; then
  if [[ -z "$URLS_FILE" ]]; then
    URLS_FILE=${positionals[0]}
    if [[ ${#positionals[@]} -gt 1 ]]; then
      SERVER_SHA=${positionals[1]}
    fi
    if [[ ${#positionals[@]} -gt 2 ]]; then
      die "too many positional arguments (try --help)"
    fi
  else
    SERVER_SHA=${positionals[0]}
    if [[ ${#positionals[@]} -gt 1 ]]; then
      die "too many positional arguments when --urls is already set (try --help)"
    fi
  fi
fi

command -v oha >/dev/null 2>&1 || die "oha is required and was not found in PATH"
command -v jq >/dev/null 2>&1 || die "jq is required to read oha's JSON output"
command -v mktemp >/dev/null 2>&1 || die "mktemp is required"

[[ -n "$BASE_URL" ]] ||
  die "BASE_URL is not set (use --base-url or the environment)"
[[ -n "$URLS_FILE" ]] ||
  die "URLS_FILE is not set (pass it positionally or with --urls)"
[[ -f "$URLS_FILE" ]] || die "URLs file does not exist: $URLS_FILE"
[[ -r "$URLS_FILE" ]] || die "URLs file is not readable: $URLS_FILE"

while [[ "$BASE_URL" == */ ]]; do
  BASE_URL=${BASE_URL%/}
done
case "$BASE_URL" in
  http://* | https://*) ;;
  *) die "BASE_URL must start with http:// or https://, got: $BASE_URL" ;;
esac

# Refuse to run if the URLs point somewhere other than BASE_URL: the file
# carries the host, and a stale list would silently benchmark a different box.
if ! awk -v prefix="$BASE_URL/" '
  NF && index($0, prefix) != 1 { exit 1 }
  END { if (NR == 0) exit 1 }
' "$URLS_FILE"; then
  die "every URL in $URLS_FILE must start with $BASE_URL/ (regenerate it with make_urls.sh)"
fi

url_count=$(grep -cve '^[[:space:]]*$' "$URLS_FILE" || true)
url_count=${url_count//[[:space:]]/}
[[ "$url_count" -gt 0 ]] || die "URLs file is empty: $URLS_FILE"
if [[ "$url_count" -ne "$EXPECTED_URLS" ]]; then
  printf 'bench_rhit.sh: note: R-hit is defined with %s URLs; %s has %s\n' \
    "$EXPECTED_URLS" "$URLS_FILE" "$url_count" >&2
fi

OHA_VERSION=$(oha --version 2>&1 || true)
OHA_VERSION=${OHA_VERSION%%$'\n'*}
[[ -n "$OHA_VERSION" ]] || die "could not read the oha version"

# Capture the help once: piping oha into grep -q under pipefail can make oha
# die with SIGPIPE after the first match.
OHA_HELP=$(oha --help 2>&1 || true)
if ! grep -q -- '--output-format' <<<"$OHA_HELP"; then
  die "$OHA_VERSION does not support --output-format. oha 1.16.0 uses '--output-format json'; update the script or the installed oha"
fi
if ! grep -q -- '--urls-from-file' <<<"$OHA_HELP"; then
  die "$OHA_VERSION does not support --urls-from-file; update the installed oha"
fi

# 400 connections plus keep-alive needs more descriptors than the usual 1024.
# Raise the soft limit before any run starts, and stop if it did not take.
if ! ulimit -n 65535 2>/dev/null; then
  die "could not raise the open-file limit to 65535 (soft=$(ulimit -n), hard=$(ulimit -Hn)). Raise the hard limit (for example as root, or in /etc/security/limits.conf) and retry"
fi
open_files=$(ulimit -n)
if [[ "$open_files" -lt 65535 ]]; then
  die "open-file limit is $open_files after 'ulimit -n 65535'; 65535 is required"
fi

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
  printf 'urls_file: %s\n' "$URLS_FILE"
  printf 'urls: %s\n' "$url_count"
  printf 'server_sha: %s\n' "${SERVER_SHA:-not provided}"
  printf 'oha: %s\n' "$OHA_VERSION"
  printf 'duration_per_run: %s\n' "$DURATION"
  printf 'open_file_limit: %s\n' "$open_files"
  printf 'warmup: first %s-connection run, excluded from the summary table\n' "$WARMUP_CONNS"
  printf 'host: %s\n' "$(uname -n)"
  printf 'uname: %s\n' "$(uname -a)"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf 'note: DRY RUN, not an R-hit result\n'
  fi
} >"$run_dir/meta.txt"

cat >&2 <<EOF
WARNING: the first run fills the server cache and spends Helius quota.
Run this only against the deployed API server for this test, never against a
shared or production URL.

Target:     $BASE_URL
URLs file:  $URLS_FILE ($url_count lines)
Duration:   $DURATION per run
Results:    $run_dir
EOF
if [[ "$DRY_RUN" -eq 1 ]]; then
  printf 'bench_rhit.sh: DRY RUN with %s per run; these numbers are not R-hit results\n' \
    "$DURATION" >&2
fi

run_oha() {
  local label=$1
  local connections=$2
  local filename=$3

  printf 'bench_rhit.sh: %s (%s connections, %s)\n' \
    "$label" "$connections" "$DURATION" >&2
  printf 'oha -z %q -c %q --no-tui --output-format json --urls-from-file %q -o %q\n' \
    "$DURATION" "$connections" "$URLS_FILE" "$run_dir/$filename" \
    >>"$run_dir/commands.txt"

  oha -z "$DURATION" \
    -c "$connections" \
    --no-tui \
    --output-format json \
    --urls-from-file "$URLS_FILE" \
    -o "$run_dir/$filename"
}

run_oha "warm-up (excluded)" "$WARMUP_CONNS" "01-warmup-50c.json"
run_oha "50c run 1" "$HOT_CONNS" "02-50c-run-1.json"
run_oha "50c run 2" "$HOT_CONNS" "03-50c-run-2.json"
run_oha "200c" "$MID_CONNS" "04-200c.json"
run_oha "400c" "$MAX_CONNS" "05-400c.json"

# One row of the summary. oha 1.16 exposes both the older camelCase summary
# and the newer metrics/latency_ms block; prefer the latter and fall back.
extract_row() {
  local file=$1
  jq -r '
    def rps: (.metrics.requests_per_sec // .summary.requestsPerSec // 0);
    def p50: (.metrics.latency_ms.p50 // ((.latencyPercentiles.p50 // 0) * 1000));
    def p99: (.metrics.latency_ms.p99 // ((.latencyPercentiles.p99 // 0) * 1000));
    def success: ((.metrics.success_rate // .summary.successRate // 0) * 100);
    def non200: (([(.statusCodeDistribution // {}) | to_entries[]?
                  | select(.key != "200") | .value] | add) // 0);
    [rps, p50, p99, success, non200] | @tsv
  ' "$file"
}

printf '\n## R-hit results\n\n'
printf '| Run | Connections | req/s | median (ms) | p99 (ms) | success | non-200 |\n'
printf '| --- | ---: | ---: | ---: | ---: | ---: | ---: |\n'

labels=("50c run 1" "50c run 2" "200c" "400c")
conns=("$HOT_CONNS" "$HOT_CONNS" "$MID_CONNS" "$MAX_CONNS")
files=("02-50c-run-1.json" "03-50c-run-2.json" "04-200c.json" "05-400c.json")

for i in "${!labels[@]}"; do
  row=$(extract_row "$run_dir/${files[$i]}") ||
    die "could not parse $run_dir/${files[$i]}"
  IFS=$'\t' read -r rps p50 p99 success non200 <<<"$row"
  printf '| %s | %s | %.1f | %.2f | %.2f | %.2f%% | %s |\n' \
    "${labels[$i]}" "${conns[$i]}" "$rps" "$p50" "$p99" "$success" "$non200"
done

printf '\n'
printf 'Warm-up excluded from the table; JSON kept at %s/01-warmup-50c.json.\n' "$run_dir"
printf 'oha success rate counts transport errors, not non-2xx responses; use the non-200 column for HTTP failures.\n'
printf 'Results directory: %s\n' "$run_dir"
