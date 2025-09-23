//! Retry strategies for resilient operations

use std::future::Future;
use std::time::Duration;
use tokio::time::sleep;
use nebula_log::{debug, warn};

use crate::{ResilienceError, ResilienceResult, ErrorClass};

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
    /// Custom retry predicate
    pub retry_predicate: Option<fn(&ResilienceError) -> bool>,
}

impl Default for RetryStrategy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            jitter_factor: 0.1,
            exponential: true,
            retry_predicate: None,
        }
    }
}

impl RetryStrategy {
    /// Create a new retry strategy
    pub fn new(max_attempts: usize, base_delay: Duration) -> Self {
        Self {
            max_attempts,
            base_delay,
            max_delay: base_delay.saturating_mul(60),
            jitter_factor: 0.1,
            exponential: true,
            retry_predicate: None,
        }
    }

    /// Set the maximum delay cap
    pub fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    /// Set the jitter factor
    pub fn with_jitter(mut self, jitter_factor: f64) -> Self {
        self.jitter_factor = jitter_factor.clamp(0.0, 1.0);
        self
    }

    /// Disable exponential backoff
    pub fn without_exponential(mut self) -> Self {
        self.exponential = false;
        self
    }

    /// Set custom retry predicate
    pub fn with_predicate(mut self, predicate: fn(&ResilienceError) -> bool) -> Self {
        self.retry_predicate = Some(predicate);
        self
    }

    /// Check if an error should be retried
    pub fn should_retry(&self, error: &ResilienceError) -> bool {
        if let Some(predicate) = self.retry_predicate {
            predicate(error)
        } else {
            error.is_retryable()
        }
    }

    /// Calculate delay for a specific attempt
    fn calculate_delay(&self, attempt: usize) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }

        let mut delay = if self.exponential {
            self.base_delay.saturating_mul(2_u32.saturating_pow(attempt as u32 - 1))
        } else {
            self.base_delay
        };

        // Apply jitter
        if self.jitter_factor > 0.0 {
            use std::collections::hash_map::RandomState;
            use std::hash::{BuildHasher, Hash, Hasher};

            let mut hasher = RandomState::new().build_hasher();
            attempt.hash(&mut hasher);
            let hash = hasher.finish();

            let jitter_range = (delay.as_millis() as f64 * self.jitter_factor) as u64;
            let jitter = hash % (jitter_range + 1);
            delay = delay.saturating_add(Duration::from_millis(jitter));
        }

        // Cap at maximum delay
        delay.min(self.max_delay)
    }

    /// Create a fixed delay strategy
    pub fn fixed(max_attempts: usize, delay: Duration) -> Self {
        Self {
            max_attempts,
            base_delay: delay,
            max_delay: delay,
            jitter_factor: 0.0,
            exponential: false,
            retry_predicate: None,
        }
    }

    /// Create a linear backoff strategy
    pub fn linear(max_attempts: usize, base_delay: Duration, increment: Duration) -> Self {
        Self {
            max_attempts,
            base_delay,
            max_delay: base_delay.saturating_add(increment.saturating_mul(max_attempts as u32)),
            jitter_factor: 0.1,
            exponential: false,
            retry_predicate: None,
        }
    }
}

/// Execute an operation with retry logic (for Fn closures)
pub async fn retry_with_operation<T, F, Fut>(
    strategy: RetryStrategy,
    operation: F,
) -> ResilienceResult<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = ResilienceResult<T>>,
{
    let mut last_error = None;

    for attempt in 0..=strategy.max_attempts {
        match operation().await {
            Ok(value) => {
                if attempt > 0 {
                    debug!("Operation succeeded after {} retry attempts", attempt);
                }
                return Ok(value);
            }
            Err(error) => {
                if !strategy.should_retry(&error) {
                    warn!("Error is not retryable: {:?}", error);
                    return Err(error);
                }

                if attempt < strategy.max_attempts {
                    let delay = error.retry_after()
                        .unwrap_or_else(|| strategy.calculate_delay(attempt + 1));

                    warn!(
                        "Operation failed (attempt {}/{}), retrying in {:?}: {:?}",
                        attempt + 1,
                        strategy.max_attempts,
                        delay,
                        error
                    );

                    sleep(delay).await;
                    last_error = Some(error);
                } else {
                    last_error = Some(error);
                }
            }
        }
    }

    Err(ResilienceError::retry_limit_exceeded_with_cause(
        strategy.max_attempts,
        last_error.unwrap_or_else(|| ResilienceError::Custom {
            message: "No error recorded".to_string(),
            retryable: false,
        }),
    ))
}

/// Execute an operation with retry logic (for FnMut closures)
pub async fn retry<T, F, Fut>(
    strategy: RetryStrategy,
    mut operation: F,
) -> ResilienceResult<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ResilienceResult<T>>,
{
    let mut last_error = None;

    for attempt in 0..=strategy.max_attempts {
        match operation().await {
            Ok(value) => {
                if attempt > 0 {
                    debug!("Operation succeeded after {} retry attempts", attempt);
                }
                return Ok(value);
            }
            Err(error) => {
                if !strategy.should_retry(&error) {
                    warn!("Error is not retryable: {:?}", error);
                    return Err(error);
                }

                if attempt < strategy.max_attempts {
                    let delay = error.retry_after()
                        .unwrap_or_else(|| strategy.calculate_delay(attempt + 1));

                    warn!(
                        "Operation failed (attempt {}/{}), retrying in {:?}: {:?}",
                        attempt + 1,
                        strategy.max_attempts,
                        delay,
                        error
                    );

                    sleep(delay).await;
                    last_error = Some(error);
                } else {
                    last_error = Some(error);
                }
            }
        }
    }

    Err(ResilienceError::retry_limit_exceeded_with_cause(
        strategy.max_attempts,
        last_error.unwrap_or_else(|| ResilienceError::Custom {
            message: "No error recorded".to_string(),
            retryable: false,
        }),
    ))
}

/// Retry builder for fluent API
pub struct RetryBuilder {
    strategy: RetryStrategy,
}

impl RetryBuilder {
    pub fn new() -> Self {
        Self {
            strategy: RetryStrategy::default(),
        }
    }

    pub fn max_attempts(mut self, attempts: usize) -> Self {
        self.strategy.max_attempts = attempts;
        self
    }

    pub fn base_delay(mut self, delay: Duration) -> Self {
        self.strategy.base_delay = delay;
        self
    }

    pub fn max_delay(mut self, delay: Duration) -> Self {
        self.strategy.max_delay = delay;
        self
    }

    pub fn exponential(mut self) -> Self {
        self.strategy.exponential = true;
        self
    }

    pub fn linear(mut self) -> Self {
        self.strategy.exponential = false;
        self
    }

    pub fn jitter(mut self, factor: f64) -> Self {
        self.strategy.jitter_factor = factor.clamp(0.0, 1.0);
        self
    }

    pub fn when(mut self, predicate: fn(&ResilienceError) -> bool) -> Self {
        self.strategy.retry_predicate = Some(predicate);
        self
    }

    pub async fn execute<T, F, Fut>(self, operation: F) -> ResilienceResult<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        retry(self.strategy, operation).await
    }

    pub async fn execute_fn<T, F, Fut>(self, operation: F) -> ResilienceResult<T>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        retry_with_operation(self.strategy, operation).await
    }
}

impl Default for RetryBuilder {
    fn default() -> Self {
        Self::new()
    }
}