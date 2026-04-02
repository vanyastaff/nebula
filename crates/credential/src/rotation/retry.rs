//! Retry logic with exponential backoff — delegates to [`nebula_resilience`].
//!
//! [`RotationRetryPolicy`] is a config facade that builds
//! [`nebula_resilience::RetryConfig`] internally.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use nebula_resilience::retry::{BackoffConfig, JitterConfig, RetryConfig};

use super::error::RotationError;

/// Retry policy configuration for rotation operations.
///
/// Wraps [`nebula_resilience::RetryConfig`] with rotation-specific defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationRetryPolicy {
    /// Maximum number of retry attempts
    pub max_attempts: u32,

    /// Initial backoff duration
    #[serde(with = "humantime_serde")]
    pub initial_backoff: Duration,

    /// Backoff multiplier (typically 2.0 for exponential)
    pub backoff_multiplier: f32,

    /// Maximum backoff duration
    #[serde(with = "humantime_serde")]
    pub max_backoff: Duration,
}

impl Default for RotationRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_backoff: Duration::from_millis(100),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(32),
        }
    }
}

impl RotationRetryPolicy {
    /// Create a new retry policy with custom parameters
    pub fn new(
        max_attempts: u32,
        initial_backoff: Duration,
        backoff_multiplier: f32,
        max_backoff: Duration,
    ) -> Self {
        Self {
            max_attempts,
            initial_backoff,
            backoff_multiplier,
            max_backoff,
        }
    }

    /// Build a [`RetryConfig`] from this policy.
    fn to_retry_config<E: 'static>(&self) -> RetryConfig<E> {
        RetryConfig::new(self.max_attempts)
            .expect("max_attempts validated at construction")
            .backoff(BackoffConfig::Exponential {
                base: self.initial_backoff,
                multiplier: f64::from(self.backoff_multiplier),
                max: self.max_backoff,
            })
            .jitter(JitterConfig::Full {
                factor: 0.1,
                seed: None,
            })
    }
}

/// Retry an async operation with exponential backoff.
///
/// Delegates to [`nebula_resilience::retry_with`] when `E: Classify`,
/// or uses a retry-all predicate for generic error types.
///
/// # Example
///
/// ```rust,ignore
/// let policy = RotationRetryPolicy::default();
/// let result = retry_with_backoff(&policy, "database_connection", || async {
///     database.connect().await
/// }).await?;
/// ```
pub async fn retry_with_backoff<F, Fut, T, E>(
    policy: &RotationRetryPolicy,
    operation_name: &str,
    f: F,
) -> Result<T, RotationError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>> + Send,
    E: std::fmt::Display + nebula_error::Classify + Send + 'static,
{
    let op_name = operation_name.to_owned();
    let max = policy.max_attempts;

    let config = policy
        .to_retry_config::<E>()
        .retry_if(|_: &E| true)
        .on_retry(move |err: &E, delay: Duration, attempt: u32| {
            tracing::warn!(
                operation = %op_name,
                attempt,
                delay_ms = delay.as_millis(),
                error = %err,
                "Retry attempt failed"
            );
        });

    nebula_resilience::retry_with(config, f).await.map_err(|e| {
        let last_msg = e
            .into_operation()
            .map(|e| format!(" (last error: {e})"))
            .unwrap_or_default();
        RotationError::MaxRetriesExceeded {
            operation: format!("{operation_name}{last_msg}"),
            max_attempts: max,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Test error that implements Classify (retryable by default).
    #[derive(Debug)]
    struct TestErr(&'static str);
    impl std::fmt::Display for TestErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }
    impl nebula_error::Classify for TestErr {
        fn category(&self) -> nebula_error::ErrorCategory {
            nebula_error::ErrorCategory::External
        }
        fn code(&self) -> nebula_error::ErrorCode {
            nebula_error::codes::INTERNAL
        }
    }

    #[tokio::test]
    async fn retry_success_on_first_attempt() {
        let policy = RotationRetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(10),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(1),
        };

        let counter = AtomicU32::new(0);

        let result = retry_with_backoff(&policy, "test_op", || async {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<i32, TestErr>(42)
        })
        .await
        .unwrap();

        assert_eq!(result, 42);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retry_success_on_second_attempt() {
        let policy = RotationRetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(10),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(1),
        };

        let counter = AtomicU32::new(0);

        let result = retry_with_backoff(&policy, "test_op", || async {
            let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
            if count == 1 {
                Err(TestErr("first attempt fails"))
            } else {
                Ok::<i32, TestErr>(42)
            }
        })
        .await
        .unwrap();

        assert_eq!(result, 42);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retry_max_attempts_exceeded() {
        let policy = RotationRetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(10),
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(1),
        };

        let counter = AtomicU32::new(0);

        let result = retry_with_backoff(&policy, "test_op", || async {
            counter.fetch_add(1, Ordering::SeqCst);
            Err::<i32, TestErr>(TestErr("always fails"))
        })
        .await;

        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 3);
        assert!(matches!(
            result.unwrap_err(),
            RotationError::MaxRetriesExceeded {
                max_attempts: 3,
                ..
            }
        ));
    }
}
