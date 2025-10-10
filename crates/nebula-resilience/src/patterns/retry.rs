//! Modern retry strategies for resilient operations

use serde::{Deserialize, Serialize};
use std::future::Future;
use std::time::Duration;

use crate::ResilienceError;
use crate::core::config::{ConfigError, ConfigResult, ResilienceConfig};

/// Modern retry strategy with flexible backoff policies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryStrategy {
    /// Maximum number of retry attempts
    pub max_attempts: usize,
    /// Backoff policy
    pub backoff: BackoffPolicy,
    /// Custom retry condition predicates
    pub retry_condition: RetryCondition,
}

/// Backoff policies for retry delays
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackoffPolicy {
    /// Fixed delay between retries
    Fixed {
        /// Fixed delay duration
        delay: Duration,
    },
    /// Linear backoff: delay = `base_delay` * attempt
    Linear {
        /// Base delay for calculations
        base_delay: Duration,
        /// Maximum delay cap
        max_delay: Duration,
    },
    /// Exponential backoff: delay = `base_delay` * multiplier^attempt
    Exponential {
        /// Base delay for calculations
        base_delay: Duration,
        /// Exponential multiplier
        multiplier: f64,
        /// Maximum delay cap
        max_delay: Duration,
        /// Jitter policy to apply
        jitter: JitterPolicy,
    },
    /// Custom backoff with specified delays per attempt
    Custom {
        /// List of delays for each attempt
        delays: Vec<Duration>,
    },
}

/// Jitter policies to avoid thundering herd
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JitterPolicy {
    /// No jitter
    None,
    /// Full jitter: random(0, `calculated_delay`)
    Full,
    /// Equal jitter: `calculated_delay/2` + random(0, `calculated_delay/2`)
    Equal,
    /// Decorrelated jitter: `random(base_delay`, `previous_delay` * 3)
    Decorrelated,
}

/// Conditions for when to retry an operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryCondition {
    /// Retry on timeout errors
    pub on_timeout: bool,
    /// Retry on rate limit errors
    pub on_rate_limit: bool,
    /// Retry on circuit breaker open errors
    pub on_circuit_breaker_open: bool,
    /// Retry on custom retryable errors
    pub on_custom_retryable: bool,
    /// Don't retry on these specific error types
    pub exclude_errors: Vec<String>,
}

impl Default for RetryStrategy {
    fn default() -> Self {
        Self::exponential_backoff(3, Duration::from_millis(100))
    }
}

impl RetryStrategy {
    /// Create a fixed delay retry strategy
    #[must_use] 
    pub fn fixed_delay(max_attempts: usize, delay: Duration) -> Self {
        Self {
            max_attempts,
            backoff: BackoffPolicy::Fixed { delay },
            retry_condition: RetryCondition::default(),
        }
    }

    /// Create a linear backoff retry strategy
    #[must_use] 
    pub fn linear_backoff(max_attempts: usize, base_delay: Duration) -> Self {
        Self {
            max_attempts,
            backoff: BackoffPolicy::Linear {
                base_delay,
                max_delay: Duration::from_secs(30),
            },
            retry_condition: RetryCondition::default(),
        }
    }

    /// Create an exponential backoff retry strategy
    #[must_use] 
    pub fn exponential_backoff(max_attempts: usize, base_delay: Duration) -> Self {
        Self {
            max_attempts,
            backoff: BackoffPolicy::Exponential {
                base_delay,
                multiplier: 2.0,
                max_delay: Duration::from_secs(30),
                jitter: JitterPolicy::Equal,
            },
            retry_condition: RetryCondition::default(),
        }
    }

    /// Create a custom retry strategy with specific delays
    #[must_use] 
    pub fn custom_delays(delays: Vec<Duration>) -> Self {
        let max_attempts = delays.len();
        Self {
            max_attempts,
            backoff: BackoffPolicy::Custom { delays },
            retry_condition: RetryCondition::default(),
        }
    }

    /// Set custom retry condition
    #[must_use = "builder methods must be chained or built"]
    pub fn with_condition(mut self, condition: RetryCondition) -> Self {
        self.retry_condition = condition;
        self
    }

