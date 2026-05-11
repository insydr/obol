use std::time::Duration;
use tokio::time::sleep;

use crate::error::{CharonError, Result};

/// Retry an async operation with exponential backoff.  The closure `f` is
/// called up to `max_retries` times.  On each failure the wait time
/// doubles, starting from 500 ms.
pub async fn retry_with_backoff<F, Fut, T>(max_retries: u32, f: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempt = 0;
    let mut delay_ms = 500;

    loop {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                attempt += 1;
                if attempt >= max_retries {
                    return Err(e);
                }
                tracing::warn!(
                    attempt = attempt,
                    max_retries = max_retries,
                    error = %e,
                    "Retrying after failure"
                );
                sleep(Duration::from_millis(delay_ms)).await;
                delay_ms *= 2;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_retry_succeeds_on_first_try() {
        let result = retry_with_backoff(3, || async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_succeeds_after_failures() {
        let mut attempts = 0;
        let result = retry_with_backoff(3, || {
            attempts += 1;
            async move {
                if attempts < 3 {
                    Err(CharonError::Internal("fail".to_string()))
                } else {
                    Ok(attempts)
                }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 3);
    }

    #[tokio::test]
    async fn test_retry_exhausts_attempts() {
        let result: Result<i32> = retry_with_backoff(2, || async {
            Err(CharonError::Internal("always fail".to_string()))
        })
        .await;
        assert!(result.is_err());
    }
}
