//! Retry logic with exponential backoff
//!
//! Provides retry policies and execution utilities for cloud storage providers.

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::Duration;

/// Retry policy configuration for exponential backoff
///
/// Controls retry behavior for transient failures in cloud storage operations.
/// Uses exponential backoff with optional jitter to prevent thundering herd.
///
/// # Example
///
/// ```rust
/// use nebula_credential::utils::RetryPolicy;
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

    /// Calculate delay for a given attempt number
    ///
    /// # Arguments
    ///
    /// * `attempt` - Retry attempt number (0-based)
    ///
    /// # Returns
    ///
    /// Duration to wait before this attempt, capped at max_delay_ms
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        let delay_ms = self.base_delay_ms as f64 * self.multiplier.powi(attempt as i32);
        let capped_delay_ms = delay_ms.min(self.max_delay_ms as f64) as u64;

        Duration::from_millis(capped_delay_ms)
    }

    /// Apply jitter to a delay
    ///
    /// # Arguments
    ///
    /// * `delay` - Base delay duration
    ///
    /// # Returns
    ///
    /// Duration with ±25% jitter applied (if jitter is enabled)
    pub fn apply_jitter(&self, delay: Duration) -> Duration {
        if !self.jitter {
            return delay;
        }

        use rand::Rng;
        let mut rng = rand::rng();

        // Apply ±25% jitter
        let delay_ms = delay.as_millis() as f64;
        let jitter_range = delay_ms * 0.25;
        let jitter = rng.random_range(-jitter_range..=jitter_range);

        let jittered_ms = (delay_ms + jitter).max(0.0) as u64;
        Duration::from_millis(jittered_ms)
    }
}

