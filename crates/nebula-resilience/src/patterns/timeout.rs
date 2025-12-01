//! Timeout management for async operations
//!
//! This module provides timeout functionality for limiting operation duration.

use futures::Future;
use std::time::Duration;
use tokio::time::{Timeout, timeout as tokio_timeout};

use crate::{ResilienceError, ResilienceResult};

// =============================================================================
// TIMEOUT FUNCTIONS
// =============================================================================

/// Execute a future with a timeout
///
/// This is the primary timeout wrapper for all I/O operations.
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
/// use nebula_resilience::timeout_fn;
/// use std::time::Duration;
///
/// async fn example() -> Result<(), Box<dyn std::error::Error>> {
///     let result = timeout_fn(
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
/// Useful when you need to pass the timeout-wrapped future to other functions.
pub fn with_timeout<T, F>(duration: Duration, future: F) -> Timeout<F>
where
    F: Future<Output = T>,
{
    tokio_timeout(duration, future)
}

/// Execute a future with a timeout, converting errors to `ResilienceError`
///
/// This variant converts both timeout and operation errors into `ResilienceError`.
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

// =============================================================================
// TIMEOUT POLICIES
// =============================================================================

/// Marker trait for timeout policies.
pub trait TimeoutPolicy: Send + Sync + 'static {
    /// Policy name for observability.
    fn name() -> &'static str;

    /// Default timeout for this policy.
    fn default_timeout() -> Duration;
}

/// Strict timeout policy - fails fast on timeout.
#[derive(Debug, Clone, Copy)]
pub struct StrictPolicy;

impl TimeoutPolicy for StrictPolicy {
    fn name() -> &'static str {
        "strict"
    }
    fn default_timeout() -> Duration {
        Duration::from_secs(5)
    }
}

/// Lenient timeout policy - longer timeouts for slow operations.
#[derive(Debug, Clone, Copy)]
pub struct LenientPolicy;

impl TimeoutPolicy for LenientPolicy {
    fn name() -> &'static str {
        "lenient"
    }
    fn default_timeout() -> Duration {
        Duration::from_secs(60)
    }
}

/// Adaptive timeout policy - adjusts based on operation type.
#[derive(Debug, Clone, Copy)]
pub struct AdaptivePolicy;

impl TimeoutPolicy for AdaptivePolicy {
    fn name() -> &'static str {
        "adaptive"
    }
    fn default_timeout() -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
