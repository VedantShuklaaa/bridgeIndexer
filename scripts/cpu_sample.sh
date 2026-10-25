#!/usr/bin/env bash
# Sample CPU once per second with `vmstat 1`, prefixing every line with a UTC
# timestamp. Run one copy on the load generator and one on the API server for
# the duration of an R-hit series, then stop both with Ctrl+C.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/cpu_sample.sh OUTPUT_FILE

Writes `vmstat 1` output to OUTPUT_FILE, one timestamped line per second.
Stops cleanly on Ctrl+C. The file is truncated when the sampler starts.
EOF
}

die() {
  printf 'cpu_sample.sh: %s\n' "$*" >&2
  exit 1
}

if [[ $# -ne 1 || ${1:-} == "-h" || ${1:-} == "--help" ]]; then
  usage >&2
  if [[ ${1:-} == "-h" || ${1:-} == "--help" ]]; then
    exit 0
  fi
  exit 2
fi

OUTPUT_FILE=$1
[[ -n "$OUTPUT_FILE" ]] || die "output file must not be empty"
[[ ! -d "$OUTPUT_FILE" ]] || die "output path is a directory: $OUTPUT_FILE"

command -v vmstat >/dev/null 2>&1 ||
  die "vmstat is required (Linux procps; macOS does not ship it)"
command -v date >/dev/null 2>&1 || die "date is required"

out_dir=$(dirname -- "$OUTPUT_FILE")
[[ -d "$out_dir" ]] || die "output directory does not exist: $out_dir"

# Truncate up front so a failed start cannot be mistaken for a fresh sample.
: >"$OUTPUT_FILE" || die "cannot write to $OUTPUT_FILE"

stop() {
  printf '\ncpu_sample.sh: stopped; samples in %s\n' "$OUTPUT_FILE" >&2
  exit 0
}
trap stop INT TERM

printf 'cpu_sample.sh: vmstat 1 -> %s (Ctrl+C to stop)\n' "$OUTPUT_FILE" >&2

set +e
vmstat 1 | while IFS= read -r line; do
  printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$line"
done >>"$OUTPUT_FILE"
status=$?
set -e

# 130 = SIGINT, 141 = SIGPIPE (the reader stopped), 143 = SIGTERM.
if [[ "$status" -ne 0 && "$status" -ne 130 && "$status" -ne 141 && "$status" -ne 143 ]]; then
  die "vmstat exited with status $status"
fi