    /// Check if an error should be retried
    #[must_use] 
    pub fn should_retry(&self, error: &ResilienceError) -> bool {
        match error {
            ResilienceError::Timeout { .. } => self.retry_condition.on_timeout,
            ResilienceError::RateLimitExceeded { .. } => self.retry_condition.on_rate_limit,
            ResilienceError::CircuitBreakerOpen { .. } => {
                self.retry_condition.on_circuit_breaker_open
            }
            ResilienceError::Custom { retryable, .. } => {
                *retryable && self.retry_condition.on_custom_retryable
            }
            ResilienceError::RetryLimitExceeded { .. } => false,
            ResilienceError::Cancelled { .. } => false,
            ResilienceError::InvalidConfig { .. } => false,
            _ => false,
        }
    }

    /// Calculate delay for a specific attempt (1-indexed)
    #[must_use] 
    pub fn delay_for_attempt(&self, attempt: usize) -> Option<Duration> {
        if attempt > self.max_attempts {
            return None;
        }

        let base_delay = match &self.backoff {
            BackoffPolicy::Fixed { delay } => *delay,
            BackoffPolicy::Linear {
                base_delay,
                max_delay,
            } => {
                let calculated =
                    Duration::from_millis(base_delay.as_millis() as u64 * attempt as u64);
                std::cmp::min(calculated, *max_delay)
            }
            BackoffPolicy::Exponential {
                base_delay,
                multiplier,
                max_delay,
                jitter,
            } => {
                let calculated_ms =
                    (base_delay.as_millis() as f64 * multiplier.powi(attempt as i32 - 1)) as u64;
                let calculated = Duration::from_millis(calculated_ms);
                let capped = std::cmp::min(calculated, *max_delay);

                Self::apply_jitter(capped, jitter, *base_delay)
            }
            BackoffPolicy::Custom { delays } => {
                if attempt > 0 && attempt <= delays.len() {
                    delays[attempt - 1]
                } else {
                    return None;
                }
            }
        };

        Some(base_delay)
    }

    /// Apply jitter to delay
    fn apply_jitter(delay: Duration, jitter: &JitterPolicy, base_delay: Duration) -> Duration {
        use rand::Rng;
        let mut rng = rand::thread_rng();

        match jitter {
            JitterPolicy::None => delay,
            JitterPolicy::Full => {
                let jitter_ms = rng.gen_range(0..=delay.as_millis() as u64);
                Duration::from_millis(jitter_ms)
            }
            JitterPolicy::Equal => {
                let half = delay.as_millis() as u64 / 2;
                let jitter_ms = half + rng.gen_range(0..=half);
                Duration::from_millis(jitter_ms)
            }
            JitterPolicy::Decorrelated => {
                let min = base_delay.as_millis() as u64;
                let max = delay.as_millis() as u64 * 3;
                let jitter_ms = rng.gen_range(min..=max);
                Duration::from_millis(jitter_ms)
            }
        }
    }
}

impl Default for RetryCondition {
    fn default() -> Self {
        Self {
            on_timeout: true,
            on_rate_limit: true,
            on_circuit_breaker_open: false, // Usually don't retry on circuit breaker
            on_custom_retryable: true,
            exclude_errors: Vec::new(),
        }
    }
}

impl RetryCondition {
    /// Create a permissive retry condition (retry most errors)
    #[must_use] 
    pub fn permissive() -> Self {
        Self {
            on_timeout: true,
            on_rate_limit: true,
            on_circuit_breaker_open: true,
            on_custom_retryable: true,
            exclude_errors: Vec::new(),
        }
    }

    /// Create a conservative retry condition (retry only safe errors)
    #[must_use] 
    pub fn conservative() -> Self {
        Self {
            on_timeout: true,
            on_rate_limit: false,
            on_circuit_breaker_open: false,
            on_custom_retryable: false,
            exclude_errors: Vec::new(),
        }
    }

    /// Exclude specific error types from retry
    #[must_use = "builder methods must be chained or built"]
    pub fn exclude_error(mut self, error_type: impl Into<String>) -> Self {
        self.exclude_errors.push(error_type.into());
        self
    }
}

