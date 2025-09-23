//! Advanced resilience manager combining all patterns

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::{
    patterns::bulkhead::Bulkhead,
    patterns::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig},
    ResilienceError, ResilienceResult,
    patterns::fallback::{FallbackStrategy, AnyStringFallbackStrategy},
    patterns::hedge::{HedgeExecutor, HedgeConfig},
    policy::ResiliencePolicy,
    patterns::rate_limiter::{RateLimiter, AnyRateLimiter},
    patterns::retry::{RetryStrategy, retry},
    patterns::timeout::timeout,
};

/// Resilience manager for managing multiple resilience patterns
pub struct ResilienceManager {
    /// Named policies
    policies: Arc<RwLock<HashMap<String, Arc<ResiliencePolicy>>>>,
    /// Circuit breakers by service
    circuit_breakers: Arc<RwLock<HashMap<String, Arc<CircuitBreaker>>>>,
    /// Rate limiters by service
    rate_limiters: Arc<RwLock<HashMap<String, Arc<AnyRateLimiter>>>>,
    /// Bulkheads by service
    bulkheads: Arc<RwLock<HashMap<String, Arc<Bulkhead>>>>,
    /// Fallback strategies
    fallbacks: Arc<RwLock<HashMap<String, Arc<AnyStringFallbackStrategy>>>>,
    /// Hedge executors
    hedge_executors: Arc<RwLock<HashMap<String, Arc<HedgeExecutor>>>>,
    /// Global configuration
    config: ResilienceManagerConfig,
    /// Metrics collector
    metrics: Arc<ResilienceMetrics>,
}

/// Resilience manager configuration
#[derive(Debug, Clone)]
pub struct ResilienceManagerConfig {
    /// Default timeout
    pub default_timeout: Duration,
    /// Enable adaptive rate limiting
    pub adaptive_rate_limiting: bool,
    /// Enable hedge requests
    pub enable_hedging: bool,
    /// Collect detailed metrics
    pub collect_metrics: bool,
    /// Circuit breaker config
    pub default_circuit_breaker: CircuitBreakerConfig,
    /// Default retry strategy
    pub default_retry: RetryStrategy,
}

impl Default for ResilienceManagerConfig {
    fn default() -> Self {
        Self {
            default_timeout: Duration::from_secs(30),
            adaptive_rate_limiting: true,
            enable_hedging: false,
            collect_metrics: true,
            default_circuit_breaker: CircuitBreakerConfig::default(),
            default_retry: RetryStrategy::default(),
        }
    }
}

impl ResilienceManager {
    /// Create new resilience manager
    pub fn new(config: ResilienceManagerConfig) -> Self {
        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
            rate_limiters: Arc::new(RwLock::new(HashMap::new())),
            bulkheads: Arc::new(RwLock::new(HashMap::new())),
            fallbacks: Arc::new(RwLock::new(HashMap::new())),
            hedge_executors: Arc::new(RwLock::new(HashMap::new())),
            config,
            metrics: Arc::new(ResilienceMetrics::new()),
        }
    }

    /// Create builder for resilience manager
    pub fn builder() -> ResilienceManagerBuilder {
        ResilienceManagerBuilder::default()
    }

    /// Register a service with resilience configuration
    pub async fn register_service(&self, service: &str, policy: ResiliencePolicy) {
        let mut policies = self.policies.write().await;
        policies.insert(service.to_string(), Arc::new(policy));

        // Initialize components based on policy
        if let Some(cb_config) = &policy.circuit_breaker {
            let mut breakers = self.circuit_breakers.write().await;
            breakers.insert(
                service.to_string(),
                Arc::new(CircuitBreaker::with_config(cb_config.clone())),
            );
        }

        if let Some(max_concurrency) = policy.bulkhead {
            let mut bulkheads = self.bulkheads.write().await;
            bulkheads.insert(
                service.to_string(),
                Arc::new(Bulkhead::new(max_concurrency)),
            );
        }
    }

    /// Execute operation with resilience for a service
    pub async fn execute<T, F, Fut>(
        &self,
        service: &str,
        operation: F,
    ) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
        T: Clone + Send + Sync + 'static,
    {
        let start = std::time::Instant::now();

        // Get policy for service
        let policy = {
            let policies = self.policies.read().await;
            policies.get(service).cloned()
        };

        let result = if let Some(policy) = policy {
            self.execute_with_policy(service, operation, &policy).await
        } else {
            // Use default policy
            self.execute_with_defaults(service, operation).await
        };

        // Record metrics
        if self.config.collect_metrics {
            self.metrics.record(service, start.elapsed(), result.is_ok()).await;
        }

        result
    }

    /// Execute with specific policy
    async fn execute_with_policy<T, F, Fut>(
        &self,
        service: &str,
        operation: F,
        policy: &ResiliencePolicy,
    ) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
        T: Clone + Send + Sync,
    {
        // Apply rate limiting
        if let Some(limiter) = self.rate_limiters.read().await.get(service) {
            limiter.acquire().await?;
        }

        // Apply bulkhead
        if let Some(bulkhead) = self.bulkheads.read().await.get(service) {
            return bulkhead.execute(|| async {
                self.execute_core(service, operation, policy).await
            }).await;
        }

        self.execute_core(service, operation, policy).await
    }

    /// Core execution with timeout, retry, and circuit breaker
    async fn execute_core<T, F, Fut>(
        &self,
        service: &str,
        operation: F,
        policy: &ResiliencePolicy,
    ) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
        T: Clone,
    {
        // Apply circuit breaker if configured
        if let Some(breaker) = self.circuit_breakers.read().await.get(service) {
            breaker.can_execute().await?;
        }

        // Execute with retry and timeout
        let result = if let Some(retry_strategy) = &policy.retry {
            retry(retry_strategy.clone(), || async {
                if let Some(timeout_duration) = policy.timeout {
                    timeout(timeout_duration, operation()).await?
                } else {
                    operation().await
                }
            }).await
        } else if let Some(timeout_duration) = policy.timeout {
            timeout(timeout_duration, operation()).await?
        } else {
            operation().await
        };

        // Record result in circuit breaker
        if let Some(breaker) = self.circuit_breakers.read().await.get(service) {
            match &result {
                Ok(_) => breaker.record_success().await,
                Err(e) => {
                    if !matches!(e, ResilienceError::Timeout { .. }) {
                        breaker.record_failure().await;
                    }
                }
            }
        }

        result
    }

    /// Execute with default configuration
    async fn execute_with_defaults<T, F, Fut>(
        &self,
        service: &str,
        operation: F,
    ) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
        T: Clone,
    {
        retry(self.config.default_retry.clone(), || async {
            timeout(self.config.default_timeout, operation()).await?
        }).await
    }

    /// Set rate limiter for a service
    pub async fn set_rate_limiter(&self, service: &str, limiter: Arc<AnyRateLimiter>) {
        let mut limiters = self.rate_limiters.write().await;
        limiters.insert(service.to_string(), limiter);
    }

    /// Set fallback strategy for a service
    pub async fn set_fallback(&self, service: &str, fallback: Arc<AnyStringFallbackStrategy>) {
        let mut fallbacks = self.fallbacks.write().await;
        fallbacks.insert(service.to_string(), fallback);
    }

    /// Enable hedging for a service
    pub async fn enable_hedging(&self, service: &str, config: HedgeConfig) {
        let mut executors = self.hedge_executors.write().await;
        executors.insert(service.to_string(), Arc::new(HedgeExecutor::new(config)));
    }

    /// Get circuit breaker state for a service
    pub async fn circuit_breaker_state(&self, service: &str) -> Option<String> {
        let breakers = self.circuit_breakers.read().await;
        if let Some(breaker) = breakers.get(service) {
            let state = breaker.state().await;
            Some(format!("{:?}", state))
        } else {
            None
        }
    }

    /// Reset circuit breaker for a service
    pub async fn reset_circuit_breaker(&self, service: &str) -> ResilienceResult<()> {
        let breakers = self.circuit_breakers.read().await;
        if let Some(breaker) = breakers.get(service) {
            breaker.reset().await;
            Ok(())
        } else {
            Err(ResilienceError::InvalidConfig {
                message: format!("No circuit breaker for service: {}", service),
            })
        }
    }

    /// Get metrics for a service
    pub async fn metrics(&self, service: &str) -> ServiceMetrics {
        self.metrics.get(service).await
    }
}

