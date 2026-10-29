//! Per-mint cache around an adapter's token metadata lookup.
//!
//! Metadata is enrichment for a transfer, not the transfer itself, so it must not
//! compete with `getTransaction` for the shared Helius quota. On a cache miss the
//! lookup takes a limiter token only if one is free right now; otherwise it answers
//! "no metadata" without an upstream call and without caching that answer.

use std::sync::Arc;
use std::time::{Duration, Instant};

use moka::Expiry;
use moka::future::Cache;

use super::ChainAdapter;
use crate::clients::limiter::Limiter;
use crate::error::AppError;

/// A resolved mint keeps its answer for a day. Failures and "the provider has no
/// metadata for this mint" are retried after a minute.
const SUCCESS_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_TTL: Duration = Duration::from_secs(60);
const MAX_ENTRIES: u64 = 10_000;

#[derive(Clone)]
enum CachedMetadata {
    Resolved {
        decimals: Option<u8>,
        symbol: Option<String>,
    },
    Failed(String),
}

fn ttl_for(value: &CachedMetadata) -> Duration {
    match value {
        CachedMetadata::Resolved { decimals, symbol } if decimals.is_some() || symbol.is_some() => {
            SUCCESS_TTL
        }
        _ => RETRY_TTL,
    }
}

struct MetadataExpiry;

impl Expiry<(u16, String), CachedMetadata> for MetadataExpiry {
    fn expire_after_create(
        &self,
        _key: &(u16, String),
        value: &CachedMetadata,
        _created_at: Instant,
    ) -> Option<Duration> {
        Some(ttl_for(value))
    }
}

pub struct TokenMetadataLookup {
    limiter: Arc<Limiter>,
    cache: Cache<(u16, String), CachedMetadata>,
}

impl TokenMetadataLookup {
    pub fn new(limiter: Arc<Limiter>) -> Self {
        Self {
            limiter,
            cache: Cache::builder()
                .max_capacity(MAX_ENTRIES)
                .expire_after(MetadataExpiry)
                .build(),
        }
    }

    pub fn limiter(&self) -> Arc<Limiter> {
        self.limiter.clone()
    }

