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

/// Runs an upstream call behind a rate limiter, retrying transient failures
/// with exponential backoff + jitter. Every attempt (including retries) goes
/// through the limiter, so retries can't exceed the provider's rate limit.
pub async fn call_upstream<T, F, Fut>(limiter: &Limiter, mut f: F) -> Result<T, AppError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AppError>>,
{
    let mut delay = Duration::from_millis(200);
    let mut attempt = 0;
    loop {
        // fail fast instead of queueing forever
        tokio::time::timeout(Duration::from_secs(2), limiter.until_ready())
            .await
            .map_err(|_| AppError::Overloaded)?;

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
