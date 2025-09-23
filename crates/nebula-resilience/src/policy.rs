//! Unified resilience policy combining multiple resilience patterns

use std::future::Future;
use std::time::Duration;
use std::sync::Arc;
use std::pin::Pin;

use crate::{
    patterns::bulkhead::{Bulkhead, BulkheadConfig},
    patterns::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig},
    ResilienceError, ResilienceResult,
    patterns::retry::{RetryStrategy, retry_with_operation},
    patterns::timeout::timeout,
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
    pub bulkhead: Option<BulkheadConfig>,
    /// Policy name for debugging
    pub name: String,
}

impl Default for ResiliencePolicy {
    fn default() -> Self {
        Self {
            timeout: Some(Duration::from_secs(30)),
            retry: Some(RetryStrategy::default()),
            circuit_breaker: Some(CircuitBreakerConfig::default()),
            bulkhead: Some(BulkheadConfig::default()),
            name: "default".to_string(),
        }
    }
}

impl ResiliencePolicy {
    /// Create a new resilience policy with name
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Set timeout
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set retry strategy
    #[must_use]
    pub fn with_retry(mut self, retry: RetryStrategy) -> Self {
        self.retry = Some(retry);
        self
    }

    /// Set circuit breaker configuration
    #[must_use]
    pub fn with_circuit_breaker(mut self, config: CircuitBreakerConfig) -> Self {
        self.circuit_breaker = Some(config);
        self
    }

    /// Set bulkhead configuration
    #[must_use]
    pub fn with_bulkhead_config(mut self, config: BulkheadConfig) -> Self {
        self.bulkhead = Some(config);
        self
    }

    /// Execute an operation with the configured resilience policy
    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = ResilienceResult<T>> + Send + 'static,
        T: Send + 'static,
    {
        // Create resilience components
        let components = PolicyComponents::from_policy(self);

        // Build execution chain
        let chain = ExecutionChain::new(components, self.retry.clone());

        // Apply timeout as outermost wrapper
        if let Some(timeout_duration) = self.timeout {
            timeout(timeout_duration, chain.execute(operation)).await?
        } else {
            chain.execute(operation).await
        }
    }

    /// Create a reusable executor for this policy
    pub fn executor(self: Arc<Self>) -> PolicyExecutor {
        PolicyExecutor::new(self)
    }
}

/// Internal components created from policy
struct PolicyComponents {
    circuit_breaker: Option<Arc<CircuitBreaker>>,
    bulkhead: Option<Arc<Bulkhead>>,
}

impl PolicyComponents {
    fn from_policy(policy: &ResiliencePolicy) -> Self {
        Self {
            circuit_breaker: policy.circuit_breaker.as_ref()
                .map(|config| Arc::new(CircuitBreaker::with_config(config.clone()))),
            bulkhead: policy.bulkhead.as_ref()
                .map(|config| Arc::new(Bulkhead::with_config(config.clone()))),
        }
    }
}

/// Execution chain that properly orders resilience patterns
struct ExecutionChain {
    components: PolicyComponents,
    retry_strategy: Option<RetryStrategy>,
}

impl ExecutionChain {
    fn new(components: PolicyComponents, retry_strategy: Option<RetryStrategy>) -> Self {
        Self { components, retry_strategy }
    }

    async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = ResilienceResult<T>> + Send + 'static,
        T: Send + 'static,
    {
        // Order: Bulkhead -> Retry -> Circuit Breaker -> Operation

        if let Some(ref bulkhead) = self.components.bulkhead {
            let components = self.components.clone();
            let retry_strategy = self.retry_strategy.clone();

            bulkhead.execute(move || {
                Self::execute_with_retry_and_cb(operation, retry_strategy, components.circuit_breaker)
            }).await
        } else {
            Self::execute_with_retry_and_cb(
                operation,
                self.retry_strategy.clone(),
                self.components.circuit_breaker.clone()
            ).await
        }
    }

    async fn execute_with_retry_and_cb<T, F, Fut>(
        operation: F,
        retry_strategy: Option<RetryStrategy>,
        circuit_breaker: Option<Arc<CircuitBreaker>>,
    ) -> ResilienceResult<T>
    where
        F: Fn() -> Fut + Clone + Send + Sync,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        let cb = circuit_breaker.clone();
        let with_cb = move || {
            let cb = cb.clone();
            let op = operation.clone();
            async move {
                if let Some(ref breaker) = cb {
                    breaker.execute(op).await
                } else {
                    op().await
                }
            }
        };

        if let Some(strategy) = retry_strategy {
            retry_with_operation(strategy, with_cb).await
        } else {
            with_cb().await
        }
    }
}

/// Reusable executor for a policy
pub struct PolicyExecutor {
    policy: Arc<ResiliencePolicy>,
    components: PolicyComponents,
}

impl PolicyExecutor {
    fn new(policy: Arc<ResiliencePolicy>) -> Self {
        let components = PolicyComponents::from_policy(&policy);
        Self { policy, components }
    }

