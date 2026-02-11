//! Resilience manager for centralized pattern orchestration
//!
//! This module provides type-safe service management using advanced patterns:
//!
//! - **Typed service identifiers** for compile-time service name validation
//! - **Sealed operation traits** for controlled extensibility
//! - **Category markers** for service classification
//! - **Typed metrics** with statically-known dimensions
//!
//! # Type-Safe Service Registration
//!
//! ```rust,ignore
//! use nebula_resilience::manager::{Service, ServiceCategory};
//!
//! // Define typed service identifiers
//! struct DatabaseService;
//! impl Service for DatabaseService {
//!     const NAME: &'static str = "database";
//!     type Category = DatabaseCategory;
//! }
//!
//! // Register with type safety
//! manager.register_service_typed::<DatabaseService>(policy).await;
//! ```

use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;

use crate::{
    ResilienceError, ResilienceResult,
    core::categories::{Category, ServiceCategory},
    patterns::{
        bulkhead::{Bulkhead, BulkheadConfig, BulkheadStats},
        circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitBreakerStats},
        timeout::timeout,
    },
    policy::{ResiliencePolicy, RetryPolicyConfig},
};

// =============================================================================
// SEALED SERVICE CATEGORY TRAIT
// =============================================================================

// Service categories are now imported from unified categories module
// This eliminates duplication and provides consistent categorization

// =============================================================================
// TYPED SERVICE IDENTIFIER
// =============================================================================

/// service identifier for compile-time service name validation.
///
/// Implement this trait for your services to enable type-safe registration
/// and execution with the resilience manager.
///
/// # Example
///
/// ```rust,ignore
/// struct UserService;
/// impl Service for UserService {
///     const NAME: &'static str = "user-service";
///     type Category = HttpCategory;
/// }
/// ```
pub trait Service: Send + Sync + 'static {
    /// Service name (compile-time constant).
    const NAME: &'static str;

    /// Service category.
    type Category: ServiceCategory;

    /// Get the service name.
    fn name() -> &'static str {
        Self::NAME
    }

    /// Get the category name.
    fn category_name() -> &'static str {
        Self::Category::name()
    }
}

/// operation identifier for compile-time operation name validation.
pub trait Operation: Send + Sync + 'static {
    /// Operation name.
    const NAME: &'static str;

    /// Whether this operation is idempotent.
    const IDEMPOTENT: bool = false;

    /// Get the operation name.
    fn name() -> &'static str {
        Self::NAME
    }
}

// =============================================================================
// TYPED EXECUTION CONTEXT
// =============================================================================

/// execution context with compile-time service and operation validation
#[derive(Debug)]
#[cfg(test)]
pub struct ExecutionContext<S: Service, O: Operation = DefaultOperation> {
    /// Current attempt number.
    pub attempt: usize,
    /// Start time.
    pub start_time: std::time::Instant,
    _marker: PhantomData<(S, O)>,
}

#[cfg(test)]
impl<S: Service, O: Operation> ExecutionContext<S, O> {
    /// Create new typed execution context.
    pub fn new() -> Self {
        Self {
            attempt: 1,
            start_time: std::time::Instant::now(),
            _marker: PhantomData,
        }
    }

    /// Get service name.
    pub fn service_name(&self) -> &'static str {
        S::NAME
    }

    /// Get operation name.
    pub fn operation_name(&self) -> &'static str {
        O::NAME
    }

    /// Get category name.
    pub fn category(&self) -> &'static str {
        S::Category::name()
    }

    /// Check if operation is idempotent.
    pub fn is_idempotent(&self) -> bool {
        O::IDEMPOTENT
    }

    /// Increment attempt counter
    pub(crate) fn next_attempt(&mut self) {
        self.attempt += 1;
    }

    /// Get elapsed execution time
    pub(crate) fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }
}

#[cfg(test)]
impl<S: Service, O: Operation> Default for ExecutionContext<S, O> {
    fn default() -> Self {
        Self::new()
    }
}

/// Default operation type when no specific operation is specified.
#[cfg(test)]
pub struct DefaultOperation;

#[cfg(test)]
impl Operation for DefaultOperation {
    const NAME: &'static str = "default";
    const IDEMPOTENT: bool = false;
}

