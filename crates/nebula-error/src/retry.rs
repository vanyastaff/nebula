//! Retry logic and strategies for Nebula

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::{sleep, timeout};

use super::error::NebulaError;

/// Retry strategy configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryStrategy {
    /// Maximum number of retry attempts
    pub max_attempts: u32,
    /// Base delay between retries
    pub base_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Exponential backoff multiplier
    pub backoff_multiplier: f64,
    /// Jitter factor (0.0 = no jitter, 1.0 = full jitter)
    pub jitter_factor: f64,
    /// Whether to use exponential backoff
    pub exponential_backoff: bool,
    /// Timeout for the entire retry operation
    pub timeout: Option<Duration>,
}

impl Default for RetryStrategy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            backoff_multiplier: 2.0,
            jitter_factor: 0.1,
            exponential_backoff: true,
            timeout: Some(Duration::from_secs(60)),
        }
    }
}

impl RetryStrategy {
    /// Create a new retry strategy
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum attempts
    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    /// Set base delay
    pub fn with_base_delay(mut self, base_delay: Duration) -> Self {
        self.base_delay = base_delay;
        self
    }

    /// Set maximum delay
    pub fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    /// Set backoff multiplier
    pub fn with_backoff_multiplier(mut self, multiplier: f64) -> Self {
        self.backoff_multiplier = multiplier;
        self
    }

    /// Set jitter factor
    pub fn with_jitter_factor(mut self, jitter: f64) -> Self {
        self.jitter_factor = jitter;
        self
    }

    /// Enable/disable exponential backoff
    pub fn with_exponential_backoff(mut self, enabled: bool) -> Self {
        self.exponential_backoff = enabled;
        self
    }

    /// Set timeout
    pub fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Calculate delay for a specific attempt
    pub fn calculate_delay(&self, attempt: u32) -> Duration {
        if !self.exponential_backoff || attempt == 0 {
            return self.base_delay;
        }

        let mut delay = self.base_delay.as_millis() as f64;

        // Apply exponential backoff
        for _ in 0..attempt {
            delay *= self.backoff_multiplier;
        }

        // Apply jitter
        if self.jitter_factor > 0.0 {
            let jitter = delay * self.jitter_factor * (rand::random::<f64>() - 0.5);
            delay += jitter;
        }

        // Ensure delay is within bounds
        delay = delay.max(self.base_delay.as_millis() as f64);
        delay = delay.min(self.max_delay.as_millis() as f64);

        Duration::from_millis(delay as u64)
    }

    /// Create a fixed delay strategy (no exponential backoff)
    pub fn fixed_delay(delay: Duration, max_attempts: u32) -> Self {
        Self {
            max_attempts,
            base_delay: delay,
            max_delay: delay,
            backoff_multiplier: 1.0,
            jitter_factor: 0.0,
            exponential_backoff: false,
            timeout: None,
        }
    }

    /// Create an aggressive retry strategy for transient failures
    pub fn aggressive() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(5),
            backoff_multiplier: 1.5,
            jitter_factor: 0.2,
            exponential_backoff: true,
            timeout: Some(Duration::from_secs(30)),
        }
    }

    /// Create a conservative retry strategy for critical operations
    pub fn conservative() -> Self {
        Self {
            max_attempts: 2,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            backoff_multiplier: 3.0,
            jitter_factor: 0.05,
            exponential_backoff: true,
            timeout: Some(Duration::from_secs(300)),
        }
    }
}

/// Trait for operations that can be retried
#[async_trait]
pub trait Retryable {
    /// The output type of the operation
    type Output: Send;
    /// The error type of the operation
    type Error: Into<NebulaError> + Send;

    /// Execute the operation
    async fn execute(&self) -> Result<Self::Output, Self::Error>;

    /// Check if an error is retryable
    fn is_retryable_error(&self, error: &Self::Error) -> bool;

