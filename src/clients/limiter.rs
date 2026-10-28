use crate::error::AppError;
use governor::{
    RateLimiter,
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
};
use std::{future::Future, time::Duration};

pub type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

fn is_retryable(e: &AppError) -> bool {
    matches!(
        e,
        AppError::UpstreamRateLimited(_)
            | AppError::UpstreamTimeout(_)
            | AppError::UpstreamProvider { .. }
    )
}

/// Takes one token from the limiter, failing fast with `Overloaded` instead of
/// queueing forever. Lets a caller pay for an upstream call before starting any
/// other work that would be wasted if the wait times out.
pub async fn acquire_token(limiter: &Limiter) -> Result<(), AppError> {
    tokio::time::timeout(Duration::from_secs(2), limiter.until_ready())
        .await
        .map_err(|_| AppError::Overloaded)?;
    Ok(())
}

/// Runs an upstream call behind a rate limiter, retrying transient failures
/// with exponential backoff + jitter. Every attempt (including retries) goes
/// through the limiter, so retries can't exceed the provider's rate limit.
pub async fn call_upstream<T, F, Fut>(limiter: &Limiter, f: F) -> Result<T, AppError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AppError>>,
{
    call_upstream_inner(limiter, false, f).await
}

/// Like [`call_upstream`], but the first attempt runs without waiting on the
/// limiter: the caller already took its token with [`acquire_token`]. Retries
/// still go through the limiter.
pub async fn call_upstream_after_token<T, F, Fut>(limiter: &Limiter, f: F) -> Result<T, AppError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AppError>>,
{
    call_upstream_inner(limiter, true, f).await
}

async fn call_upstream_inner<T, F, Fut>(
    limiter: &Limiter,
    first_attempt_paid: bool,
    mut f: F,
) -> Result<T, AppError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AppError>>,
{
    let mut delay = Duration::from_millis(200);
    let mut attempt = 0;
    loop {
        if !(first_attempt_paid && attempt == 0) {
            acquire_token(limiter).await?;
        }

        match f().await {
            Err(e) if is_retryable(&e) && attempt < 3 => {
                let jitter = rand::random::<u64>() % (delay.as_millis() as u64 / 2 + 1);
                tokio::time::sleep(delay + Duration::from_millis(jitter)).await;
                delay *= 2;
                attempt += 1;
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use governor::Quota;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Instant;

    /// One token every 10 s: after the burst is spent, the next token is well
    /// beyond the 2 s fail-fast window.
    fn slow_limiter() -> Limiter {
        RateLimiter::direct(Quota::with_period(Duration::from_secs(10)).unwrap())
    }

    #[tokio::test]
    async fn acquire_token_sheds_before_the_call_starts() {
        let limiter = slow_limiter();
        limiter.check().expect("the burst token is available");

        // The caller only starts the upstream call when the token was taken.
        let called = AtomicBool::new(false);
        let started = Instant::now();
        let token = acquire_token(&limiter).await;
        if token.is_ok() {
            called.store(true, Ordering::SeqCst);
        }

        assert!(matches!(token, Err(AppError::Overloaded)));
        assert!(!called.load(Ordering::SeqCst));
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_secs(2),
            "shed after only {waited:?}"
        );
        assert!(
            waited < Duration::from_secs(5),
            "shed after {waited:?}, much later than the 2 s timeout"
        );
    }

    #[tokio::test]
    async fn retry_after_paid_first_attempt_still_waits_on_the_limiter() {
        let limiter = slow_limiter();
        acquire_token(&limiter).await.expect("token is available");

        let calls = AtomicUsize::new(0);
        let started = Instant::now();
        let result = call_upstream_after_token(&limiter, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), _>(AppError::UpstreamTimeout("test")) }
        })
        .await;

        // The paid first attempt runs; the retry must take a token, none arrives
        // within 2 s, so it sheds. The elapsed time includes the backoff first.
        assert!(matches!(result, Err(AppError::Overloaded)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_secs(2),
            "shed after only {waited:?}"
        );
        assert!(
            waited < Duration::from_secs(5),
            "shed after {waited:?}, much later than the 2 s timeout"
        );
    }

    #[tokio::test]
    async fn paid_first_attempt_does_not_wait_on_the_limiter() {
        let limiter = slow_limiter();
        acquire_token(&limiter).await.expect("token is available");

        let calls = AtomicUsize::new(0);
        let started = Instant::now();
        let result = call_upstream_after_token(&limiter, || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, AppError>("ok") }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the paid first attempt waited {:?}",
            started.elapsed()
        );
    }
}
