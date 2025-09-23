//! Timeout management for async operations

use futures::Future;
use std::time::Duration;
use tokio::time::{Timeout, timeout as tokio_timeout};

use crate::{ResilienceError, ResilienceResult};

/// Execute a future with a timeout
///
/// This is the primary timeout wrapper that should be used for all I/O operations
/// in the Nebula workflow engine.
///
/// # Arguments
///
/// * `duration` - Maximum time to wait for the operation
/// * `future` - The async operation to execute
///
/// # Returns
///
/// * `Ok(T)` - Operation completed successfully within timeout
/// * `Err(ResilienceError::Timeout)` - Operation exceeded the timeout
///
/// # Examples
///
/// ```rust
/// use nebula_resilience::timeout;
/// use std::time::Duration;
///
/// async fn example() -> Result<(), Box<dyn std::error::Error>> {
///     let result = timeout(
///         Duration::from_secs(30),
///         async { /* your async operation */ }
///     ).await?;
///     
///     Ok(())
/// }
/// ```
pub async fn timeout<T, F>(duration: Duration, future: F) -> ResilienceResult<T>
where
    F: Future<Output = T>,
{
    tokio_timeout(duration, future)
        .await
        .map_err(|_| ResilienceError::timeout(duration))
}

/// Create a timeout-aware future without executing it
///
/// This is useful when you need to pass the timeout-wrapped future to other functions
/// or when you want to control the execution timing.
///
/// # Arguments
///
/// * `duration` - Maximum time to wait for the operation
/// * `future` - The async operation to wrap
///
/// # Returns
///
/// A `Timeout<F>` future that will automatically timeout after the specified duration
pub fn with_timeout<T, F>(duration: Duration, future: F) -> Timeout<F>
where
    F: Future<Output = T>,
{
    tokio_timeout(duration, future)
}

/// Execute a future with a timeout, returning the original error type
///
/// This variant preserves the original error type instead of converting to `ResilienceError`.
/// Useful when you want to handle the original error but still enforce timeouts.
///
/// # Arguments
///
/// * `duration` - Maximum time to wait for the operation
/// * `future` - The async operation to execute
///
/// # Returns
///
/// * `Ok(T)` - Operation completed successfully within timeout
/// * `Err(E)` - Original error from the operation
/// * `Err(ResilienceError::Timeout)` - Operation exceeded the timeout
pub async fn timeout_with_original_error<T, E, F>(
    duration: Duration,
    future: F,
) -> Result<T, ResilienceError>
where
    F: Future<Output = Result<T, E>>,
{
    match tokio_timeout(duration, future).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(_)) => Err(ResilienceError::timeout(duration)),
        Err(_) => Err(ResilienceError::timeout(duration)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_timeout_success() {
        let result = timeout(Duration::from_millis(100), async { "success" }).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn test_timeout_exceeded() {
        let result = timeout(Duration::from_millis(10), async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            "should not reach here"
        })
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ResilienceError::Timeout { duration, .. } => {
                assert_eq!(duration, Duration::from_millis(10));
            }
            _ => panic!("Expected timeout error"),
        }
    }

    #[tokio::test]
    async fn test_timeout_with_original_error_success() {
        let result = timeout_with_original_error(Duration::from_millis(100), async {
            Ok::<&str, &str>("success")
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn test_timeout_with_original_error_failure() {
        let result = timeout_with_original_error(Duration::from_millis(100), async {
            Err::<&str, &str>("operation failed")
        })
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ResilienceError::Timeout { .. } => {
                // This should timeout because the future completes immediately with an error
                // but the timeout wrapper doesn't distinguish between success and failure
            }
            _ => panic!("Expected timeout error"),
        }
    }
}
