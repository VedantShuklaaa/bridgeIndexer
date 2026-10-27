#!/usr/bin/env bash
# Cold-path baseline for 3e: R-cold and R-mix against the fake upstream.
#
# Workloads (README "Workloads"):
#   R-cold   all-unique signatures, 50 connections, 30 s
#   R-mix    90% hot (a fixed, pre-warmed set) / 10% fresh cold, 50 connections, 30 s
#
# Why a redirector: oha picks URLs from --urls-from-file with replacement, so a
# finite list of cold signatures would turn repeated URLs into cache hits and
# quietly stop being R-cold. Every cold entry in the URL file is therefore a
# local 302 that hands out the next signature from a freshly generated pool.
# Hot entries go straight to the API. One local redirect hop is added to cold
# requests only (about 0.1-0.3 ms against a 50 ms upstream); it is included in
# the measured latency and is noted in the output.
#
# Both workloads run oha with --disable-keepalive. Its HTTP/1 client keeps one
# pooled connection per worker and sends the next request over it whatever origin
# that request targets (client.rs: `client_state.send_request`), so in R-mix a
# /cold request would be sent to the API origin and answered 404. Without
# keep-alive every request dials its own origin, which is also the connection
# behaviour R-cold gets when following its 302. The cost is a loopback TCP
# handshake per request, included in the measured latency.
#
# The app and the fake upstream are started outside this script, the same way
# scripts/bench_rhit.sh expects the API server to be running. Point the app's
# HELIUS_URL/WORMHOLE_URL at the fake before running this.
set -euo pipefail

readonly DEFAULT_HOT_COUNT=769
readonly DEFAULT_COLD_POOL=100000
readonly DEFAULT_CONNECTIONS=50
readonly DEFAULT_DURATION=30s
readonly DEFAULT_RUNS=2
readonly DEFAULT_REDIRECT_PORT=18082

