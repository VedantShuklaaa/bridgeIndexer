# 3e-2: live workers wait for a Helius token

## Problem

After a redrive, the dead-letter stream filled with `service overloaded` entries. `analyse_tx` took its Helius token through `acquire_token`, a 2 s wait that then fails with `Overloaded`. The API and the Redis workers shared that wait. The API should shed after 2 s, but a worker's message can wait in the stream, so shedding there only moved it to the dead-letter stream. A worker retries an `Overloaded` error 5 times, so a burst larger than the 4/s live limiter could serve in that time ended up dead-lettered.

## Before

* 605 dead-letter entries: 197 `service overloaded`, 273 tx version -32015, 58 transaction not found, 50 VAA not available, 26 builder error, 1 rate limited.
* The 408 non-overloaded entries were redriven (`--exclude-error-contains overloaded --pace-ms 250`): 408 re-published, none came back.
* The overloaded count kept growing while redriving: 197, 221, 228, 237.
* Redriving the 237 on the old build re-dead-lettered 7 (all stamped 12:44:35Z to 12:44:48Z, before the restart).

## Change

* `acquire_token_within(limiter, max_wait)`; `acquire_token` keeps the 2 s API wait.
* `AppState::helius_wait`: 2 s for the API state, 20 s for `for_live` (consumer workers, recovery, backfill).
* Sizing: 5 workers x `XREADGROUP COUNT 10` = at most 50 waiters, served in about 13 s at 4/s. Worst case for one message is about 200 s, under the 5 min `XAUTOCLAIM` min-idle.

## After

* Deployed 12:50:22Z. Dead-letter and overload log lines since: 0.
* The 7 leftover entries redriven on the new build at about 13:08Z: 7 re-published, 0 remaining.
* Dry run at about 13:19Z: scanned 0 dead-letter entries, 0 overload log lines.

## Limits

This was not a burst test. After the restart only a few messages arrived, so it shows that nothing sheds at normal load, not that a 400-entry redrive now survives. The retry path inside `call_upstream_inner` still uses the 2 s API wait. The R-mix 503s on hot-set requests come from the API side and are covered by 3e-3 (`perf/3e-priority`).
