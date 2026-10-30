#!/usr/bin/env bash
# Hourly health watch: dead-letter stream, ingest age, and a short reconcile.
set -uo pipefail
cd "$HOME/bridgeIndexer"
set -a; source .env; set +a

LOG="$HOME/watch.log"
fail=0
note() { echo "$(date -u +%FT%TZ) $*" >> "$LOG"; }

dl=$(redis6-cli -u "$REDIS_URL" --no-auth-warning XLEN bridge:transactions:failed 2>/dev/null || echo "?")
if [ "$dl" != "0" ]; then note "FAIL dead-letter length=$dl"; fail=1; fi

age=$(psql "$DATABASE_URL" -t -A -c "SELECT COALESCE(extract(epoch FROM now() - max(created_at))::int, -1) FROM bridge_transfers;" 2>/dev/null || echo "-1")
if [ "$age" -lt 0 ] || [ "$age" -gt 3600 ]; then note "FAIL last stored row age=${age}s"; fail=1; fi

out=$(./target/release/reconcile --window-hours 2 --end-offset-secs 600 --min-recall 99.9 2>&1)
rc=$?
echo "$out" | grep -E "persisted:|missed:|recall:|ingestion lag:" | sed "s/^/$(date -u +%FT%TZ) /" >> "$LOG"
if [ $rc -ne 0 ]; then note "FAIL reconcile exit=$rc"; fail=1; fi

[ "$fail" -eq 0 ] && note "ok"

# Optional dead-man's switch (free healthchecks.io check): also catches the whole box dying.
if [ -n "${WATCH_PING_URL:-}" ]; then
  if [ "$fail" -eq 0 ]; then curl -fsS -m 10 "$WATCH_PING_URL" >/dev/null
  else curl -fsS -m 10 "$WATCH_PING_URL/fail" >/dev/null; fi
fi
exit $fail