BASE_URL=${BASE_URL:-}
FAKE_URL=${FAKE_URL:-http://127.0.0.1:9090}
LABEL=${LABEL:-default}
API_VERSION=${API_VERSION:-v2}
RUNS=${RUNS:-$DEFAULT_RUNS}
DURATION=${DURATION:-$DEFAULT_DURATION}
CONNECTIONS=${CONNECTIONS:-$DEFAULT_CONNECTIONS}
COLD_POOL=${COLD_POOL:-$DEFAULT_COLD_POOL}
HOT_COUNT=${HOT_COUNT:-$DEFAULT_HOT_COUNT}
WARM_PARALLEL=${WARM_PARALLEL:-4}
REDIRECT_PORT=${REDIRECT_PORT:-$DEFAULT_REDIRECT_PORT}
RESULTS_ROOT=${BENCH_RESULTS_DIR:-bench-results}
DRY_RUN=0

usage() {
  cat <<'EOF'
Usage: scripts/bench_cold.sh --base-url URL [options]

Runs R-cold twice and R-mix twice (or --runs N each) with oha and prints one
block per run. The API server and the fake upstream must already be running;
the app must point HELIUS_URL and WORMHOLE_URL at the fake.

Environment:
  BASE_URL          API origin, e.g. http://127.0.0.1:8081 (required)
  FAKE_URL          fake upstream origin (default: http://127.0.0.1:9090)
  LABEL             config label recorded in the results directory (default: default)
  API_VERSION       v1 or v2 (default: v2)
  RUNS              runs per workload (default: 2)
  DURATION          oha duration per run (default: 30s)
  CONNECTIONS       oha connections (default: 50)
  COLD_POOL         fresh signatures generated per run (default: 100000)
  HOT_COUNT         fixed pre-warmed signatures for R-mix (default: 769)
  WARM_PARALLEL     concurrent warm-up requests (default: 4)
  REDIRECT_PORT     local port for the cold-signature redirector (default: 18082)
  BENCH_RESULTS_DIR output root (default: bench-results)

Options:
  --base-url URL     override BASE_URL
  --fake-url URL     override FAKE_URL
  --label TEXT       override LABEL
  --api-version V    v1 or v2
  --runs N           runs per workload
  --duration D       oha duration per run
  --connections N    oha connections
  --cold-pool N      signatures per cold pool
  --hot-count N      fixed hot signatures
  --warm-parallel N  concurrent warm-up requests
  --redirect-port N  local redirector port
  --dry-run          short runs (2 s, 500 cold, 20 hot) for checking the harness;
                     never report these numbers as a baseline
  -h, --help         show this help

For every run the script resets the fake's counters, runs oha, reads the fake's
/stats and prints req/s, median, p99, the status distribution, the upstream call
counts per cold request, and the in-flight high-water mark. oha's JSON and the
fake's stats are kept under the results directory.
EOF
}

die() {
  printf 'bench_cold.sh: %s\n' "$*" >&2
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
    --fake-url)
      [[ $# -ge 2 ]] || die "--fake-url needs a value"
      FAKE_URL=$2
      shift 2
      ;;
    --label)
      [[ $# -ge 2 ]] || die "--label needs a value"
      LABEL=$2
      shift 2
      ;;
    --api-version)
      [[ $# -ge 2 ]] || die "--api-version needs a value"
      API_VERSION=$2
      shift 2
      ;;
    --runs)
      [[ $# -ge 2 ]] || die "--runs needs a value"
      RUNS=$2
      shift 2
      ;;
    --duration)
      [[ $# -ge 2 ]] || die "--duration needs a value"
      DURATION=$2
      shift 2
      ;;
    --connections)
      [[ $# -ge 2 ]] || die "--connections needs a value"
      CONNECTIONS=$2
      shift 2
      ;;
    --cold-pool)
      [[ $# -ge 2 ]] || die "--cold-pool needs a value"
      COLD_POOL=$2
      shift 2
      ;;
    --hot-count)
      [[ $# -ge 2 ]] || die "--hot-count needs a value"
      HOT_COUNT=$2
      shift 2
      ;;
    --warm-parallel)
      [[ $# -ge 2 ]] || die "--warm-parallel needs a value"
      WARM_PARALLEL=$2
      shift 2
      ;;
    --redirect-port)
      [[ $# -ge 2 ]] || die "--redirect-port needs a value"
      REDIRECT_PORT=$2
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      DURATION=2s
      COLD_POOL=500
      HOT_COUNT=20
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

[[ ${#positionals[@]} -eq 0 ]] ||
  die "unexpected positional argument: ${positionals[0]} (try --help)"

command -v oha >/dev/null 2>&1 || die "oha is required and was not found in PATH"
command -v jq >/dev/null 2>&1 || die "jq is required"
command -v curl >/dev/null 2>&1 || die "curl is required"
command -v python3 >/dev/null 2>&1 || die "python3 is required"
command -v mktemp >/dev/null 2>&1 || die "mktemp is required"
command -v xargs >/dev/null 2>&1 || die "xargs is required"

[[ -n "$BASE_URL" ]] || die "BASE_URL is not set (use --base-url or the environment)"
case "$API_VERSION" in
  v1 | v2) ;;
  *) die "API_VERSION must be v1 or v2, got: $API_VERSION" ;;
esac
[[ "$RUNS" =~ ^[1-9][0-9]*$ ]] || die "RUNS must be a positive integer, got: $RUNS"
[[ "$CONNECTIONS" =~ ^[1-9][0-9]*$ ]] || die "CONNECTIONS must be a positive integer, got: $CONNECTIONS"
[[ "$COLD_POOL" =~ ^[1-9][0-9]*$ ]] || die "COLD_POOL must be a positive integer, got: $COLD_POOL"
[[ "$HOT_COUNT" =~ ^[1-9][0-9]*$ ]] || die "HOT_COUNT must be a positive integer, got: $HOT_COUNT"
[[ "$WARM_PARALLEL" =~ ^[1-9][0-9]*$ ]] || die "WARM_PARALLEL must be a positive integer, got: $WARM_PARALLEL"
[[ "$REDIRECT_PORT" =~ ^[1-9][0-9]*$ ]] || die "REDIRECT_PORT must be a positive integer, got: $REDIRECT_PORT"

while [[ "$BASE_URL" == */ ]]; do BASE_URL=${BASE_URL%/}; done
while [[ "$FAKE_URL" == */ ]]; do FAKE_URL=${FAKE_URL%/}; done
case "$BASE_URL" in
  http://* | https://*) ;;
  *) die "BASE_URL must start with http:// or https://, got: $BASE_URL" ;;
esac
case "$FAKE_URL" in
  http://* | https://*) ;;
  *) die "FAKE_URL must start with http:// or https://, got: $FAKE_URL" ;;
esac
case "$FAKE_URL" in
  *helius-rpc.com* | *api.wormholescan.io*)
    die "FAKE_URL points at a real provider ($FAKE_URL); this benchmark must use the local fake upstream"
    ;;
esac

OHA_VERSION=$(oha --version 2>&1 || true)
OHA_VERSION=${OHA_VERSION%%$'\n'*}
[[ -n "$OHA_VERSION" ]] || die "could not read the oha version"
OHA_HELP=$(oha --help 2>&1 || true)
for flag in --output-format --urls-from-file --redirect --disable-keepalive; do
  grep -q -- "$flag" <<<"$OHA_HELP" ||
    die "$OHA_VERSION does not support $flag; update the installed oha"
done

# 50 connections plus keep-alive needs more descriptors than the usual 1024.
if ! ulimit -n 65535 2>/dev/null; then
  die "could not raise the open-file limit to 65535 (soft=$(ulimit -n), hard=$(ulimit -Hn)). Raise the hard limit and retry"
fi
open_files=$(ulimit -n)
[[ "$open_files" -ge 65535 ]] ||
  die "open-file limit is $open_files after 'ulimit -n 65535'; 65535 is required"

# The fake must be reachable and must accept a reset before any traffic starts.
curl -fsS --max-time 5 "$FAKE_URL/stats" >/dev/null ||
  die "fake upstream stats endpoint is not reachable at $FAKE_URL/stats"
reset_body=$(curl -fsS --max-time 5 -X POST "$FAKE_URL/stats/reset") ||
  die "POST $FAKE_URL/stats/reset failed"
[[ "$(jq -r '.reset // false' <<<"$reset_body" 2>/dev/null)" == "true" ]] ||
  die "unexpected /stats/reset response: $reset_body"

# Any HTTP response means the API is listening; 404 is the expected body.
api_probe=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "$BASE_URL/" || true)
[[ "$api_probe" =~ ^[1-9][0-9][0-9]$ ]] ||
  die "API server is not reachable at $BASE_URL (curl got: ${api_probe:-no response})"

stamp=$(date -u +%Y%m%dT%H%M%SZ)
if [[ "$DRY_RUN" -eq 1 ]]; then
  run_dir="$RESULTS_ROOT/3e-dry-run-${LABEL}-${stamp}"
else
  run_dir="$RESULTS_ROOT/3e-${LABEL}-${stamp}"
fi
[[ ! -e "$run_dir" ]] || run_dir="${run_dir}-$$"
mkdir -p -- "$run_dir"

{
  printf 'date_utc: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'label: %s\n' "$LABEL"
  printf 'base_url: %s\n' "$BASE_URL"
  printf 'fake_url: %s\n' "$FAKE_URL"
  printf 'api_version: %s\n' "$API_VERSION"
  printf 'runs_per_workload: %s\n' "$RUNS"
  printf 'duration_per_run: %s\n' "$DURATION"
  printf 'connections: %s\n' "$CONNECTIONS"
  printf 'cold_pool_per_run: %s\n' "$COLD_POOL"
  printf 'hot_count: %s\n' "$HOT_COUNT"
  printf 'warm_parallel: %s\n' "$WARM_PARALLEL"
  printf 'redirect_port: %s\n' "$REDIRECT_PORT"
  printf 'oha: %s\n' "$OHA_VERSION"
  printf 'open_file_limit: %s\n' "$open_files"
  printf 'host: %s\n' "$(uname -n)"
  printf 'uname: %s\n' "$(uname -a)"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    printf 'note: DRY RUN, not an R-cold/R-mix result\n'
  fi
} >"$run_dir/meta.txt"

cat >"$run_dir/redirector.py" <<'PY'
#!/usr/bin/env python3
"""Hand out one fresh signature per cold request as a local 302.

Every request (except /__count) pops the next signature from the file. The
signature list is generated fresh for each run by bench_cold.sh, so no cold
signature is ever reused, including across runs.
"""

import argparse
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--signatures", required=True)
    parser.add_argument("--base-url", required=True)
    parser.add_argument("--api-version", required=True)
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()

    with open(args.signatures, encoding="utf-8") as handle:
        signatures = [line.strip() for line in handle if line.strip()]
    if not signatures:
        raise SystemExit("no signatures in " + args.signatures)

    state = {"next": 0, "served": 0, "exhausted": 0}
    lock = threading.Lock()
    base = args.base_url.rstrip("/")
    analyse_path = f"/api/{args.api_version}/transactions/analyse"

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def _send(self, status: int, body: bytes, headers=None) -> None:
            self.send_response(status)
            for key, value in (headers or {}).items():
                self.send_header(key, value)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self) -> None:
            if self.path == "/__count":
                with lock:
                    body = json.dumps(
                        {
                            "served": state["served"],
                            "remaining": len(signatures) - state["next"],
                            "exhausted": state["exhausted"],
                        }
                    ).encode()
                self._send(200, body, {"Content-Type": "application/json"})
                return

            with lock:
                index = state["next"]
                if index < len(signatures):
                    signature = signatures[index]
                    state["next"] = index + 1
                    state["served"] += 1
                else:
                    signature = None
                    state["exhausted"] += 1

            if signature is None:
                self._send(503, b"cold signature pool exhausted\n", {"Content-Type": "text/plain"})
                return

            location = f"{base}{analyse_path}?transaction_hash={signature}&chain=solana"
            self._send(302, b"", {"Location": location})

        def log_message(self, *_args) -> None:
            pass

    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.daemon_threads = True
    server.serve_forever()


if __name__ == "__main__":
    main()
PY
chmod +x "$run_dir/redirector.py"

printf 'bench_cold.sh: config %s, %s, /api/%s\n' "$LABEL" "$run_dir" "$API_VERSION" >&2
if [[ "$DRY_RUN" -eq 1 ]]; then
  printf 'bench_cold.sh: DRY RUN with %s per run; these numbers are not a baseline\n' "$DURATION" >&2
fi

# Deterministic fixed hot set: SHA-512 of a label, base58-encoded, so every run
# and both configs warm exactly the same keys. The set is not secret material.
hot_file="$run_dir/hot-signatures.txt"
python3 - "$hot_file" "$HOT_COUNT" <<'PY'
import hashlib
import sys

ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def base58(raw: bytes) -> str:
    number = int.from_bytes(raw, "big")
    digits = []
    while number:
        number, remainder = divmod(number, 58)
        digits.append(ALPHABET[remainder])
    zeros = len(raw) - len(raw.lstrip(b"\0"))
    return "1" * zeros + ("".join(reversed(digits)) if digits else "")


path, count = sys.argv[1], int(sys.argv[2])
signatures = []
for i in range(count):
    digest = hashlib.sha512(f"bridgeIndexer/3e/hot/{i}".encode()).digest()
    signatures.append(base58(digest))
assert len(set(signatures)) == count, "hot signature collision"
with open(path, "w", encoding="utf-8") as handle:
    for signature in signatures:
        handle.write(signature + "\n")
PY

hot_urls_file="$run_dir/hot-urls.txt"
: >"$hot_urls_file"
while IFS= read -r signature; do
  [[ -z "$signature" ]] && continue
  printf '%s/api/%s/transactions/analyse?transaction_hash=%s&chain=solana\n' \
    "$BASE_URL" "$API_VERSION" "$signature" >>"$hot_urls_file"
done <"$hot_file"

if command -v sha256sum >/dev/null 2>&1; then
  hot_sha=$(sha256sum "$hot_file" | awk '{print $1}')
else
  hot_sha=$(shasum -a 256 "$hot_file" | awk '{print $1}')
fi
printf 'bench_cold.sh: hot set %s signatures, sha256 %s\n' "$HOT_COUNT" "$hot_sha" >&2

redirect_url="http://127.0.0.1:${REDIRECT_PORT}/cold"
REDIRECTOR_PID=""

cleanup() {
  if [[ -n "${REDIRECTOR_PID:-}" ]]; then
    kill "$REDIRECTOR_PID" 2>/dev/null || true
    wait "$REDIRECTOR_PID" 2>/dev/null || true
    REDIRECTOR_PID=""
  fi
}
trap cleanup EXIT

start_redirector() {
  local signatures=$1
  local log=$2
  python3 "$run_dir/redirector.py" \
    --signatures "$signatures" \
    --base-url "$BASE_URL" \
    --api-version "$API_VERSION" \
    --port "$REDIRECT_PORT" >"$log" 2>&1 &
  REDIRECTOR_PID=$!
  local _
  for _ in $(seq 1 100); do
    if curl -fsS --max-time 1 "http://127.0.0.1:${REDIRECT_PORT}/__count" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  die "redirector did not start on port $REDIRECT_PORT; see $log"
}

stop_redirector() {
  if [[ -n "${REDIRECTOR_PID:-}" ]]; then
    kill "$REDIRECTOR_PID" 2>/dev/null || true
    wait "$REDIRECTOR_PID" 2>/dev/null || true
    REDIRECTOR_PID=""
  fi
}

generate_cold() {
  local path=$1
  local count=$2
  python3 - "$path" "$count" <<'PY'
import os
import sys

ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def base58(raw: bytes) -> str:
    number = int.from_bytes(raw, "big")
    digits = []
    while number:
        number, remainder = divmod(number, 58)
        digits.append(ALPHABET[remainder])
    zeros = len(raw) - len(raw.lstrip(b"\0"))
    return "1" * zeros + ("".join(reversed(digits)) if digits else "")


path, count = sys.argv[1], int(sys.argv[2])
seen = set()
signatures = []
while len(signatures) < count:
    signature = base58(os.urandom(64))
    if signature in seen:
        continue
    seen.add(signature)
    signatures.append(signature)
with open(path, "w", encoding="utf-8") as handle:
    for signature in signatures:
        handle.write(signature + "\n")
PY
}

warm_hot_set() {
  local log=$1
  printf 'bench_cold.sh: warming %s hot signatures with %s parallel requests\n' \
    "$HOT_COUNT" "$WARM_PARALLEL" >&2
  if ! xargs -P "$WARM_PARALLEL" -n 1 sh -c '
    url=$1
    attempt=1
    while [ "$attempt" -le 5 ]; do
      code=$(curl -s -o /dev/null -w "%{http_code}" --max-time 10 "$url" || true)
      [ "$code" = "200" ] && exit 0
      sleep 0.5
      attempt=$((attempt + 1))
    done
    printf "warm-up failed: HTTP %s %s\n" "$code" "$url" >&2
    exit 1
  ' _ <"$hot_urls_file" >"$log" 2>&1; then
    tail -20 "$log" >&2 || true
    die "hot-set warm-up failed; see $log"
  fi
  printf 'bench_cold.sh: hot set warm\n' >&2
}

reset_fake_stats() {
  local body
  body=$(curl -fsS --max-time 5 -X POST "$FAKE_URL/stats/reset") ||
    die "POST $FAKE_URL/stats/reset failed"
  [[ "$(jq -r '.reset // false' <<<"$body" 2>/dev/null)" == "true" ]] ||
    die "unexpected /stats/reset response: $body"
}

num_ratio() {
  awk -v numerator="$1" -v denominator="$2" 'BEGIN {
    if (denominator > 0) printf "%.3f", numerator / denominator
    else printf "n/a"
  }'
}

run_one() {
  local workload=$1
  local run=$2
  local tag="${LABEL}-${workload}-run${run}"
  local cold_file="$run_dir/${tag}-cold-signatures.txt"
  local urls_file="$run_dir/${tag}-urls.txt"
  local oha_json="$run_dir/${tag}-oha.json"
  local oha_log="$run_dir/${tag}-oha.log"
  local stats_json="$run_dir/${tag}-stats.json"
  local redirect_log="$run_dir/${tag}-redirector.log"

  printf 'bench_cold.sh: %s run %s (%s, %s connections, %s)\n' \
    "$workload" "$run" "$LABEL" "$CONNECTIONS" "$DURATION" >&2

  generate_cold "$cold_file" "$COLD_POOL"

  if [[ "$workload" == "R-cold" ]]; then
    # One cold entry: every request is redirected to the next fresh signature.
    printf '%s\n' "$redirect_url" >"$urls_file"
  else
    # 9 copies of the hot set plus HOT_COUNT cold entries: 90% / 10% by line.
    : >"$urls_file"
    local _
    for _ in $(seq 1 9); do
      cat "$hot_urls_file" >>"$urls_file"
    done
    for _ in $(seq 1 "$HOT_COUNT"); do
      printf '%s\n' "$redirect_url" >>"$urls_file"
    done
  fi

  start_redirector "$cold_file" "$redirect_log"

  reset_fake_stats

  if ! oha -z "$DURATION" \
    -c "$CONNECTIONS" \
    -r 1 \
    --disable-keepalive \
    --no-tui \
    --urls-from-file "$urls_file" \
    --output-format json \
    -o "$oha_json" >"$oha_log" 2>&1; then
    tail -20 "$oha_log" >&2 || true
    die "oha failed for $tag"
  fi
  [[ -s "$oha_json" ]] || die "oha wrote no JSON for $tag"

  curl -fsS --max-time 5 "$FAKE_URL/stats" >"$stats_json" ||
    die "GET $FAKE_URL/stats failed after $tag"
  local count_json
  count_json=$(curl -fsS --max-time 5 "http://127.0.0.1:${REDIRECT_PORT}/__count") ||
    die "GET redirector count failed after $tag"
  stop_redirector

  local rps p50 p99 s200 s429 s503 other transport total success_rate
  read -r rps p50 p99 s200 s429 s503 other transport total success_rate < <(
    jq -r '
      def scd: (.statusCodeDistribution // {});
      def err: (.errorDistribution // {});
      def sum_obj: ([.[]] | add) // 0;
      [
        (.metrics.requests_per_sec // .summary.requestsPerSec // 0),
        (.metrics.latency_ms.p50 // ((.latencyPercentiles.p50 // 0) * 1000)),
        (.metrics.latency_ms.p99 // ((.latencyPercentiles.p99 // 0) * 1000)),
        (scd["200"] // 0),
        (scd["429"] // 0),
        (scd["503"] // 0),
        ([scd | to_entries[] | select(.key != "200" and .key != "429" and .key != "503") | .value] | add) // 0,
        (err | sum_obj),
        (([scd[]] | add) // 0),
        (.metrics.success_rate // .summary.successRate // 0)
      ] | @tsv
    ' "$oha_json"
  ) || die "could not parse $oha_json"

  local served remaining exhausted
  read -r served remaining exhausted < <(
    jq -r '[.served, .remaining, .exhausted] | @tsv' <<<"$count_json"
  ) || die "could not parse redirector count"

  local get_tx get_accounts operations global_tx high_water
  read -r get_tx get_accounts operations global_tx high_water < <(
    jq -r '[
      (.calls["helius:getTransaction"] // 0),
      (.calls["helius:getMultipleAccounts"] // 0),
      (.calls["wormhole:operations"] // 0),
      (.calls["wormhole:global-tx"] // 0),
      (.in_flight_high_water // 0)
    ] | @tsv' "$stats_json"
  ) || die "could not parse $stats_json"

  local total_all=$((total + transport))
  local cold_share
  cold_share=$(num_ratio "$served" "$total_all")

  printf '\n### %s run %s (config %s, /api/%s)\n' "$workload" "$run" "$LABEL" "$API_VERSION"
  printf 'oha: %.1f req/s | median %.2f ms | p99 %.2f ms | requests %s | success_rate %.3f\n' \
    "$rps" "$p50" "$p99" "$total_all" "$success_rate"
  printf 'status: 200=%s 429=%s 503=%s other=%s transport_errors=%s\n' \
    "$s200" "$s429" "$s503" "$other" "$transport"
  printf 'cold requests: %s (share of oha requests %s; redirector remaining %s, exhausted %s)\n' \
    "$served" "$cold_share" "$remaining" "$exhausted"
  printf 'upstream calls: getTransaction=%s getMultipleAccounts=%s operations=%s global_tx=%s\n' \
    "$get_tx" "$get_accounts" "$operations" "$global_tx"
  printf 'upstream per cold request: getTransaction=%s getMultipleAccounts=%s operations=%s\n' \
    "$(num_ratio "$get_tx" "$served")" \
    "$(num_ratio "$get_accounts" "$served")" \
    "$(num_ratio "$operations" "$served")"
  printf 'in-flight high-water: %s\n' "$high_water"
  printf 'raw fake stats: %s\n' "$(jq -c . "$stats_json")"
  printf 'files: %s %s\n' "$oha_json" "$stats_json"
}

for run in $(seq 1 "$RUNS"); do
  run_one "R-cold" "$run"
done

# Warm the hot keys once for the whole R-mix series: the completed-result cache
# lives 600 s, well past both runs, and doing it inside run_one would repeat the
# warm-up (and its upstream calls) before every run.
warm_hot_set "$run_dir/${LABEL}-R-mix-warmup.log"

for run in $(seq 1 "$RUNS"); do
  run_one "R-mix" "$run"
done

printf '\nResults directory: %s\n' "$run_dir"
printf 'Cold requests go through one local 302 hop; that hop is included in the measured latency.\n'
printf 'oha runs with --disable-keepalive, so every request pays a loopback TCP handshake inside its measured latency.\n'
printf 'The fake upstream counts every RPC method it sees, including helius:getAsset from token metadata lookups.\n'