// =============================================================================
// TYPED SERVICE METRICS
// =============================================================================

/// service metrics with statically-known dimensions.
#[derive(Debug, Clone)]
pub struct ServiceMetrics<S: Service> {
    /// Circuit breaker statistics.
    pub circuit_breaker: Option<CircuitBreakerStats>,
    /// Bulkhead statistics.
    pub bulkhead: Option<BulkheadStats>,
    /// Total operations executed.
    pub total_operations: u64,
    /// Failed operations count.
    pub failed_operations: u64,
    /// Average latency in milliseconds.
    pub avg_latency_ms: f64,
    _marker: PhantomData<S>,
}

impl<S: Service> ServiceMetrics<S> {
    /// Get service name.
    pub const fn service_name(&self) -> &'static str {
        S::NAME
    }

    /// Get category name.
    pub fn category(&self) -> &'static str {
        S::Category::name()
    }

    /// Success rate (0.0 - 1.0).
    pub fn success_rate(&self) -> f64 {
        if self.total_operations == 0 {
            1.0
        } else {
            (self.total_operations - self.failed_operations) as f64 / self.total_operations as f64
        }
    }
}

// =============================================================================
// ORIGINAL IMPLEMENTATION (kept for backwards compatibility)
// =============================================================================

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
    retry: Option<RetryPolicyConfig>,
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
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Set retry strategy with exponential backoff
    #[must_use = "builder methods must be chained or built"]
    pub const fn with_retry_exponential(
        mut self,
        max_attempts: usize,
        base_delay: Duration,
    ) -> Self {
        self.retry = Some(RetryPolicyConfig::exponential(max_attempts, base_delay));
        self
    }

    /// Set retry strategy with fixed delay
    #[must_use = "builder methods must be chained or built"]
    pub const fn with_retry_fixed(mut self, max_attempts: usize, delay: Duration) -> Self {
        self.retry = Some(RetryPolicyConfig::fixed(max_attempts, delay));
        self
    }

    /// Set retry configuration directly
    #[must_use = "builder methods must be chained or built"]
    pub const fn with_retry(mut self, config: RetryPolicyConfig) -> Self {
        self.retry = Some(config);
        self
    }

    /// Set circuit breaker configuration
    #[must_use = "builder methods must be chained or built"]
    pub const fn with_circuit_breaker(mut self, config: CircuitBreakerConfig) -> Self {
        self.circuit_breaker = Some(config);
        self
    }

    /// Set bulkhead configuration
    #[must_use = "builder methods must be chained or built"]
    pub const fn with_bulkhead(mut self, config: BulkheadConfig) -> Self {
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
pub struct UnTypedExecutionContext {
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

impl UnTypedExecutionContext {
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
    pub(crate) const fn next_attempt(&mut self) {
        self.attempt += 1;
    }

    /// Get elapsed time since start (for latency metrics)
    #[allow(dead_code)]
    pub(crate) fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }
}

/// Modern resilience manager with proper async semantics and concurrent access
///
/// Uses `DashMap` for lock-free concurrent reads, optimized for high-throughput scenarios.
#[derive(Debug)]
pub struct ResilienceManager {
    /// Service policies (concurrent `HashMap` for lock-free reads)
    policies: Arc<DashMap<String, Arc<ResiliencePolicy>>>,
    /// Circuit breakers per service (concurrent `HashMap`)
    circuit_breakers: Arc<DashMap<String, Arc<CircuitBreaker>>>,
    /// Bulkheads per service (concurrent `HashMap`)
    bulkheads: Arc<DashMap<String, Arc<Bulkhead>>>,
    /// Default policy for unregistered services (Arc for cheap cloning)
    default_policy: Arc<ResiliencePolicy>,
}

impl ResilienceManager {
    /// Create new resilience manager with default policy
    #[must_use]
    pub fn new(default_policy: ResiliencePolicy) -> Self {
        Self {
            policies: Arc::new(DashMap::new()),
            circuit_breakers: Arc::new(DashMap::new()),
            bulkheads: Arc::new(DashMap::new()),
            default_policy: Arc::new(default_policy),
        }
    }

    /// Create manager with default settings
    #[must_use]
    pub fn with_defaults() -> Self {
        let default_policy = PolicyBuilder::new()
            .with_timeout(Duration::from_secs(30))
            .with_retry_exponential(3, Duration::from_millis(100))
            .build();
        Self::new(default_policy)
    }

    /// Register a service with specific resilience policy
    pub fn register_service(&self, service: impl Into<String>, policy: ResiliencePolicy) {
        let service_name = service.into();

        // Initialize circuit breaker if configured
        if policy.circuit_breaker.is_some() {
            // Use default circuit breaker config (const generic version with defaults)
            if let Ok(cb) = CircuitBreaker::with_defaults() {
                self.circuit_breakers
                    .insert(service_name.clone(), Arc::new(cb));
            }
        }

        // Initialize bulkhead if configured
        if let Some(ref bulkhead_config) = policy.bulkhead {
            self.bulkheads.insert(
                service_name.clone(),
                Arc::new(Bulkhead::new(bulkhead_config.max_concurrency)),
            );
        }

        // Store policy (DashMap provides lock-free concurrent writes)
        self.policies.insert(service_name, Arc::new(policy));
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
        let mut context = UnTypedExecutionContext::new(service, operation_name);
        let policy = self.get_policy(service);

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
        let mut context = UnTypedExecutionContext::new(service, operation_name);
        self.execute_with_policy(&mut context, &operation, &policy_override)
            .await
    }

    /// Get policy for service (or default)
    ///
    /// Returns Arc for cheap cloning - lock-free read with `DashMap`
    fn get_policy(&self, service: &str) -> Arc<ResiliencePolicy> {
        self.policies.get(service).map_or_else(
            || Arc::clone(&self.default_policy),
            |entry| Arc::clone(entry.value()),
        )
    }

    /// Core execution logic with proper composition and optimized locking
    async fn execute_with_policy<T, Op>(
        &self,
        context: &mut UnTypedExecutionContext,
        operation: &Op,
        policy: &ResiliencePolicy,
    ) -> ResilienceResult<T>
    where
        Op: RetryableOperation<T> + Send + Sync,
        T: Send,
    {
        // Get components with lock-free reads from DashMap
        let circuit_breaker = self
            .circuit_breakers
            .get(&context.service_name)
            .map(|entry| Arc::clone(entry.value()));

        let bulkhead = self
            .bulkheads
            .get(&context.service_name)
            .map(|entry| Arc::clone(entry.value()));

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
        let result = if let Some(ref retry_config) = policy.retry {
            self.execute_with_retry(context, operation, retry_config, policy.timeout)
                .await
        } else {
            // Single execution with optional timeout
            self.execute_single(operation, policy.timeout).await
        };

        // Update circuit breaker based on result
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
        context: &mut UnTypedExecutionContext,
        operation: &Op,
        retry_config: &RetryPolicyConfig,
        timeout_duration: Option<Duration>,
    ) -> ResilienceResult<T>
    where
        Op: RetryableOperation<T> + Send + Sync,
        T: Send,
    {
        let mut last_error = None;

        for attempt in 0..retry_config.max_attempts {
            context.attempt = attempt + 1;

            let result = self.execute_single(operation, timeout_duration).await;

            match result {
                Ok(value) => return Ok(value),
                Err(e) if attempt + 1 == retry_config.max_attempts => {
                    last_error = Some(e);
                    break;
                }
                Err(e) if Self::should_retry(&e) => {
                    last_error = Some(e);

                    // Calculate delay for next attempt
                    if let Some(delay) = retry_config.delay_for_attempt(attempt) {
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
            attempts: retry_config.max_attempts,
            last_error: last_error.map(Box::new),
        })
    }

    /// Check if error is retryable
    const fn should_retry(error: &ResilienceError) -> bool {
        error.is_retryable()
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
    pub async fn get_metrics(&self, service: &str) -> Option<UnTypedServiceMetrics> {
        // Check if service exists (lock-free read)
        if !self.policies.contains_key(service) {
            return None;
        }

        // Collect circuit breaker stats (lock-free read)
        let circuit_breaker = match self.circuit_breakers.get(service) {
            Some(cb) => Some(cb.value().stats().await),
            None => None,
        };

        // Collect bulkhead stats (lock-free read)
        let bulkhead = self.bulkheads.get(service).map(|bh| bh.value().stats());

        Some(UnTypedServiceMetrics {
            service_name: service.to_string(),
            circuit_breaker,
            bulkhead,
            total_operations: 0,
            failed_operations: 0,
            avg_latency_ms: 0.0,
        })
    }

    /// Get aggregated metrics for all services
    pub async fn get_all_metrics(
        &self,
    ) -> std::collections::HashMap<String, UnTypedServiceMetrics> {
        let mut metrics = std::collections::HashMap::new();

        let services: Vec<String> = self
            .policies
            .iter()
            .map(|entry| entry.key().clone())
            .collect();

        for service in services {
            if let Some(service_metrics) = self.get_metrics(&service).await {
                metrics.insert(service.clone(), service_metrics);
            }
        }

        metrics
    }

    /// Remove service and cleanup resources
    pub fn unregister_service(&self, service: &str) {
        self.policies.remove(service);
        self.circuit_breakers.remove(service);
        self.bulkheads.remove(service);
    }

    /// Get all registered services
    #[must_use]
    pub fn list_services(&self) -> Vec<String> {
        self.policies
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    // =========================================================================
    // TYPED API METHODS
    // =========================================================================

    /// Register a service with category-based default policy.
    pub fn register_service_typed<S: Service>(&self, policy: ResiliencePolicy) {
        self.register_service(S::NAME, policy);
    }

    /// Register a service with category defaults.
    pub fn register_service_with_defaults<S: Service>(&self) {
        let policy = PolicyBuilder::new()
            .with_timeout(S::Category::default_timeout())
            .with_retry_exponential(
                S::Category::default_retry_attempts(),
                Duration::from_millis(100),
            )
            .build();
        self.register_service(S::NAME, policy);
    }

    /// Execute operation with typed service and operation identifiers.
    pub async fn execute_typed<S, O, T, Op>(&self, operation: Op) -> ResilienceResult<T>
    where
        S: Service,
        O: Operation,
        Op: RetryableOperation<T> + Send + Sync,
        T: Send,
    {
        self.execute(S::NAME, O::NAME, operation).await
    }

    /// Get metrics for a service.
    pub async fn get_service_metrics<S: Service>(&self) -> Option<ServiceMetrics<S>> {
        let metrics = self.get_metrics(S::NAME).await?;
        Some(ServiceMetrics {
            circuit_breaker: metrics.circuit_breaker,
            bulkhead: metrics.bulkhead,
            total_operations: metrics.total_operations,
            failed_operations: metrics.failed_operations,
            avg_latency_ms: metrics.avg_latency_ms,
            _marker: PhantomData,
        })
    }

    /// Unregister a service.
    pub fn unregister_service_typed<S: Service>(&self) {
        self.unregister_service(S::NAME);
    }

    /// Check if a service is registered.
    #[must_use]
    pub fn is_service_registered<S: Service>(&self) -> bool {
        self.policies.contains_key(S::NAME)
    }
}

impl Default for ResilienceManager {
    fn default() -> Self {
        Self::with_defaults()
    }
}

/// Service metrics aggregation
#[derive(Debug, Clone)]
pub struct UnTypedServiceMetrics {
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
    use crate::core::categories::{Cache, Database, Http, MessageQueue};
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
    #[expect(clippy::excessive_nesting)]
    async fn test_retry_on_failure() {
        let manager = ResilienceManager::with_defaults();
        let policy = PolicyBuilder::new()
            .with_retry_fixed(3, Duration::from_millis(10))
            .build();

        manager.register_service("retry-service", policy);

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
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    // =========================================================================
    // TYPED API TESTS
    // =========================================================================

    #[test]
    fn test_service_categories() {
        // Database category
        assert_eq!(Database::name(), "database");
        assert_eq!(Database::default_timeout(), Duration::from_secs(5));
        assert_eq!(Database::default_retry_attempts(), 2);
        assert!(Database::is_critical());

        // Test HTTP category
        assert_eq!(Http::name(), "http");
        assert_eq!(Http::default_timeout(), Duration::from_secs(10));
        assert_eq!(Http::default_retry_attempts(), 3);
        assert!(!Http::is_critical());

        // Test Cache category
        assert_eq!(Cache::name(), "cache");
        assert_eq!(Cache::default_timeout(), Duration::from_millis(500));
        assert_eq!(Cache::default_retry_attempts(), 1);

        // Test Message Queue category
        assert_eq!(MessageQueue::name(), "message_queue");
        assert_eq!(MessageQueue::default_retry_attempts(), 5);
    }

    // Define test services
    struct TestDatabaseService;
    impl Service for TestDatabaseService {
        const NAME: &'static str = "test-database";
        type Category = Database;
    }

    struct TestHttpService;
    impl Service for TestHttpService {
        const NAME: &'static str = "test-http";
        type Category = Http;
    }

    // Define test operations
    struct QueryOperation;
    impl Operation for QueryOperation {
        const NAME: &'static str = "query";
        const IDEMPOTENT: bool = true;
    }

    struct WriteOperation;
    impl Operation for WriteOperation {
        const NAME: &'static str = "write";
        const IDEMPOTENT: bool = false;
    }

    #[test]
    fn test_typed_service_id() {
        assert_eq!(TestDatabaseService::name(), "test-database");
        assert_eq!(TestDatabaseService::category_name(), "database");

        assert_eq!(TestHttpService::name(), "test-http");
        assert_eq!(TestHttpService::category_name(), "http");
    }

    #[test]
    fn test_typed_operation_id() {
        assert_eq!(QueryOperation::name(), "query");
        const { assert!(QueryOperation::IDEMPOTENT) };

        assert_eq!(WriteOperation::name(), "write");
        const { assert!(!WriteOperation::IDEMPOTENT) };
    }

    #[test]
    fn test_execution_context() {
        let mut ctx = ExecutionContext::<TestDatabaseService, QueryOperation>::new();

        assert_eq!(ctx.service_name(), "test-database");
        assert_eq!(ctx.operation_name(), "query");
        assert_eq!(ctx.category(), "database");
        assert!(ctx.is_idempotent());
        assert_eq!(ctx.attempt, 1);

        ctx.next_attempt();
        assert_eq!(ctx.attempt, 2);

        // Check elapsed time is non-negative
        assert!(ctx.elapsed() >= Duration::ZERO);
    }

    #[test]
    fn test_service_metrics() {
        let metrics = ServiceMetrics::<TestDatabaseService> {
            circuit_breaker: None,
            bulkhead: None,
            total_operations: 100,
            failed_operations: 10,
            avg_latency_ms: 5.5,
            _marker: PhantomData,
        };

        assert_eq!(metrics.service_name(), "test-database");
        assert_eq!(metrics.category(), "database");
        assert!((metrics.success_rate() - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn test_success_rate_zero_operations() {
        let metrics = ServiceMetrics::<TestHttpService> {
            circuit_breaker: None,
            bulkhead: None,
            total_operations: 0,
            failed_operations: 0,
            avg_latency_ms: 0.0,
            _marker: PhantomData,
        };

        // Zero operations should return 100% success rate
        assert!((metrics.success_rate() - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_typed_service_registration() {
        let manager = ResilienceManager::with_defaults();

        // Register with defaults
        manager.register_service_with_defaults::<TestDatabaseService>();
        assert!(manager.is_service_registered::<TestDatabaseService>());

        // Custom policy
        let custom_policy = PolicyBuilder::new()
            .with_timeout(Duration::from_secs(1))
            .build();
        manager.register_service_typed::<TestHttpService>(custom_policy);
        assert!(manager.is_service_registered::<TestHttpService>());

        // Unregister
        manager.unregister_service_typed::<TestDatabaseService>();
        assert!(!manager.is_service_registered::<TestDatabaseService>());
    }

    #[tokio::test]
    async fn test_typed_execution() {
        let manager = ResilienceManager::with_defaults();
        manager.register_service_with_defaults::<TestDatabaseService>();

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        let operation = move || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<u32, ResilienceError>(42)
            }
        };

        let result = manager
            .execute_typed::<TestDatabaseService, QueryOperation, _, _>(operation)
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }
}