    /// Resolves one mint. Concurrent lookups of the same mint share a single
    /// upstream call. Returns `Ok(None)` when no limiter token was free right now
    /// (that outcome is not cached) and `Err` when the provider call failed
    /// (cached briefly, so a transient failure is not retried on every request).
    pub async fn resolve(
        &self,
        token_chain: u16,
        token_address: &str,
        adapter: &dyn ChainAdapter,
    ) -> Result<Option<(Option<u8>, Option<String>)>, AppError> {
        let key = (token_chain, token_address.to_string());
        let cached = self
            .cache
            .optionally_get_with(key, async {
                // Never wait: metadata is best effort, and a token that a
                // getTransaction needs must not be held by a lookup that can be
                // skipped. A skipped lookup is not inserted, so the next request
                // tries again once the quota has room.
                if self.limiter.check().is_err() {
                    return None;
                }
                let value = match adapter.token_metadata(token_address).await {
                    Ok((decimals, symbol)) => CachedMetadata::Resolved { decimals, symbol },
                    Err(error) => CachedMetadata::Failed(error.to_string()),
                };
                Some(value)
            })
            .await;

        match cached {
            None => Ok(None),
            Some(CachedMetadata::Resolved { decimals, symbol }) => Ok(Some((decimals, symbol))),
            Some(CachedMetadata::Failed(message)) => Err(AppError::UpstreamProvider {
                provider: "token-metadata",
                message,
                // Cached for a minute; the next lookup may well succeed.
                retryable: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use governor::Quota;
    use governor::RateLimiter;
    use std::num::NonZeroU32;
    use std::sync::atomic::{AtomicUsize, Ordering};

    enum Outcome {
        Resolved(Option<u8>, Option<String>),
        Failed,
    }

    struct CountingAdapter {
        calls: AtomicUsize,
        outcome: Outcome,
    }

    impl CountingAdapter {
        fn new(outcome: Outcome) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                outcome,
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl ChainAdapter for CountingAdapter {
        fn name(&self) -> &'static str {
            "fake"
        }

        async fn token_metadata(
            &self,
            _token_address: &str,
        ) -> Result<(Option<u8>, Option<String>), AppError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.outcome {
                Outcome::Resolved(decimals, symbol) => Ok((*decimals, symbol.clone())),
                Outcome::Failed => Err(AppError::UpstreamProvider {
                    provider: "fake",
                    message: "boom".into(),
                    retryable: true,
                }),
            }
        }
    }

    fn limiter(quota: Quota) -> Arc<Limiter> {
        Arc::new(RateLimiter::direct(quota))
    }

    fn one_per_second() -> Quota {
        Quota::per_second(NonZeroU32::new(1).unwrap())
    }

    #[tokio::test]
    async fn two_lookups_of_a_mint_make_one_upstream_call_and_the_hit_needs_no_token() {
        // One token per second: the first lookup spends it, so a cache hit that
        // reached the limiter would be shed and answer None. The hit still returns
        // the cached value.
        let lookup = TokenMetadataLookup::new(limiter(one_per_second()));
        let adapter = CountingAdapter::new(Outcome::Resolved(Some(6), Some("USDC".into())));

        let first = lookup.resolve(1, "mint", &adapter).await.unwrap();
        let second = lookup.resolve(1, "mint", &adapter).await.unwrap();

        assert_eq!(first, Some((Some(6), Some("USDC".into()))));
        assert_eq!(second, first);
        assert_eq!(adapter.calls(), 1);
    }

    #[tokio::test]
    async fn a_failure_is_cached_briefly_and_not_re_called_immediately() {
        let lookup =
            TokenMetadataLookup::new(limiter(Quota::per_second(NonZeroU32::new(100).unwrap())));
        let adapter = CountingAdapter::new(Outcome::Failed);

        assert!(lookup.resolve(1, "mint", &adapter).await.is_err());
        assert!(lookup.resolve(1, "mint", &adapter).await.is_err());

        assert_eq!(adapter.calls(), 1);
        assert_eq!(ttl_for(&CachedMetadata::Failed("boom".into())), RETRY_TTL);
    }

    #[test]
    fn a_resolved_mint_is_kept_for_a_day_and_an_empty_answer_for_a_minute() {
        assert_eq!(
            ttl_for(&CachedMetadata::Resolved {
                decimals: Some(6),
                symbol: None,
            }),
            SUCCESS_TTL
        );
        assert_eq!(
            ttl_for(&CachedMetadata::Resolved {
                decimals: None,
                symbol: None,
            }),
            RETRY_TTL
        );
    }

    #[tokio::test]
    async fn an_exhausted_limiter_sheds_immediately_and_is_not_cached() {
        let quota = Quota::with_period(Duration::from_millis(100)).unwrap();
        let limiter = limiter(quota);
        limiter.check().expect("the burst token is free");
        let lookup = TokenMetadataLookup::new(limiter);
        let adapter = CountingAdapter::new(Outcome::Resolved(Some(9), Some("SOL".into())));

        let started = std::time::Instant::now();
        let shed = lookup.resolve(1, "mint", &adapter).await.unwrap();
        let waited = started.elapsed();

        assert_eq!(shed, None);
        assert_eq!(adapter.calls(), 0);
        assert!(
            waited < Duration::from_secs(1),
            "shed after only {waited:?}"
        );

        // The empty answer was not cached: once the next token arrives the same
        // mint is resolved (and counted) upstream.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let resolved = lookup.resolve(1, "mint", &adapter).await.unwrap();
        assert_eq!(resolved, Some((Some(9), Some("SOL".into()))));
        assert_eq!(adapter.calls(), 1);
    }
}
