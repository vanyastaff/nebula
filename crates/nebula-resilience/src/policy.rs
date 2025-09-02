//! Unified resilience policy combining multiple resilience patterns

use std::time::Duration;
use futures::Future;
use tracing::{debug, info};

use crate::{
    error::{ResilienceError, ResilienceResult},
    timeout::timeout,
    retry::{RetryStrategy, Retryable},
    circuit_breaker::{CircuitBreaker, CircuitBreakerConfig},
    bulkhead::Bulkhead,
};

/// Resilience policy configuration
#[derive(Debug, Clone)]
pub struct ResiliencePolicy {
    /// Timeout for operations
    pub timeout: Option<Duration>,
    /// Retry strategy
    pub retry: Option<RetryStrategy>,
    /// Circuit breaker configuration
    pub circuit_breaker: Option<CircuitBreakerConfig>,
    /// Bulkhead configuration
    pub bulkhead: Option<usize>,
}

impl Default for ResiliencePolicy {
    fn default() -> Self {
        Self {
            timeout: Some(Duration::from_secs(30)),
            retry: Some(RetryStrategy::default()),
            circuit_breaker: Some(CircuitBreakerConfig::default()),
            bulkhead: Some(10),
        }
    }
}

impl ResiliencePolicy {
    /// Create a new resilience policy
    pub fn new() -> Self {
        Self::default()
    }

    /// Set timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set retry strategy
    pub fn with_retry(mut self, retry: RetryStrategy) -> Self {
        self.retry = Some(retry);
        self
    }

    /// Set circuit breaker configuration
    pub fn with_circuit_breaker(mut self, config: CircuitBreakerConfig) -> Self {
        self.circuit_breaker = Some(config);
        self
    }

    /// Set bulkhead concurrency limit
    pub fn with_bulkhead(mut self, max_concurrency: usize) -> Self {
        self.bulkhead = Some(max_concurrency);
        self
    }

    /// Remove timeout
    pub fn without_timeout(mut self) -> Self {
        self.timeout = None;
        self
    }

    /// Remove retry
    pub fn without_retry(mut self) -> Self {
        self.retry = None;
        self
    }

    /// Remove circuit breaker
    pub fn without_circuit_breaker(mut self) -> Self {
        self.circuit_breaker = None;
        self
    }

    /// Remove bulkhead
    pub fn without_bulkhead(mut self) -> Self {
        self.bulkhead = None;
        self
    }

    /// Execute an operation with the configured resilience policy
    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
        T: Clone,
    {
        // Apply timeout if configured
        if let Some(timeout_duration) = self.timeout {
            timeout(timeout_duration, self.execute_inner(operation)).await?
        } else {
            self.execute_inner(operation).await
        }
    }

    /// Internal execution logic without timeout
    async fn execute_inner<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
        T: Clone,
    {
        let result = operation().await;

        // Apply retry if configured
        if let Some(retry_strategy) = &self.retry {
            let strategy = retry_strategy.clone();
            // Convert ResilienceResult<T> to Result<T, ResilienceError> for retry
            return crate::retry::retry(strategy.clone(), || async { 
                match &result {
                    Ok(value) => Ok(value.clone()),
                    Err(e) => Err(e.clone()),
                }
            }).await;
        }

        // Apply circuit breaker if configured
        if let Some(config) = &self.circuit_breaker {
            let circuit_breaker = CircuitBreaker::with_config(config.clone());
            return circuit_breaker.execute(|| async { result.clone() }).await;
        }

        // Apply bulkhead if configured
        if let Some(max_concurrency) = self.bulkhead {
            let bulkhead = Bulkhead::new(max_concurrency);
            return bulkhead.execute(|| async { result.clone() }).await;
        }

        result
    }

    /// Execute an operation with timeout only
    pub async fn execute_with_timeout<T, F, Fut>(
        &self,
        operation: F,
    ) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        if let Some(timeout_duration) = self.timeout {
            timeout(timeout_duration, operation()).await?
        } else {
            operation().await
        }
    }

    /// Execute an operation with retry only
    pub async fn execute_with_retry<T, F, Fut>(
        &self,
        mut operation: F,
    ) -> ResilienceResult<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        if let Some(retry_strategy) = &self.retry {
            crate::retry::retry(retry_strategy.clone(), operation).await
        } else {
            operation().await
        }
    }

    /// Execute an operation with circuit breaker only
    pub async fn execute_with_circuit_breaker<T, F, Fut>(
        &self,
        operation: F,
    ) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        if let Some(config) = &self.circuit_breaker {
            let circuit_breaker = CircuitBreaker::with_config(config.clone());
            circuit_breaker.execute(operation).await
        } else {
            operation().await
        }
    }

    /// Execute an operation with bulkhead only
    pub async fn execute_with_bulkhead<T, F, Fut>(
        &self,
        operation: F,
    ) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
    {
        if let Some(max_concurrency) = self.bulkhead {
            let bulkhead = Bulkhead::new(max_concurrency);
            bulkhead.execute(operation).await
        } else {
            operation().await
        }
    }
}

