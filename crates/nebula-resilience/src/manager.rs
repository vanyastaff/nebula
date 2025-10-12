//! Resilience manager for centralized pattern orchestration

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use crate::{
    ResilienceError, ResilienceResult,
    patterns::{
        bulkhead::{Bulkhead, BulkheadConfig, BulkheadStats},
        circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitBreakerStats},
        retry::RetryStrategy,
        timeout::timeout,
    },
    policy::ResiliencePolicy,
};

/// Service operations that can be retried
///
/// This trait allows operations to be called multiple times for retry scenarios
/// while maintaining proper async semantics and error handling.
#[async_trait::async_trait]
pub trait RetryableOperation<T> {
    /// Execute the operation
    async fn execute(&self) -> ResilienceResult<T>;
}

/// Implement `RetryableOperation` for async closures that can be called multiple times
#[async_trait::async_trait]
impl<F, Fut, T> RetryableOperation<T> for F
where
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = ResilienceResult<T>> + Send,
{
    async fn execute(&self) -> ResilienceResult<T> {
        self().await
    }
}

/// Builder for resilience policies
#[derive(Debug, Clone, Default)]
pub struct PolicyBuilder {
    timeout: Option<Duration>,
    retry: Option<RetryStrategy>,
    circuit_breaker: Option<CircuitBreakerConfig>,
    bulkhead: Option<BulkheadConfig>,
}

impl PolicyBuilder {
    /// Create a new policy builder
    #[must_use] 
    pub fn new() -> Self {
        Self::default()
    }

    /// Set timeout
    #[must_use = "builder methods must be chained or built"]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set retry strategy
    #[must_use = "builder methods must be chained or built"]
    pub fn with_retry(mut self, strategy: RetryStrategy) -> Self {
        self.retry = Some(strategy);
        self
    }

    /// Set circuit breaker configuration
    #[must_use = "builder methods must be chained or built"]
    pub fn with_circuit_breaker(mut self, config: CircuitBreakerConfig) -> Self {
        self.circuit_breaker = Some(config);
        self
    }

    /// Set bulkhead configuration
    #[must_use = "builder methods must be chained or built"]
    pub fn with_bulkhead(mut self, config: BulkheadConfig) -> Self {
        self.bulkhead = Some(config);
        self
    }

    /// Build the policy
    #[must_use] 
    pub fn build(self) -> ResiliencePolicy {
        ResiliencePolicy {
            timeout: self.timeout,
            retry: self.retry,
            circuit_breaker: self.circuit_breaker,
            bulkhead: self.bulkhead,
            metadata: crate::policy::PolicyMetadata::default(),
        }
    }
}

/// Execution context for resilience operations
///
/// Tracks execution metadata for observability and metrics collection.
/// Fields are preserved for future logging/metrics integration.
#[derive(Debug)]
pub(crate) struct ExecutionContext {
    /// Name of the service being executed
    pub service_name: String,
    /// Name of the operation being performed (for metrics/logging)
    #[allow(dead_code)]
    pub operation_name: String,
    /// Current attempt number (starts at 1)
    pub attempt: usize,
    /// Timestamp when execution started (for latency metrics)
    #[allow(dead_code)]
    pub start_time: std::time::Instant,
}

impl ExecutionContext {
    /// Create new execution context
    pub(crate) fn new(service: impl Into<String>, operation: impl Into<String>) -> Self {
        Self {
            service_name: service.into(),
            operation_name: operation.into(),
            attempt: 1,
            start_time: std::time::Instant::now(),
        }
    }

    /// Increment attempt counter (for retry tracking)
    #[allow(dead_code)]
    pub(crate) fn next_attempt(&mut self) {
        self.attempt += 1;
    }

    /// Get elapsed time since start (for latency metrics)
    #[allow(dead_code)]
    pub(crate) fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }
}

/// Modern resilience manager with proper async semantics
#[derive(Debug)]
pub struct ResilienceManager {
    /// Service policies
    policies: Arc<RwLock<HashMap<String, Arc<ResiliencePolicy>>>>,
    /// Circuit breakers per service
    circuit_breakers: Arc<RwLock<HashMap<String, Arc<CircuitBreaker>>>>,
    /// Bulkheads per service
    bulkheads: Arc<RwLock<HashMap<String, Arc<Bulkhead>>>>,
    /// Default policy for unregistered services
    default_policy: ResiliencePolicy,
}