/// Resilience manager builder
pub struct ResilienceManagerBuilder {
    config: ResilienceManagerConfig,
    services: Vec<(String, ResiliencePolicy)>,
}

impl Default for ResilienceManagerBuilder {
    fn default() -> Self {
        Self {
            config: ResilienceManagerConfig::default(),
            services: Vec::new(),
        }
    }
}

impl ResilienceManagerBuilder {
    /// Set default timeout
    pub fn default_timeout(mut self, timeout: Duration) -> Self {
        self.config.default_timeout = timeout;
        self
    }

    /// Enable adaptive rate limiting
    pub fn adaptive_rate_limiting(mut self, enabled: bool) -> Self {
        self.config.adaptive_rate_limiting = enabled;
        self
    }

    /// Enable hedging
    pub fn enable_hedging(mut self, enabled: bool) -> Self {
        self.config.enable_hedging = enabled;
        self
    }

    /// Register a service
    pub fn service(mut self, name: &str, policy: ResiliencePolicy) -> Self {
        self.services.push((name.to_string(), policy));
        self
    }

    /// Build the manager
    pub async fn build(self) -> ResilienceManager {
        let manager = ResilienceManager::new(self.config);

        for (service, policy) in self.services {
            manager.register_service(&service, policy).await;
        }

        manager
    }
}

/// Resilience metrics
struct ResilienceMetrics {
    metrics: Arc<RwLock<HashMap<String, ServiceMetrics>>>,
}

impl ResilienceMetrics {
    fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn record(&self, service: &str, latency: Duration, success: bool) {
        let mut metrics = self.metrics.write().await;
        let entry = metrics.entry(service.to_string()).or_default();

        entry.total_requests += 1;
        if success {
            entry.successful_requests += 1;
        } else {
            entry.failed_requests += 1;
        }

        entry.total_latency += latency;
        entry.min_latency = entry.min_latency.min(latency);
        entry.max_latency = entry.max_latency.max(latency);
    }

    async fn get(&self, service: &str) -> ServiceMetrics {
        let metrics = self.metrics.read().await;
        metrics.get(service).cloned().unwrap_or_default()
    }
}

/// Service metrics
#[derive(Debug, Clone, Default)]
pub struct ServiceMetrics {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub total_latency: Duration,
    pub min_latency: Duration,
    pub max_latency: Duration,
}

impl ServiceMetrics {
    /// Calculate average latency
    pub fn avg_latency(&self) -> Duration {
        if self.total_requests > 0 {
            self.total_latency / self.total_requests as u32
        } else {
            Duration::ZERO
        }
    }

    /// Calculate success rate
    pub fn success_rate(&self) -> f64 {
        if self.total_requests > 0 {
            self.successful_requests as f64 / self.total_requests as f64
        } else {
            0.0
        }
    }
}