/// Execute an async operation with retry logic
///
/// Automatically retries transient failures using exponential backoff with jitter.
/// Logs retry attempts using tracing.
///
/// # Arguments
///
/// * `policy` - Retry policy configuration
/// * `operation` - Async function to execute (receives attempt number)
///
/// # Returns
///
/// * `Ok(T)` - Operation succeeded
/// * `Err(E)` - Operation failed after all retries exhausted
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::utils::{RetryPolicy, retry_with_policy};
///
/// async fn flaky_operation(attempt: u32) -> Result<String, String> {
///     if attempt < 2 {
///         Err("Transient error".into())
///     } else {
///         Ok("Success".into())
///     }
/// }
///
/// let policy = RetryPolicy::default();
/// let result = retry_with_policy(&policy, flaky_operation).await;
/// assert!(result.is_ok());
/// ```
pub async fn retry_with_policy<F, Fut, T, E>(policy: &RetryPolicy, mut operation: F) -> Result<T, E>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut last_error = None;

    for attempt in 0..=policy.max_retries {
        // Try operation
        match operation(attempt).await {
            Ok(result) => {
                if attempt > 0 {
                    tracing::info!(attempt = attempt, "Operation succeeded after retry");
                }
                return Ok(result);
            }
            Err(err) => {
                if attempt < policy.max_retries {
                    // Calculate delay with jitter
                    let base_delay = policy.calculate_delay(attempt);
                    let delay = policy.apply_jitter(base_delay);

                    tracing::warn!(
                        attempt = attempt,
                        delay_ms = delay.as_millis(),
                        error = %err,
                        "Operation failed, retrying after delay"
                    );

                    tokio::time::sleep(delay).await;
                    last_error = Some(err);
                } else {
                    tracing::error!(
                        attempt = attempt,
                        error = %err,
                        "Operation failed after all retries exhausted"
                    );
                    return Err(err);
                }
            }
        }
    }

    // This should never happen due to the loop structure, but handle it anyway
    Err(last_error.expect("Should have last error"))
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
        assert_eq!(policy.jitter, true);

        assert!(policy.validate().is_ok());
    }

    #[test]
    fn test_validate_max_retries() {
        let mut policy = RetryPolicy::default();

        policy.max_retries = 0;
        assert!(policy.validate().is_ok());

        policy.max_retries = 10;
        assert!(policy.validate().is_ok());

        policy.max_retries = 11;
        assert!(policy.validate().is_err());
    }

    #[test]
    fn test_validate_base_delay() {
        let mut policy = RetryPolicy::default();

        policy.base_delay_ms = 9;
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
        let mut policy = RetryPolicy::default();

        policy.max_delay_ms = 50; // Less than base_delay_ms (100)
        assert!(policy.validate().is_err());

        policy.max_delay_ms = 101;
        assert!(policy.validate().is_ok());
    }

    #[test]
    fn test_validate_multiplier() {
        let mut policy = RetryPolicy::default();

        policy.multiplier = 0.9;
        assert!(policy.validate().is_err());

        policy.multiplier = 1.0;
        assert!(policy.validate().is_ok());

        policy.multiplier = 10.0;
        assert!(policy.validate().is_ok());

        policy.multiplier = 10.1;
        assert!(policy.validate().is_err());
    }

    #[test]
    fn test_calculate_delay() {
        let policy = RetryPolicy {
            max_retries: 5,
            base_delay_ms: 100,
            max_delay_ms: 1_000,
            multiplier: 2.0,
            jitter: false,
        };

        // Attempt 0: 100ms * 2^0 = 100ms
        assert_eq!(policy.calculate_delay(0).as_millis(), 100);

        // Attempt 1: 100ms * 2^1 = 200ms
        assert_eq!(policy.calculate_delay(1).as_millis(), 200);

        // Attempt 2: 100ms * 2^2 = 400ms
        assert_eq!(policy.calculate_delay(2).as_millis(), 400);

        // Attempt 3: 100ms * 2^3 = 800ms
        assert_eq!(policy.calculate_delay(3).as_millis(), 800);

        // Attempt 4: 100ms * 2^4 = 1600ms, capped at 1000ms
        assert_eq!(policy.calculate_delay(4).as_millis(), 1_000);
    }

    #[test]
    fn test_apply_jitter_disabled() {
        let policy = RetryPolicy {
            jitter: false,
            ..Default::default()
        };

        let delay = Duration::from_millis(100);
        let jittered = policy.apply_jitter(delay);

        assert_eq!(jittered, delay);
    }

    #[test]
    fn test_apply_jitter_enabled() {
        let policy = RetryPolicy {
            jitter: true,
            ..Default::default()
        };

        let delay = Duration::from_millis(100);

        // Run multiple times to test jitter range
        for _ in 0..10 {
            let jittered = policy.apply_jitter(delay);
            let jittered_ms = jittered.as_millis() as i64;

            // Should be within ±25% of 100ms (75-125ms)
            assert!(jittered_ms >= 75 && jittered_ms <= 125);
        }
    }

    #[tokio::test]
    async fn test_retry_with_policy_success_first_attempt() {
        let policy = RetryPolicy::default();

        let result =
            retry_with_policy(&policy, |_attempt| async { Ok::<_, String>("success") }).await;

        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn test_retry_with_policy_success_after_retries() {
        let policy = RetryPolicy {
            max_retries: 3,
            base_delay_ms: 10, // Short delay for test
            max_delay_ms: 100,
            multiplier: 2.0,
            jitter: false,
        };

        let mut attempt_count = 0;

        let result = retry_with_policy(&policy, |_attempt| {
            attempt_count += 1;
            async move {
                if attempt_count < 3 {
                    Err("transient error")
                } else {
                    Ok("success")
                }
            }
        })
        .await;

        assert_eq!(result.unwrap(), "success");
        assert_eq!(attempt_count, 3);
    }

    #[tokio::test]
    async fn test_retry_with_policy_exhausted() {
        let policy = RetryPolicy {
            max_retries: 2,
            base_delay_ms: 10,
            max_delay_ms: 100,
            multiplier: 2.0,
            jitter: false,
        };

        let mut attempt_count = 0;

        let result = retry_with_policy(&policy, |_attempt| {
            attempt_count += 1;
            async move { Err::<String, _>("permanent error") }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "permanent error");
        assert_eq!(attempt_count, 3); // Initial + 2 retries
    }
}