impl ResilienceManager {
    /// Create new resilience manager with default policy
    #[must_use] 
    pub fn new(default_policy: ResiliencePolicy) -> Self {
        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
            bulkheads: Arc::new(RwLock::new(HashMap::new())),
            default_policy,
        }
    }

    /// Create manager with default settings
    #[must_use] 
    pub fn with_defaults() -> Self {
        let default_policy = PolicyBuilder::new()
            .with_timeout(Duration::from_secs(30))
            .with_retry(RetryStrategy::exponential_backoff(
                3,
                Duration::from_millis(100),
            ))
            .build();
        Self::new(default_policy)
    }

    /// Register a service with specific resilience policy
    pub async fn register_service(&self, service: impl Into<String>, policy: ResiliencePolicy) {
        let service_name = service.into();

        // Initialize circuit breaker if configured
        if let Some(ref cb_config) = policy.circuit_breaker {
            let mut breakers = self.circuit_breakers.write().await;
            breakers.insert(
                service_name.clone(),
                Arc::new(CircuitBreaker::with_config(cb_config.clone())),
            );
        }

        // Initialize bulkhead if configured
        if let Some(ref bulkhead_config) = policy.bulkhead {
            let mut bulkheads = self.bulkheads.write().await;
            bulkheads.insert(
                service_name.clone(),
                Arc::new(Bulkhead::new(bulkhead_config.max_concurrency)),
            );
        }

        // Store policy
        let mut policies = self.policies.write().await;
        policies.insert(service_name, Arc::new(policy));
    }

    /// Execute operation with resilience patterns
    pub async fn execute<T, Op>(
        &self,
        service: &str,
        operation_name: &str,
        operation: Op,
    ) -> ResilienceResult<T>
    where
        Op: RetryableOperation<T> + Send + Sync,
        T: Send,
    {
        let mut context = ExecutionContext::new(service, operation_name);
        let policy = self.get_policy(service).await;

        self.execute_with_policy(&mut context, &operation, &policy)
            .await
    }

    /// Execute with a specific policy override
    pub async fn execute_with_override<T, Op>(
        &self,
        service: &str,
        operation_name: &str,
        operation: Op,
        policy_override: ResiliencePolicy,
    ) -> ResilienceResult<T>
    where
        Op: RetryableOperation<T> + Send + Sync,
        T: Send,
    {
        let mut context = ExecutionContext::new(service, operation_name);
        self.execute_with_policy(&mut context, &operation, &policy_override)
            .await
    }

    /// Get policy for service (or default)
    async fn get_policy(&self, service: &str) -> ResiliencePolicy {
        let policies = self.policies.read().await;
        policies
            .get(service).map_or_else(|| self.default_policy.clone(), |p| (**p).clone())
    }

    /// Core execution logic with proper composition and optimized locking
    async fn execute_with_policy<T, Op>(
        &self,
        context: &mut ExecutionContext,
        operation: &Op,
        policy: &ResiliencePolicy,
    ) -> ResilienceResult<T>
    where
        Op: RetryableOperation<T> + Send + Sync,
        T: Send,
    {
        // Optimize: get all required components in one shot to avoid multiple locks
        let (circuit_breaker, bulkhead) = {
            let breakers = self.circuit_breakers.read().await;
            let bulkheads = self.bulkheads.read().await;
            (
                breakers.get(&context.service_name).cloned(),
                bulkheads.get(&context.service_name).cloned(),
            )
        };

        // Check circuit breaker first
        if let Some(ref breaker) = circuit_breaker {
            breaker.can_execute().await?;
        }

        // Acquire bulkhead permit if configured
        let _bulkhead_permit = if let Some(ref bulkhead) = bulkhead {
            Some(bulkhead.acquire().await?)
        } else {
            None
        };

        // Execute with retry if configured
        let result = if let Some(ref retry_strategy) = policy.retry {
            self.execute_with_retry(context, operation, retry_strategy, policy.timeout)
                .await
        } else {
            // Single execution with optional timeout
            self.execute_single(operation, policy.timeout).await
        };

        // Update circuit breaker based on result (avoid re-acquiring lock)
        if let Some(breaker) = circuit_breaker {
            match &result {
                Ok(_) => breaker.record_success().await,
                Err(_e) => breaker.record_failure().await,
            }
        }

        result
    }

    /// Execute with retry logic
    async fn execute_with_retry<T, Op>(
        &self,
        context: &mut ExecutionContext,
        operation: &Op,
        retry_strategy: &RetryStrategy,
        timeout_duration: Option<Duration>,
    ) -> ResilienceResult<T>
    where
        Op: RetryableOperation<T> + Send + Sync,
        T: Send,
    {
        let mut last_error = None;

        for attempt in 1..=retry_strategy.max_attempts {
            context.attempt = attempt;

            let result = self.execute_single(operation, timeout_duration).await;

            match result {
                Ok(value) => return Ok(value),
                Err(e) if attempt == retry_strategy.max_attempts => {
                    last_error = Some(e);
                    break;
                }
                Err(e) if retry_strategy.should_retry(&e) => {
                    last_error = Some(e);

                    // Calculate delay for next attempt
                    if let Some(delay) = retry_strategy.delay_for_attempt(attempt) {
                        tokio::time::sleep(delay).await;
                    }
                }
                Err(e) => {
                    // Non-retryable error
                    return Err(e);
                }
            }
        }

        // All retries exhausted
        Err(ResilienceError::RetryLimitExceeded {
            attempts: retry_strategy.max_attempts,
            last_error: last_error.map(Box::new),
        })
    }

    /// Execute operation once with optional timeout
    async fn execute_single<T, Op>(
        &self,
        operation: &Op,
        timeout_duration: Option<Duration>,
    ) -> ResilienceResult<T>
    where
        Op: RetryableOperation<T> + Send + Sync,
        T: Send,
    {
        if let Some(duration) = timeout_duration {
            match timeout(duration, operation.execute()).await {
                Ok(result) => result,
                Err(_timeout_err) => Err(ResilienceError::Timeout {
                    duration,
                    context: Some("Operation timed out".to_string()),
                }),
            }
        } else {
            operation.execute().await
        }
    }

    /// Get resilience metrics for a service
    ///
    /// Collects metrics from all registered resilience patterns for the specified service.
    /// Returns `None` if the service is not registered.
    ///
    /// # Examples
    ///
    /// ```
    /// # use nebula_resilience::ResilienceManager;
    /// # tokio_test::block_on(async {
    /// let manager = ResilienceManager::new();
    /// manager.register_policy("api", Default::default()).await;
    ///
    /// if let Some(metrics) = manager.get_metrics("api").await {
    ///     println!("Circuit breaker state: {:?}", metrics.circuit_breaker);
    ///     println!("Bulkhead capacity: {:?}", metrics.bulkhead);
    /// }
    /// # });
    /// ```
    pub async fn get_metrics(&self, service: &str) -> Option<ServiceMetrics> {
        // Check if service exists
        if !self.policies.read().await.contains_key(service) {
            return None;
        }

        // Collect circuit breaker stats
        let circuit_breaker = {
            let breakers = self.circuit_breakers.read().await;
            if let Some(cb) = breakers.get(service) {
                Some(cb.stats().await)
            } else {
                None
            }
        };

        // Collect bulkhead stats
        let bulkhead = {
            let bulkheads = self.bulkheads.read().await;
            if let Some(bh) = bulkheads.get(service) {
                Some(bh.stats().await)
            } else {
                None
            }
        };

        Some(ServiceMetrics {
            service_name: service.to_string(),
            circuit_breaker,
            bulkhead,
            total_operations: 0,   // TODO: Track in future with metrics collector
            failed_operations: 0,  // TODO: Track in future with metrics collector
            avg_latency_ms: 0.0,   // TODO: Track in future with metrics collector
        })
    }

    /// Get aggregated metrics for all services
    ///
    /// Returns a map of service name to metrics for all registered services.
    ///
    /// # Examples
    ///
    /// ```
    /// # use nebula_resilience::ResilienceManager;
    /// # tokio_test::block_on(async {
    /// let manager = ResilienceManager::new();
    /// manager.register_policy("api", Default::default()).await;
    /// manager.register_policy("db", Default::default()).await;
    ///
    /// let all_metrics = manager.get_all_metrics().await;
    /// println!("Monitoring {} services", all_metrics.len());
    /// # });
    /// ```
    pub async fn get_all_metrics(&self) -> std::collections::HashMap<String, ServiceMetrics> {
        let mut metrics = std::collections::HashMap::new();

        let services: Vec<String> = self
            .policies
            .read()
            .await
            .keys()
            .cloned()
            .collect();

        for service in services {
            if let Some(service_metrics) = self.get_metrics(&service).await {
                metrics.insert(service.clone(), service_metrics);
            }
        }

        metrics
    }

    /// Remove service and cleanup resources
    pub async fn unregister_service(&self, service: &str) {
        let mut policies = self.policies.write().await;
        policies.remove(service);

        let mut breakers = self.circuit_breakers.write().await;
        breakers.remove(service);

        let mut bulkheads = self.bulkheads.write().await;
        bulkheads.remove(service);
    }

    /// Get all registered services
    pub async fn list_services(&self) -> Vec<String> {
        let policies = self.policies.read().await;
        policies.keys().cloned().collect()
    }
}

