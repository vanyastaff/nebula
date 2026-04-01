//! Retry logic with exponential backoff — delegates to [`nebula_resilience`].
//!
//! [`RetryPolicy`] is a config facade that builds
//! [`nebula_resilience::RetryConfig`] internally.

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::Duration;

use nebula_resilience::retry::{BackoffConfig, JitterConfig, RetryConfig};

/// Retry policy configuration for exponential backoff
///
/// Controls retry behavior for transient failures in cloud storage operations.
/// Uses exponential backoff with optional jitter to prevent thundering herd.
///
/// # Example
///
/// ```rust
/// use nebula_credential::retry::RetryPolicy;
/// use std::time::Duration;
///
/// // Default policy: 5 retries, 100ms base delay, 2x multiplier
/// let policy = RetryPolicy::default();
///
/// // Custom policy
/// let policy = RetryPolicy {
///     max_retries: 3,
///     base_delay_ms: 200,
///     max_delay_ms: 10_000,
///     multiplier: 1.5,
///     jitter: true,
/// };
///
/// assert!(policy.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts
    ///
    /// Must be between 0 and 10. Default: 5
    pub max_retries: u32,

    /// Initial delay in milliseconds
    ///
    /// Must be between 10ms and 10,000ms. Default: 100ms
    pub base_delay_ms: u64,

    /// Maximum delay in milliseconds (cap for exponential growth)
    ///
    /// Must be greater than base_delay_ms. Default: 30,000ms (30 seconds)
    pub max_delay_ms: u64,

    /// Backoff multiplier (exponential growth factor)
    ///
    /// Must be >= 1.0 and <= 10.0. Default: 2.0 (doubles each retry)
    pub multiplier: f64,

    /// Add jitter to prevent thundering herd
    ///
    /// When true, adds ±25% randomness to delays. Default: true
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay_ms: 100,
            max_delay_ms: 30_000,
            multiplier: 2.0,
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// Validate retry policy parameters
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Policy is valid
    /// * `Err(String)` - Validation error with description
    ///
    /// # Validation Rules
    ///
    /// - max_retries: 0-10
    /// - base_delay_ms: 10-10,000
    /// - max_delay_ms > base_delay_ms
    /// - multiplier: 1.0-10.0
    pub fn validate(&self) -> Result<(), String> {
        if self.max_retries > 10 {
            return Err(format!(
                "max_retries must be <= 10, got {}",
                self.max_retries
            ));
        }

        if self.base_delay_ms < 10 {
            return Err(format!(
                "base_delay_ms must be >= 10, got {}",
                self.base_delay_ms
            ));
        }

        if self.base_delay_ms > 10_000 {
            return Err(format!(
                "base_delay_ms must be <= 10,000, got {}",
                self.base_delay_ms
            ));
        }

        if self.max_delay_ms <= self.base_delay_ms {
            return Err(format!(
                "max_delay_ms ({}) must be > base_delay_ms ({})",
                self.max_delay_ms, self.base_delay_ms
            ));
        }

        if self.multiplier < 1.0 {
            return Err(format!(
                "multiplier must be >= 1.0, got {}",
                self.multiplier
            ));
        }

        if self.multiplier > 10.0 {
            return Err(format!(
                "multiplier must be <= 10.0, got {}",
                self.multiplier
            ));
        }

        Ok(())
    }

    /// Build a [`RetryConfig`] from this policy.
    fn to_retry_config<E: 'static>(&self) -> RetryConfig<E> {
        // max_retries is the number of retries; RetryConfig.max_attempts includes the initial try.
        let max_attempts = self.max_retries + 1;
        let jitter = if self.jitter {
            JitterConfig::Full {
                factor: 0.25,
                seed: None,
            }
        } else {
            JitterConfig::None
        };

        RetryConfig::new(max_attempts)
            .expect("max_retries validated at construction")
            .backoff(BackoffConfig::Exponential {
                base: Duration::from_millis(self.base_delay_ms),
                multiplier: self.multiplier,
                max: Duration::from_millis(self.max_delay_ms),
            })
            .jitter(jitter)
    }
}