    /// Execute with retry logic
    async fn execute_with_retry(
        &self,
        strategy: &RetryStrategy,
    ) -> Result<Self::Output, NebulaError> {
        let start_time = std::time::Instant::now();
        let mut last_error = None;

        for attempt in 0..strategy.max_attempts {
            // Check timeout
            if let Some(timeout) = strategy.timeout
                && start_time.elapsed() >= timeout
            {
                return Err(NebulaError::timeout("retry operation", timeout));
            }

            // Execute the operation
            match self.execute().await {
                Ok(result) => return Ok(result),
                Err(error) => {
                    last_error = Some(error);

                    // Check if we should retry
                    if !self.is_retryable_error(last_error.as_ref().unwrap()) {
                        break;
                    }

                    // If this is the last attempt, don't sleep
                    if attempt + 1 >= strategy.max_attempts {
                        break;
                    }

                    // Calculate and apply delay
                    let delay = strategy.calculate_delay(attempt);
                    sleep(delay).await;
                }
            }
        }

        // Convert the last error to NebulaError
        Err(last_error.unwrap().into())
    }
}

/// Retry a function with the given strategy
pub async fn retry<F, Fut, T, E>(f: F, strategy: &RetryStrategy) -> Result<T, NebulaError>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<T, E>> + Send,
    T: Send,
    E: Into<NebulaError> + Send,
{
    let start_time = std::time::Instant::now();
    let mut last_error = None;

    for attempt in 0..strategy.max_attempts {
        // Check timeout
        if let Some(timeout) = strategy.timeout
            && start_time.elapsed() >= timeout
        {
            return Err(NebulaError::timeout("retry operation", timeout));
        }

        // Execute the function
        match f().await {
            Ok(result) => return Ok(result),
            Err(error) => {
                last_error = Some(error);

                // If this is the last attempt, don't sleep
                if attempt + 1 >= strategy.max_attempts {
                    break;
                }

                // Calculate and apply delay
                let delay = strategy.calculate_delay(attempt);
                sleep(delay).await;
            }
        }
    }

    // Convert the last error to NebulaError
    Err(last_error.unwrap().into())
}

/// Retry a function with timeout
pub async fn retry_with_timeout<F, Fut, T, E>(
    f: F,
    strategy: &RetryStrategy,
    operation_timeout: Duration,
) -> Result<T, NebulaError>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<T, E>> + Send,
    T: Send,
    E: Into<NebulaError> + Send,
{
    let retry_future = retry(f, strategy);

    match timeout(operation_timeout, retry_future).await {
        Ok(result) => result,
        Err(_) => Err(NebulaError::timeout("retry operation", operation_timeout)),
    }
}

/// Retry configuration for specific error types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorRetryConfig {
    /// Error code to match
    pub error_code: String,
    /// Whether this error is retryable
    pub retryable: bool,
    /// Custom retry strategy for this error
    pub strategy: Option<RetryStrategy>,
    /// Maximum retry attempts for this error
    pub max_attempts: Option<u32>,
}

impl ErrorRetryConfig {
    /// Create a new error retry configuration
    pub fn new(error_code: impl Into<String>, retryable: bool) -> Self {
        Self {
            error_code: error_code.into(),
            retryable,
            strategy: None,
            max_attempts: None,
        }
    }

    /// Set custom retry strategy
    pub fn with_strategy(mut self, strategy: RetryStrategy) -> Self {
        self.strategy = Some(strategy);
        self
    }

    /// Set maximum attempts
    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = Some(max_attempts);
        self
    }
}

/// Retry policy that defines retry behavior for different error types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Default retry strategy
    pub default_strategy: RetryStrategy,
    /// Error-specific retry configurations
    pub error_configs: Vec<ErrorRetryConfig>,
    /// Global timeout for all retry operations
    pub global_timeout: Option<Duration>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            default_strategy: RetryStrategy::default(),
            error_configs: Vec::new(),
            global_timeout: Some(Duration::from_secs(300)),
        }
    }
}

impl RetryPolicy {
    /// Create a new retry policy
    pub fn new() -> Self {
        Self::default()
    }

