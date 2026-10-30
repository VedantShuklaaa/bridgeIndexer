-- 3g: the list endpoint's token filter was the only predicate EXPLAIN showed as a
-- sequential scan. A token with few or no rows could not fill a page from the
-- ordered (created_at, id) index and fell back to a full table scan (47 to
-- 122 ms for a zero-match token on 1M seeded rows, depending on page cache). This
-- index keeps the page index-ordered and removes that scan.
--
-- The other filters are served by an existing index with no sort node:
-- created_at by idx_bt_created, status by idx_bt_status_created, protocol by the
-- (protocol, message_key) unique index (empty; a dense protocol scans
-- idx_bt_created in order and filters), and direction/chain/range by the
-- ordered idx_bt_created scan, which fills a page within a few hundred rows for
-- every value present in the data. See docs/3g-list-endpoint.md for the plans.
--
-- Plain CREATE INDEX (not CONCURRENTLY), like the earlier index migrations:
-- sqlx runs migrations in a transaction, and CONCURRENTLY cannot. Expect the build
-- to block writes while it runs; size it against the production row count first.
CREATE INDEX idx_bt_token_created
    ON bridge_transfers (token, created_at DESC, id DESC);
