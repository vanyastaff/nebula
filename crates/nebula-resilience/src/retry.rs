//! Retry strategies for resilient operations

use futures::Future;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, warn};

use crate::error::ResilienceError;

/// Trait for determining if an error is retryable
pub trait Retryable {
    /// Check if the error should trigger a retry
    fn is_retryable(&self) -> bool;

    /// Check if the error is terminal (should not be retried)
    fn is_terminal(&self) -> bool;
}

impl<T> Retryable for Result<T, ResilienceError> {
    fn is_retryable(&self) -> bool {
        self.as_ref().err().is_some_and(ResilienceError::is_retryable)
    }

    fn is_terminal(&self) -> bool {
        self.as_ref().err().is_some_and(ResilienceError::is_terminal)
    }
}

impl Retryable for ResilienceError {
    fn is_retryable(&self) -> bool {
        self.is_retryable()
    }

    fn is_terminal(&self) -> bool {
        self.is_terminal()
    }
}

/// Retry strategy configuration
#[derive(Debug, Clone)]
pub struct RetryStrategy {
    /// Maximum number of retry attempts
    pub max_attempts: usize,
    /// Base delay between retries
    pub base_delay: Duration,
    /// Maximum delay cap
    pub max_delay: Duration,
    /// Jitter factor (0.0 = no jitter, 1.0 = full jitter)
    pub jitter_factor: f64,
    /// Whether to use exponential backoff
    pub exponential: bool,
}

impl Default for RetryStrategy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            jitter_factor: 0.1,
            exponential: true,
        }
    }
}

impl RetryStrategy {
    /// Create a new retry strategy
    #[must_use] pub fn new(max_attempts: usize, base_delay: Duration) -> Self {
        Self {
            max_attempts,
            base_delay,
            max_delay: base_delay * 60, // 60x base delay as default max
            jitter_factor: 0.1,
            exponential: true,
        }
    }

    /// Set the maximum delay cap
    #[must_use] pub fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    /// Set the jitter factor
    #[must_use] pub fn with_jitter(mut self, jitter_factor: f64) -> Self {
        self.jitter_factor = jitter_factor.clamp(0.0, 1.0);
        self
    }

    /// Disable exponential backoff
    #[must_use] pub fn without_exponential(mut self) -> Self {
        self.exponential = false;
        self
    }

    /// Calculate delay for a specific attempt
    fn calculate_delay(&self, attempt: usize) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }

        let mut delay = if self.exponential {
            self.base_delay * 2_u32.pow(attempt as u32 - 1)
        } else {
            self.base_delay
        };

        // Apply jitter (deterministic based on attempt to avoid external deps)
        if self.jitter_factor > 0.0 {
            let jitter_range = (delay.as_millis() as f64 * self.jitter_factor) as u64;
            let jitter = (attempt as u64) % (jitter_range + 1);
            delay = Duration::from_millis(delay.as_millis() as u64 + jitter);
        }

        // Cap at maximum delay
        delay.min(self.max_delay)
    }
}

/// Execute an operation with retry logic
///
/// # Arguments
///
/// * `strategy` - Retry strategy configuration
/// * `operation` - The async operation to retry
///
/// # Returns
///
/// * `Ok(T)` - Operation completed successfully
/// * `Err(ResilienceError::RetryLimitExceeded)` - All retry attempts failed
/// * `Err(E)` - Terminal error from the operation
pub async fn retry<T, E, F, Fut>(
    strategy: RetryStrategy,
    mut operation: F,
) -> Result<T, ResilienceError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    for attempt in 0..=strategy.max_attempts {
        match operation().await {
            Ok(value) => {
                if attempt > 0 {
                    debug!("Operation succeeded after {} retry attempts", attempt);
                }
                return Ok(value);
            }
            Err(error) => {
                if attempt < strategy.max_attempts {
                    let delay = strategy.calculate_delay(attempt + 1);
                    warn!(
                        "Operation failed (attempt {}/{}), retrying in {:?}: {:?}",
                        attempt + 1,
                        strategy.max_attempts,
                        delay,
                        error
                    );
                    sleep(delay).await;
                }
            }
        }
    }

    Err(ResilienceError::retry_limit_exceeded(strategy.max_attempts))
}

/// Execute an operation with default retry strategy
pub async fn retry_default<T, E, F, Fut>(operation: F) -> Result<T, ResilienceError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Debug,
{
    retry(RetryStrategy::default(), operation).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_retry_success_on_first_attempt() {
        let counter = AtomicUsize::new(0);
        let result = retry(RetryStrategy::new(3, Duration::from_millis(10)), || async {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok::<&str, &str>("success")
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_success_after_failures() {
        let counter = AtomicUsize::new(0);
        let result = retry(RetryStrategy::new(3, Duration::from_millis(10)), || async {
            let attempt = counter.fetch_add(1, Ordering::SeqCst);
            if attempt < 2 {
                Err::<&str, &str>("temporary failure")
            } else {
                Ok("success")
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_retry_limit_exceeded() {
        let counter = AtomicUsize::new(0);
        let result = retry(RetryStrategy::new(2, Duration::from_millis(10)), || async {
            counter.fetch_add(1, Ordering::SeqCst);
            Err::<&str, &str>("persistent failure")
        })
        .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ResilienceError::RetryLimitExceeded { attempts } => {
                assert_eq!(attempts, 2);
            }
            _ => panic!("Expected retry limit exceeded error"),
        }
        assert_eq!(counter.load(Ordering::SeqCst), 3); // Initial + 2 retries
    }

    #[test]
    fn test_retry_strategy_default() {
        let strategy = RetryStrategy::default();
        assert_eq!(strategy.max_attempts, 3);
        assert_eq!(strategy.base_delay, Duration::from_secs(1));
        assert_eq!(strategy.max_delay, Duration::from_secs(60));
        assert_eq!(strategy.jitter_factor, 0.1);
        assert!(strategy.exponential);
    }

    #[test]
    fn test_retry_strategy_builder() {
        let strategy = RetryStrategy::new(5, Duration::from_secs(2))
            .with_max_delay(Duration::from_secs(30))
            .with_jitter(0.5)
            .without_exponential();

        assert_eq!(strategy.max_attempts, 5);
        assert_eq!(strategy.base_delay, Duration::from_secs(2));
        assert_eq!(strategy.max_delay, Duration::from_secs(30));
        assert_eq!(strategy.jitter_factor, 0.5);
        assert!(!strategy.exponential);
    }

    #[test]
    fn test_calculate_delay() {
        let strategy = RetryStrategy::new(3, Duration::from_secs(1));

        // First attempt (no delay)
        assert_eq!(strategy.calculate_delay(0), Duration::ZERO);

        // Second attempt (1s base)
        assert_eq!(strategy.calculate_delay(1), Duration::from_secs(1));

        // Third attempt (2s exponential)
        assert_eq!(strategy.calculate_delay(2), Duration::from_secs(2));
    }
}