    pub async fn execute<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: Fn() -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = ResilienceResult<T>> + Send + 'static,
        T: Send + 'static,
    {
        let chain = ExecutionChain::new(self.components.clone(), self.policy.retry.clone());

        if let Some(timeout_duration) = self.policy.timeout {
            timeout(timeout_duration, chain.execute(operation)).await?
        } else {
            chain.execute(operation).await
        }
    }
}

impl Clone for PolicyComponents {
    fn clone(&self) -> Self {
        Self {
            circuit_breaker: self.circuit_breaker.clone(),
            bulkhead: self.bulkhead.clone(),
        }
    }
}

/// Builder for creating resilience policies
pub struct ResiliencePolicyBuilder {
    policy: ResiliencePolicy,
}

impl ResiliencePolicyBuilder {
    /// Create a new builder with a name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            policy: ResiliencePolicy::named(name),
        }
    }

    /// Set timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.policy.timeout = Some(timeout);
        self
    }

    /// Remove timeout
    pub fn no_timeout(mut self) -> Self {
        self.policy.timeout = None;
        self
    }

    /// Set retry configuration
    pub fn retry(mut self, max_attempts: usize, base_delay: Duration) -> Self {
        self.policy.retry = Some(RetryStrategy::new(max_attempts, base_delay));
        self
    }

    /// Set a custom retry strategy
    pub fn retry_strategy(mut self, strategy: RetryStrategy) -> Self {
        self.policy.retry = Some(strategy);
        self
    }

    /// Remove retry
    pub fn no_retry(mut self) -> Self {
        self.policy.retry = None;
        self
    }

    /// Set circuit breaker configuration
    pub fn circuit_breaker(mut self, failure_threshold: usize, reset_timeout: Duration) -> Self {
        self.policy.circuit_breaker = Some(CircuitBreakerConfig {
            failure_threshold,
            reset_timeout,
            half_open_max_operations: 3,
            count_timeouts: true,
        });
        self
    }

    /// Set custom circuit breaker configuration
    pub fn circuit_breaker_config(mut self, config: CircuitBreakerConfig) -> Self {
        self.policy.circuit_breaker = Some(config);
        self
    }

    /// Remove circuit breaker
    pub fn no_circuit_breaker(mut self) -> Self {
        self.policy.circuit_breaker = None;
        self
    }

    /// Set a bulkhead concurrency limit
    pub fn bulkhead(mut self, max_concurrency: usize) -> Self {
        self.policy.bulkhead = Some(BulkheadConfig {
            max_concurrency,
            ..Default::default()
        });
        self
    }

    /// Set custom bulkhead configuration
    pub fn bulkhead_config(mut self, config: BulkheadConfig) -> Self {
        self.policy.bulkhead = Some(config);
        self
    }

    /// Remove bulkhead
    pub fn no_bulkhead(mut self) -> Self {
        self.policy.bulkhead = None;
        self
    }

    /// Build the resilience policy
    pub fn build(self) -> ResiliencePolicy {
        self.policy
    }
}

/// Predefined resilience policies for common scenarios
pub mod policies {
    use super::*;

    /// Policy for database operations
    pub fn database() -> ResiliencePolicy {
        ResiliencePolicyBuilder::new("database")
            .timeout(Duration::from_secs(5))
            .retry(3, Duration::from_millis(100))
            .circuit_breaker(5, Duration::from_secs(60))
            .bulkhead(20)
            .build()
    }

    /// Policy for HTTP API calls
    pub fn http_api() -> ResiliencePolicy {
        ResiliencePolicyBuilder::new("http_api")
            .timeout(Duration::from_secs(10))
            .retry(3, Duration::from_secs(1))
            .circuit_breaker(3, Duration::from_secs(30))
            .bulkhead(50)
            .build()
    }

    /// Policy for file operations
    pub fn file_operations() -> ResiliencePolicy {
        ResiliencePolicyBuilder::new("file_operations")
            .timeout(Duration::from_secs(30))
            .retry(2, Duration::from_secs(1))
            .no_circuit_breaker()
            .bulkhead(10)
            .build()
    }

    /// Policy for long-running operations
    pub fn long_running() -> ResiliencePolicy {
        ResiliencePolicyBuilder::new("long_running")
            .timeout(Duration::from_secs(300))
            .retry(1, Duration::from_secs(5))
            .no_circuit_breaker()
            .bulkhead(5)
            .build()
    }

    /// Policy for critical operations (minimal resilience)
    pub fn critical() -> ResiliencePolicy {
        ResiliencePolicyBuilder::new("critical")
            .timeout(Duration::from_secs(60))
            .no_retry()
            .no_circuit_breaker()
            .no_bulkhead()
            .build()
    }

    /// Policy for real-time operations
    pub fn real_time() -> ResiliencePolicy {
        ResiliencePolicyBuilder::new("real_time")
            .timeout(Duration::from_millis(100))
            .no_retry()
            .circuit_breaker(10, Duration::from_secs(5))
            .bulkhead(100)
            .build()
    }
}