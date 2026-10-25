#!/usr/bin/env bash
# Build urls.txt for the R-hit benchmark: one analyse URL per stored Solana
# transaction, newest first. The URL list is the workload; this script does not
# make any HTTP requests and never reads .env.
#
# R-hit (README "Workloads") is 769 distinct signatures with a warm cache, so
# the query only takes rows that already have an analysis, and it stops at 769.
set -euo pipefail

readonly EXPECTED_SIGNATURES=769

DATABASE_URL=${DATABASE_URL:-}
BASE_URL=${BASE_URL:-}
API_VERSION=${API_VERSION:-v1}
OUTPUT_FILE=${OUTPUT_FILE:-urls.txt}
PRINT_SQL=0

usage() {
  cat <<'EOF'
Usage: scripts/make_urls.sh [v1|v2] [OUTPUT] [options]

Builds one line per signature:

  BASE_URL/api/<version>/transactions/analyse?transaction_hash=<sig>&chain=solana

The set is the 769 newest distinct Solana signatures that have a stored
analysis, so the first R-hit run is a cache fill rather than a cold fetch.

Environment:
  DATABASE_URL   Postgres connection string (required)
  BASE_URL       API origin, e.g. http://172.31.38.190:8080 (required)
  API_VERSION    v1 or v2 (default: v1)
  OUTPUT_FILE    output path, or - for stdout (default: urls.txt)

Options:
  --database-url URL   override DATABASE_URL
  --base-url URL       override BASE_URL
  --api-version V      v1 or v2 (default: v1)
  -o, --output FILE    output path, or - for stdout
  --print-sql          print the SQL and exit without connecting
  -h, --help           show this help

The script never reads .env. It uses psql, so DATABASE_URL must be one that
psql accepts. The SQL filters on:
  transactions.chain = 1                    (Wormhole chain ID for Solana)
  bridge_transfers.analysis IS NOT NULL     (warm-cache workload)
  observed_tx_hash has base58 signature shape (64-88 chars)
EOF
}

die() {
  printf 'make_urls.sh: %s\n' "$*" >&2
  exit 1
}

# Short positional forms: `make_urls.sh v2`, `make_urls.sh v2 urls-v2.txt`.
case "${1:-}" in
  v1 | v2)
    API_VERSION=$1
    shift
    ;;
esac
if [[ $# -gt 0 && ${1:-} != -* ]]; then
  OUTPUT_FILE=$1
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --database-url)
      [[ $# -ge 2 ]] || die "--database-url needs a value"
      DATABASE_URL=$2
      shift 2
      ;;
    --base-url)
      [[ $# -ge 2 ]] || die "--base-url needs a value"
      BASE_URL=$2
      shift 2
      ;;
    --api-version)
      [[ $# -ge 2 ]] || die "--api-version needs a value"
      API_VERSION=$2
      shift 2
      ;;
    -o | --output)
      [[ $# -ge 2 ]] || die "$1 needs a value"
      OUTPUT_FILE=$2
      shift 2
      ;;
    --print-sql)
      PRINT_SQL=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1 (try --help)"
      ;;
  esac
done

# transactions.chain stores Chain::wormhole_id(), and Solana is 1
# (src/domain/transaction.rs). observed_tx_hash is the Solana signature the
# transfer was decoded from, and bridge_transfers.analysis is what the read
# path serves on a cache hit.
SQL=$(cat <<'SQL'
SELECT b.observed_tx_hash
FROM bridge_transfers AS b
JOIN transactions AS t ON t.id = b.transaction_id
WHERE t.chain = 1
  AND b.analysis IS NOT NULL
  AND b.observed_tx_hash ~ '^[1-9A-HJ-NP-Za-km-z]{64,88}$'
GROUP BY b.observed_tx_hash, t.timestamp, t.created_at
ORDER BY t.timestamp DESC NULLS LAST, t.created_at DESC, b.observed_tx_hash
LIMIT 769;
SQL
)

if [[ $PRINT_SQL -eq 1 ]]; then
  printf '%s\n' "$SQL"
  exit 0
fi

case "$API_VERSION" in
  v1 | v2) ;;
  *) die "API_VERSION must be v1 or v2, got: $API_VERSION" ;;
esac

[[ -n "$DATABASE_URL" ]] ||
  die "DATABASE_URL is not set (use --database-url or the environment); .env is deliberately not read"
[[ -n "$BASE_URL" ]] ||
  die "BASE_URL is not set (use --base-url or the environment)"
[[ -n "$OUTPUT_FILE" ]] || die "output path is empty"

# Normalise the origin: no trailing slashes, and require a scheme.
while [[ "$BASE_URL" == */ ]]; do
  BASE_URL=${BASE_URL%/}
done
case "$BASE_URL" in
  http://* | https://*) ;;
  *) die "BASE_URL must start with http:// or https://, got: $BASE_URL" ;;
esac

command -v psql >/dev/null 2>&1 || die "psql is required"
command -v mktemp >/dev/null 2>&1 || die "mktemp is required"

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/make_urls.XXXXXX")
cleanup() {
  rm -rf -- "$tmp_dir"
}
trap cleanup EXIT

sig_file="$tmp_dir/signatures.txt"
url_file="$tmp_dir/urls.txt"

if ! psql -X -q -A -t -v ON_ERROR_STOP=1 --dbname="$DATABASE_URL" \
  -c "$SQL" >"$sig_file"; then
  die "query failed; check DATABASE_URL, that migrations are applied, and that psql can reach the database"
fi

# `grep -c` exits 1 when there are no matches, which is a valid (fatal) count here.
signature_count=$(grep -cve '^[[:space:]]*$' "$sig_file" || true)
signature_count=${signature_count//[[:space:]]/}

if [[ "$signature_count" -ne "$EXPECTED_SIGNATURES" ]]; then
  die "expected $EXPECTED_SIGNATURES Solana signatures with a stored analysis, found $signature_count; R-hit is defined on 769, so this URL list would not be the same workload"
fi

: >"$url_file"
while IFS= read -r signature; do
  [[ -z "$signature" ]] && continue
  # Same shape the SQL filter uses. A 64-byte signature is 64-88 base58
  # characters (86 is not unusual), and the server's base58 decode to exactly
  # 64 bytes remains the authoritative check.
  [[ "$signature" =~ ^[1-9A-HJ-NP-Za-km-z]{64,88}$ ]] ||
    die "database returned a value that is not a Solana signature: $signature"
  printf '%s/api/%s/transactions/analyse?transaction_hash=%s&chain=solana\n' \
    "$BASE_URL" "$API_VERSION" "$signature"
done <"$sig_file" >"$url_file"

if [[ "$OUTPUT_FILE" == "-" ]]; then
  cat "$url_file"
  printf 'make_urls.sh: wrote %s URLs to stdout\n' "$signature_count" >&2
  exit 0
fi

out_dir=$(dirname -- "$OUTPUT_FILE")
[[ -d "$out_dir" ]] || die "output directory does not exist: $out_dir"

# Write next to the destination and rename, so an interrupted run never leaves
# a half-written urls.txt that a benchmark could pick up.
final_tmp=$(mktemp "$out_dir/.urls.XXXXXX")
cp -- "$url_file" "$final_tmp"
mv -f -- "$final_tmp" "$OUTPUT_FILE"

printf 'make_urls.sh: wrote %s URLs to %s (newest first, %s)\n' \
  "$signature_count" "$OUTPUT_FILE" "$API_VERSION" >&2