/// Builder for creating resilience policies
pub struct ResilienceBuilder {
    policy: ResiliencePolicy,
}

impl ResilienceBuilder {
    /// Create a new resilience builder
    pub fn new() -> Self {
        Self {
            policy: ResiliencePolicy::new(),
        }
    }

    /// Set timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.policy = self.policy.with_timeout(timeout);
        self
    }

    /// Set retry configuration
    pub fn retry(mut self, max_attempts: usize, base_delay: Duration) -> Self {
        let retry_strategy = RetryStrategy::new(max_attempts, base_delay);
        self.policy = self.policy.with_retry(retry_strategy);
        self
    }

    /// Set circuit breaker configuration
    pub fn circuit_breaker(
        mut self,
        failure_threshold: usize,
        reset_timeout: Duration,
    ) -> Self {
        let config = CircuitBreakerConfig {
            failure_threshold,
            reset_timeout,
            half_open_max_operations: 3,
            count_timeouts: true,
        };
        self.policy = self.policy.with_circuit_breaker(config);
        self
    }

    /// Set bulkhead concurrency limit
    pub fn bulkhead(mut self, max_concurrency: usize) -> Self {
        self.policy = self.policy.with_bulkhead(max_concurrency);
        self
    }

    /// Build the resilience policy
    pub fn build(self) -> ResiliencePolicy {
        self.policy
    }
}

impl Default for ResilienceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Predefined resilience policies for common scenarios
pub mod policies {
    use super::*;

    /// Policy for database operations
    pub fn database() -> ResiliencePolicy {
        ResilienceBuilder::new()
            .timeout(Duration::from_secs(5))
            .retry(3, Duration::from_millis(100))
            .circuit_breaker(5, Duration::from_secs(60))
            .bulkhead(20)
            .build()
    }

    /// Policy for HTTP API calls
    pub fn http_api() -> ResiliencePolicy {
        ResilienceBuilder::new()
            .timeout(Duration::from_secs(10))
            .retry(3, Duration::from_secs(1))
            .circuit_breaker(3, Duration::from_secs(30))
            .bulkhead(50)
            .build()
    }

    /// Policy for file operations
    pub fn file_operations() -> ResiliencePolicy {
        ResilienceBuilder::new()
            .timeout(Duration::from_secs(30))
            .retry(2, Duration::from_secs(1))
            .bulkhead(10)
            .build()
    }

    /// Policy for long-running operations
    pub fn long_running() -> ResiliencePolicy {
        ResilienceBuilder::new()
            .timeout(Duration::from_secs(300))
            .retry(1, Duration::from_secs(5))
            .bulkhead(5)
            .build()
    }

    /// Policy for critical operations (minimal resilience)
    pub fn critical() -> ResiliencePolicy {
        ResilienceBuilder::new()
            .timeout(Duration::from_secs(60))
            .retry(1, Duration::from_secs(1))
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_resilience_policy_default() {
        let policy = ResiliencePolicy::default();
        assert!(policy.timeout.is_some());
        assert!(policy.retry.is_some());
        assert!(policy.circuit_breaker.is_some());
        assert!(policy.bulkhead.is_some());
    }

    #[tokio::test]
    async fn test_resilience_policy_builder() {
        let policy = ResilienceBuilder::new()
            .timeout(Duration::from_secs(10))
            .retry(5, Duration::from_secs(2))
            .circuit_breaker(3, Duration::from_secs(30))
            .bulkhead(15)
            .build();

        assert_eq!(policy.timeout, Some(Duration::from_secs(10)));
        assert!(policy.retry.is_some());
        assert!(policy.circuit_breaker.is_some());
        assert_eq!(policy.bulkhead, Some(15));
    }

    #[tokio::test]
    async fn test_resilience_policy_execute() {
        let policy = ResiliencePolicy::new()
            .with_timeout(Duration::from_millis(100))
            .with_retry(RetryStrategy::new(2, Duration::from_millis(10)));

        let result = policy
            .execute(|| async { Ok::<&str, ResilienceError>("success") })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "success");
    }

    #[tokio::test]
    async fn test_resilience_policy_timeout() {
        let policy = ResiliencePolicy::new()
            .with_timeout(Duration::from_millis(10));

        let result = policy
            .execute(|| async {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok::<&str, ResilienceError>("should timeout")
            })
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            ResilienceError::Timeout { .. } => {}
            _ => panic!("Expected timeout error"),
        }
    }

    #[tokio::test]
    async fn test_predefined_policies() {
        let db_policy = policies::database();
        assert_eq!(db_policy.timeout, Some(Duration::from_secs(5)));

        let http_policy = policies::http_api();
        assert_eq!(http_policy.timeout, Some(Duration::from_secs(10)));

        let file_policy = policies::file_operations();
        assert_eq!(file_policy.timeout, Some(Duration::from_secs(30)));

        let long_policy = policies::long_running();
        assert_eq!(long_policy.timeout, Some(Duration::from_secs(300)));

        let critical_policy = policies::critical();
        assert_eq!(critical_policy.timeout, Some(Duration::from_secs(60)));
    }
}
