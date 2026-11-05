//! Prometheus metrics.
//!
//! [`init`] installs the global recorder and serves `/metrics` on the metrics port.
//! The metric names and recorded labels are documented in the README "Metrics"
//! section; the constants here are the single source of truth for names.

use metrics_exporter_prometheus::PrometheusBuilder;
use std::net::SocketAddr;

/// Metrics port when `METRICS_PORT` is unset.
pub const DEFAULT_PORT: u16 = 9100;

pub const CACHE_LOOKUPS_TOTAL: &str = "cache_lookups_total";
pub const CACHE_REFRESH_SKIPPED_TOTAL: &str = "cache_refresh_skipped_total";
pub const LIMITER_WAIT_SECONDS: &str = "limiter_wait_seconds";
pub const UPSTREAM_CALLS_TOTAL: &str = "upstream_calls_total";
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "http_request_duration_seconds";
pub const STREAM_LAG: &str = "stream_lag";
pub const STREAM_PENDING: &str = "stream_pending";
pub const DEAD_LETTER_DEPTH: &str = "dead_letter_depth";
pub const INGEST_AGE_SECONDS: &str = "ingest_age_seconds";
pub const INGEST_RECONNECTS_TOTAL: &str = "ingest_reconnects_total";

/// Installs the Prometheus recorder and starts its HTTP listener on
/// `0.0.0.0:$METRICS_PORT` (default 9100). Every role calls this once at startup.
pub fn init() -> anyhow::Result<()> {
    let port = std::env::var("METRICS_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    PrometheusBuilder::new()
        .with_http_listener(addr)
        .install()?;

    describe();
    tracing::info!(%addr, "prometheus metrics listening");
    Ok(())
}

/// Registers HELP text for every metric this process can emit.
fn describe() {
    metrics::describe_counter!(
        CACHE_LOOKUPS_TOTAL,
        "Read-through cache lookups by result (memory, pg_fresh, pg_stale, cold)"
    );
    metrics::describe_counter!(
        CACHE_REFRESH_SKIPPED_TOTAL,
        "Stale-while-revalidate refreshes skipped, by reason (no_token, in_flight)"
    );
    metrics::describe_histogram!(
        LIMITER_WAIT_SECONDS,
        metrics::Unit::Seconds,
        "Time spent waiting for a provider rate-limiter token, by provider and lane"
    );
    metrics::describe_counter!(
        UPSTREAM_CALLS_TOTAL,
        "Upstream provider calls by outcome (ok, err, timeout), by provider and lane"
    );
    metrics::describe_histogram!(
        HTTP_REQUEST_DURATION_SECONDS,
        metrics::Unit::Seconds,
        "HTTP request duration by matched route and status class"
    );
    metrics::describe_gauge!(
        STREAM_LAG,
        "Bridge stream consumer-group lag, from the health probe"
    );
    metrics::describe_gauge!(
        STREAM_PENDING,
        "Bridge stream consumer-group pending entries, from the health probe"
    );
    metrics::describe_gauge!(
        DEAD_LETTER_DEPTH,
        "Entries in the failed-transaction stream, from the health probe"
    );
    metrics::describe_gauge!(
        INGEST_AGE_SECONDS,
        "Seconds since the ingester last handled a notification, from the health probe"
    );
    metrics::describe_counter!(
        INGEST_RECONNECTS_TOTAL,
        "Solana WebSocket reconnects by reason (watchdog, subscribe_failed, closed, error)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The metrics macros are no-ops without a recorder: a module that records
    /// before (or without) `init` must not panic.
    #[test]
    fn recording_without_an_installed_recorder_does_not_panic() {
        metrics::counter!(CACHE_LOOKUPS_TOTAL, "result" => "memory").increment(1);
        metrics::histogram!(
            LIMITER_WAIT_SECONDS,
            "provider" => "helius",
            "lane" => "api"
        )
        .record(0.001);
    }
}