    /// Add error-specific retry configuration
    pub fn with_error_config(mut self, config: ErrorRetryConfig) -> Self {
        self.error_configs.push(config);
        self
    }

    /// Set global timeout
    pub fn with_global_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.global_timeout = timeout;
        self
    }

    /// Get retry strategy for a specific error
    pub fn get_strategy_for_error(&self, error: &NebulaError) -> RetryStrategy {
        // Look for error-specific configuration
        for config in &self.error_configs {
            if config.error_code == error.error_code() {
                if let Some(ref strategy) = config.strategy {
                    return strategy.clone();
                }
                if let Some(max_attempts) = config.max_attempts {
                    let mut strategy = self.default_strategy.clone();
                    strategy.max_attempts = max_attempts;
                    return strategy;
                }
            }
        }

        // Return default strategy
        self.default_strategy.clone()
    }

    /// Check if an error is retryable according to this policy
    pub fn is_retryable(&self, error: &NebulaError) -> bool {
        // Check error-specific configuration first
        for config in &self.error_configs {
            if config.error_code == error.error_code() {
                return config.retryable;
            }
        }

        // Fall back to error's own retryable flag
        error.is_retryable()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_retry_strategy_default() {
        let strategy = RetryStrategy::default();
        assert_eq!(strategy.max_attempts, 3);
        assert_eq!(strategy.base_delay, Duration::from_millis(100));
        assert!(strategy.exponential_backoff);
    }

    #[tokio::test]
    async fn test_retry_strategy_custom() {
        let strategy = RetryStrategy::new()
            .with_max_attempts(5)
            .with_base_delay(Duration::from_secs(1))
            .with_exponential_backoff(false);

        assert_eq!(strategy.max_attempts, 5);
        assert_eq!(strategy.base_delay, Duration::from_secs(1));
        assert!(!strategy.exponential_backoff);
    }

    #[tokio::test]
    async fn test_retry_strategy_delay_calculation() {
        let strategy = RetryStrategy::new()
            .with_base_delay(Duration::from_millis(100))
            .with_backoff_multiplier(2.0);

        let delay1 = strategy.calculate_delay(1);
        let delay2 = strategy.calculate_delay(2);
        let delay3 = strategy.calculate_delay(3);

        // With exponential backoff, delays should increase
        assert!(delay2 > delay1);
        assert!(delay3 > delay2);
    }

    #[tokio::test]
    async fn test_retry_strategy_presets() {
        let aggressive = RetryStrategy::aggressive();
        assert_eq!(aggressive.max_attempts, 5);
        assert_eq!(aggressive.base_delay, Duration::from_millis(50));

        let conservative = RetryStrategy::conservative();
        assert_eq!(conservative.max_attempts, 2);
        assert_eq!(conservative.base_delay, Duration::from_secs(1));
    }

    #[tokio::test]
    async fn test_retry_policy() {
        let policy = RetryPolicy::new()
            .with_error_config(ErrorRetryConfig::new("TIMEOUT_ERROR", true))
            .with_error_config(ErrorRetryConfig::new("VALIDATION_ERROR", false));

        assert!(policy.is_retryable(&NebulaError::timeout("test", Duration::from_secs(1))));
        assert!(!policy.is_retryable(&NebulaError::validation("test")));
    }

    #[tokio::test]
    async fn test_retry_function() {
        let strategy = RetryStrategy::fixed_delay(Duration::from_millis(10), 3);
        let attempts = std::sync::Arc::new(std::sync::Mutex::new(0));

        let result = retry(
            {
                let attempts = attempts.clone();
                move || {
                    let attempts = attempts.clone();
                    async move {
                        let mut count = attempts.lock().unwrap();
                        *count += 1;
                        if *count < 3 {
                            Err("temporary error")
                        } else {
                            Ok("success")
                        }
                    }
                }
            },
            &strategy,
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
        assert_eq!(*attempts.lock().unwrap(), 3);
    }
}