/// Execute an async operation with retry logic.
///
/// Delegates to [`nebula_resilience::retry_with`] with exponential backoff.
/// The operation closure no longer receives an attempt number — use
/// [`on_retry`](RetryConfig::on_retry) for per-attempt logging.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::retry::{RetryPolicy, retry_with_policy};
///
/// let policy = RetryPolicy::default();
/// let result = retry_with_policy(&policy, || async {
///     Ok::<_, MyError>("success")
/// }).await;
/// assert!(result.is_ok());
/// ```
pub async fn retry_with_policy<F, Fut, T, E>(policy: &RetryPolicy, f: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>> + Send,
    E: nebula_error::Classify + std::fmt::Display + Send + 'static,
{
    let config =
        policy
            .to_retry_config::<E>()
            .on_retry(|err: &E, delay: Duration, attempt: u32| {
                tracing::warn!(
                    attempt,
                    delay_ms = delay.as_millis(),
                    error = %err,
                    "Operation failed, retrying after delay"
                );
            });

    nebula_resilience::retry_with(config, f)
        .await
        .map_err(|call_err| match call_err {
            nebula_resilience::CallError::Operation(e)
            | nebula_resilience::CallError::RetriesExhausted { last: e, .. } => e,
            _ => unreachable!("retry_with only returns Operation or RetriesExhausted"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_policy() {
        let policy = RetryPolicy::default();

        assert_eq!(policy.max_retries, 5);
        assert_eq!(policy.base_delay_ms, 100);
        assert_eq!(policy.max_delay_ms, 30_000);
        assert_eq!(policy.multiplier, 2.0);
        assert!(policy.jitter);

        assert!(policy.validate().is_ok());
    }

    #[test]
    fn test_validate_max_retries() {
        let mut policy = RetryPolicy {
            max_retries: 0,
            ..Default::default()
        };
        assert!(policy.validate().is_ok());

        policy.max_retries = 10;
        assert!(policy.validate().is_ok());

        policy.max_retries = 11;
        assert!(policy.validate().is_err());
    }

    #[test]
    fn test_validate_base_delay() {
        let mut policy = RetryPolicy {
            base_delay_ms: 9,
            ..Default::default()
        };
        assert!(policy.validate().is_err());

        policy.base_delay_ms = 10;
        assert!(policy.validate().is_ok());

        policy.base_delay_ms = 10_000;
        assert!(policy.validate().is_ok());

        policy.base_delay_ms = 10_001;
        assert!(policy.validate().is_err());
    }

    #[test]
    fn test_validate_max_delay() {
        let mut policy = RetryPolicy {
            max_delay_ms: 50,
            ..Default::default()
        }; // Less than base_delay_ms (100)
        assert!(policy.validate().is_err());

        policy.max_delay_ms = 101;
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn test_validate_multiplier() {
        let mut policy = RetryPolicy {
            multiplier: 0.9,
            ..Default::default()
        };
        assert!(policy.validate().is_err());

        policy.multiplier = 1.0;
        assert!(policy.validate().is_ok());

        policy.multiplier = 10.0;
        assert!(policy.validate().is_ok());

        policy.multiplier = 10.1;
        assert!(policy.validate().is_err());
    }

    // Backoff calculation and jitter tests are covered by nebula-resilience.
    // These tests verify the retry_with_policy integration.

    /// Test error implementing Classify.
    #[derive(Debug, Clone, PartialEq)]
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
    async fn retry_with_policy_success_first_attempt() {
        let policy = RetryPolicy::default();

        let result: Result<&str, TestErr> =
            retry_with_policy(&policy, || async { Ok("success") }).await;

        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn retry_with_policy_success_after_retries() {
        let policy = RetryPolicy {
            max_retries: 3,
            base_delay_ms: 10,
            max_delay_ms: 100,
            multiplier: 2.0,
            jitter: false,
        };

        let counter = std::sync::atomic::AtomicU32::new(0);

        let result: Result<&str, TestErr> = retry_with_policy(&policy, || {
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async move {
                if n < 2 {
                    Err(TestErr("transient"))
                } else {
                    Ok("success")
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), "success");
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_with_policy_exhausted() {
        let policy = RetryPolicy {
            max_retries: 2,
            base_delay_ms: 10,
            max_delay_ms: 100,
            multiplier: 2.0,
            jitter: false,
        };

        let counter = std::sync::atomic::AtomicU32::new(0);

        let result: Result<&str, TestErr> = retry_with_policy(&policy, || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Err(TestErr("permanent")) }
        })
        .await;

        assert_eq!(result.unwrap_err(), TestErr("permanent"));
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 3); // Initial + 2 retries
    }
}
