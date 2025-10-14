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

/// Execute a future with a timeout, converting errors to `ResilienceError`
///
/// This variant converts both timeout and operation errors into `ResilienceError`.
/// When the operation completes with an error, it's wrapped as a `Custom` error.
/// When the timeout fires, it returns a `Timeout` error.
///
/// # Arguments
///
/// * `duration` - Maximum time to wait for the operation
/// * `future` - The async operation to execute
///
/// # Returns
///
/// * `Ok(T)` - Operation completed successfully within timeout
/// * `Err(ResilienceError::Custom)` - Original error from the operation (if operation completed)
/// * `Err(ResilienceError::Timeout)` - Operation exceeded the timeout
pub async fn timeout_with_original_error<T, E, F>(
    duration: Duration,
    future: F,
) -> Result<T, ResilienceError>
where
    F: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    match tokio_timeout(duration, future).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(e)) => Err(ResilienceError::Custom {
            message: format!("Operation failed: {e}"),
            retryable: false,
            source: None,
        }),
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
    async fn test_timeout_with_original_error_operation_failure() {
        let result = timeout_with_original_error(Duration::from_millis(100), async {
            Err::<&str, &str>("operation failed")
        })
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ResilienceError::Custom { message, .. } => {
                // Original error should be preserved as a Custom error
                assert!(message.contains("operation failed"));
            }
            other => panic!("Expected Custom error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_timeout_with_original_error_timeout_exceeded() {
        let result = timeout_with_original_error(Duration::from_millis(10), async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Err::<&str, &str>("should not reach here")
        })
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ResilienceError::Timeout { duration, .. } => {
                assert_eq!(duration, Duration::from_millis(10));
            }
            other => panic!("Expected Timeout error, got: {:?}", other),
        }
    }
}