impl Default for ResilienceManager {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Service metrics aggregation
#[derive(Debug, Clone)]
pub struct ServiceMetrics {
    /// Service name
    pub service_name: String,
    /// Circuit breaker statistics (if registered)
    pub circuit_breaker: Option<CircuitBreakerStats>,
    /// Bulkhead statistics (if registered)
    pub bulkhead: Option<BulkheadStats>,
    /// Total operations executed
    pub total_operations: u64,
    /// Failed operations count
    pub failed_operations: u64,
    /// Average latency in milliseconds
    pub avg_latency_ms: f64,
}

/// Convenience macro for creating retryable operations
#[macro_export]
macro_rules! retryable {
    ($expr:expr) => {
        move || async move { $expr }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_basic_execution() {
        let manager = ResilienceManager::with_defaults();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let operation = move || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<u32, ResilienceError>(42)
            }
        };

        let result = manager.execute("test-service", "test-op", operation).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_on_failure() {
        let manager = ResilienceManager::with_defaults();
        let policy = PolicyBuilder::new()
            .with_retry(RetryStrategy::fixed_delay(3, Duration::from_millis(10)))
            .build();

        manager.register_service("retry-service", policy).await;

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let operation = move || {
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
                    Ok::<u32, ResilienceError>(100)
                }
            }
        };

        let result = manager
            .execute("retry-service", "retry-op", operation)
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 100);
        assert_eq!(counter.load(Ordering::SeqCst), 3); // Failed twice, succeeded on third
    }
}