impl ResilienceConfig for RetryStrategy {
    fn validate(&self) -> ConfigResult<()> {
        if self.max_attempts == 0 {
            return Err(ConfigError::validation(
                "max_attempts must be greater than 0",
            ));
        }

        match &self.backoff {
            BackoffPolicy::Fixed { delay } => {
                if delay.is_zero() {
                    return Err(ConfigError::validation("Fixed delay cannot be zero"));
                }
            }
            BackoffPolicy::Linear {
                base_delay,
                max_delay,
            } => {
                if base_delay.is_zero() {
                    return Err(ConfigError::validation("Base delay cannot be zero"));
                }
                if max_delay < base_delay {
                    return Err(ConfigError::validation("Max delay must be >= base delay"));
                }
            }
            BackoffPolicy::Exponential {
                base_delay,
                multiplier,
                max_delay,
                ..
            } => {
                if base_delay.is_zero() {
                    return Err(ConfigError::validation("Base delay cannot be zero"));
                }
                if *multiplier <= 1.0 {
                    return Err(ConfigError::validation(
                        "Exponential multiplier must be > 1.0",
                    ));
                }
                if max_delay < base_delay {
                    return Err(ConfigError::validation("Max delay must be >= base delay"));
                }
            }
            BackoffPolicy::Custom { delays } => {
                if delays.is_empty() {
                    return Err(ConfigError::validation("Custom delays cannot be empty"));
                }
                if delays.len() != self.max_attempts {
                    return Err(ConfigError::validation(
                        "Custom delays length must match max_attempts",
                    ));
                }
            }
        }

        Ok(())
    }

    fn default_config() -> Self {
        Self::default()
    }

    fn merge(&mut self, other: Self) {
        *self = other;
    }
}

/// Legacy retry function for backward compatibility
pub async fn retry<T, F, Fut, E>(
    strategy: RetryStrategy,
    mut operation: F,
) -> Result<T, ResilienceError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: Into<ResilienceError>,
{
    let mut last_error = None;

    for attempt in 1..=strategy.max_attempts {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                let error = e.into();

                if attempt == strategy.max_attempts {
                    last_error = Some(error);
                    break;
                }

                if strategy.should_retry(&error) {
                    last_error = Some(error);

                    if let Some(delay) = strategy.delay_for_attempt(attempt) {
                        tokio::time::sleep(delay).await;
                    }
                } else {
                    return Err(error);
                }
            }
        }
    }

    Err(ResilienceError::RetryLimitExceeded {
        attempts: strategy.max_attempts,
        last_error: last_error.map(Box::new),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_fixed_delay_retry() {
        let strategy = RetryStrategy::fixed_delay(3, Duration::from_millis(10));
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let result = retry(strategy, || {
            let counter = counter_clone.clone();
            async move {
                let count = counter.fetch_add(1, Ordering::SeqCst);
                if count < 2 {
                    Err(ResilienceError::Custom {
                        message: "Simulated failure".to_string(),
                        retryable: true,
                        source: None,
                    })
                } else {
                    Ok::<u32, ResilienceError>(42)
                }
            }
        })
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_exponential_backoff() {
        let strategy = RetryStrategy::exponential_backoff(3, Duration::from_millis(10));

        // Test delay calculation
        let delay1 = strategy.delay_for_attempt(1);
        assert!(delay1.is_some(), "Delay for attempt 1 should not be None");
        assert!(delay1.unwrap() >= Duration::from_millis(5));

        let delay2 = strategy.delay_for_attempt(2);
        assert!(delay2.is_some(), "Delay for attempt 2 should not be None");
        assert!(delay2.unwrap() >= Duration::from_millis(10));

        let delay3 = strategy.delay_for_attempt(3);
        assert!(delay3.is_some(), "Delay for attempt 3 should not be None");
        assert!(delay3.unwrap() >= Duration::from_millis(20));
    }

    #[test]
    fn test_retry_conditions() {
        let strategy = RetryStrategy::default();

        // Should retry
        assert!(strategy.should_retry(&ResilienceError::Timeout {
            duration: Duration::from_secs(1),
            context: None,
        }));

        // Should not retry
        assert!(!strategy.should_retry(&ResilienceError::Cancelled { reason: None }));
    }
}
