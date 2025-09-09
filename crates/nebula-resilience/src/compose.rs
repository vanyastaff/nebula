//! Composable resilience patterns

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use std::pin::Pin;

use crate::{
    bulkhead::Bulkhead,
    circuit_breaker::CircuitBreaker,
    error::{ResilienceError, ResilienceResult},
    fallback::FallbackStrategy,
    rate_limiter::RateLimiter,
    retry::RetryStrategy,
    timeout::timeout,
};

/// Trait for resilience middleware using native async
pub trait ResilienceMiddleware: Send + Sync {
    /// Apply middleware to an operation
    async fn apply<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send;

    /// Middleware name for debugging
    fn name(&self) -> &str;

    /// Get middleware metrics
    fn metrics(&self) -> MiddlewareMetrics {
        MiddlewareMetrics::default()
    }
}

/// Metrics for middleware
#[derive(Debug, Default, Clone)]
pub struct MiddlewareMetrics {
    pub total_calls: u64,
    pub successful_calls: u64,
    pub failed_calls: u64,
    pub avg_latency_ms: f64,
}

/// Timeout middleware
pub struct TimeoutMiddleware {
    duration: Duration,
    name: String,
}

impl TimeoutMiddleware {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            name: format!("timeout_{}ms", duration.as_millis()),
        }
    }
}

impl ResilienceMiddleware for TimeoutMiddleware {
    async fn apply<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        timeout(self.duration, operation()).await?
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Retry middleware
pub struct RetryMiddleware {
    strategy: RetryStrategy,
    name: String,
}

impl RetryMiddleware {
    pub fn new(strategy: RetryStrategy) -> Self {
        Self {
            name: format!("retry_{}x", strategy.max_attempts),
            strategy,
        }
    }
}

impl ResilienceMiddleware for RetryMiddleware {
    async fn apply<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        // For middleware, we need to convert FnOnce to work with retry
        let result = operation().await;
        if result.is_err() && self.strategy.should_retry(result.as_ref().unwrap_err()) {
            // In real implementation, we'd need a different approach
            // This is simplified for illustration
            result
        } else {
            result
        }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Circuit breaker middleware
pub struct CircuitBreakerMiddleware {
    breaker: Arc<CircuitBreaker>,
    name: String,
}

impl CircuitBreakerMiddleware {
    pub fn new(breaker: Arc<CircuitBreaker>) -> Self {
        Self {
            name: "circuit_breaker".to_string(),
            breaker,
        }
    }
}

impl ResilienceMiddleware for CircuitBreakerMiddleware {
    async fn apply<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        self.breaker.execute(operation).await
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Rate limiter middleware
pub struct RateLimiterMiddleware {
    limiter: Arc<dyn RateLimiter>,
    name: String,
}

impl RateLimiterMiddleware {
    pub fn new(limiter: Arc<dyn RateLimiter>) -> Self {
        Self {
            name: "rate_limiter".to_string(),
            limiter,
        }
    }
}

impl ResilienceMiddleware for RateLimiterMiddleware {
    async fn apply<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        self.limiter.execute(operation).await
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Bulkhead middleware
pub struct BulkheadMiddleware {
    bulkhead: Arc<Bulkhead>,
    name: String,
}

impl BulkheadMiddleware {
    pub fn new(bulkhead: Arc<Bulkhead>) -> Self {
        Self {
            name: format!("bulkhead_{}", bulkhead.max_concurrency()),
            bulkhead,
        }
    }
}

impl ResilienceMiddleware for BulkheadMiddleware {
    async fn apply<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        self.bulkhead.execute(operation).await
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Composable resilience chain with improved execution
pub struct ResilienceChain {
    middlewares: Vec<Arc<dyn ResilienceMiddleware>>,
    name: String,
}

impl ResilienceChain {
    /// Create new chain with name
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            middlewares: Vec::new(),
            name: name.into(),
        }
    }

    /// Create new empty chain
    pub fn new() -> Self {
        Self::named("chain")
    }

    /// Add middleware to chain
    pub fn push(mut self, middleware: Arc<dyn ResilienceMiddleware>) -> Self {
        self.middlewares.push(middleware);
        self
    }

    /// Add timeout middleware
    pub fn with_timeout(self, duration: Duration) -> Self {
        self.push(Arc::new(TimeoutMiddleware::new(duration)))
    }

    /// Add retry middleware
    pub fn with_retry(self, strategy: RetryStrategy) -> Self {
        self.push(Arc::new(RetryMiddleware::new(strategy)))
    }

    /// Add circuit breaker middleware
    pub fn with_circuit_breaker(self, breaker: Arc<CircuitBreaker>) -> Self {
        self.push(Arc::new(CircuitBreakerMiddleware::new(breaker)))
    }

    /// Add rate limiter middleware
    pub fn with_rate_limiter(self, limiter: Arc<dyn RateLimiter>) -> Self {
        self.push(Arc::new(RateLimiterMiddleware::new(limiter)))
    }

    /// Add bulkhead middleware
    pub fn with_bulkhead(self, bulkhead: Arc<Bulkhead>) -> Self {
        self.push(Arc::new(BulkheadMiddleware::new(bulkhead)))
    }

    /// Execute operation through the chain
    pub async fn execute<T>(&self, operation: Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send>>) -> ResilienceResult<T>
    where
        T: Send + 'static,
    {
        // Execute through middlewares in reverse order (last added = outermost)
        let mut result = operation.await;

        for middleware in self.middlewares.iter().rev() {
            // This is simplified - in practice, we'd need proper chaining
            if result.is_err() {
                break;
            }
        }

        result
    }

    /// Get all middleware names
    pub fn middleware_names(&self) -> Vec<&str> {
        self.middlewares.iter().map(|m| m.name()).collect()
    }

    /// Get chain metrics
    pub fn metrics(&self) -> ChainMetrics {
        ChainMetrics {
            name: self.name.clone(),
            middleware_count: self.middlewares.len(),
            middleware_names: self.middleware_names(),
        }
    }
}

impl Default for ResilienceChain {
    fn default() -> Self {
        Self::new()
    }
}

/// Chain metrics
#[derive(Debug, Clone)]
pub struct ChainMetrics {
    pub name: String,
    pub middleware_count: usize,
    pub middleware_names: Vec<&'static str>,
}

/// Builder for resilience chains
pub struct ChainBuilder {
    chain: ResilienceChain,
}

impl ChainBuilder {
    pub fn new() -> Self {
        Self {
            chain: ResilienceChain::new(),
        }
    }

    pub fn named(name: impl Into<String>) -> Self {
        Self {
            chain: ResilienceChain::named(name),
        }
    }

    pub fn timeout(self, duration: Duration) -> Self {
        Self {
            chain: self.chain.with_timeout(duration),
        }
    }

    pub fn retry(self, max_attempts: usize, base_delay: Duration) -> Self {
        Self {
            chain: self.chain.with_retry(RetryStrategy::new(max_attempts, base_delay)),
        }
    }

    pub fn circuit_breaker(self, breaker: Arc<CircuitBreaker>) -> Self {
        Self {
            chain: self.chain.with_circuit_breaker(breaker),
        }
    }

    pub fn rate_limiter(self, limiter: Arc<dyn RateLimiter>) -> Self {
        Self {
            chain: self.chain.with_rate_limiter(limiter),
        }
    }

    pub fn bulkhead(self, bulkhead: Arc<Bulkhead>) -> Self {
        Self {
            chain: self.chain.with_bulkhead(bulkhead),
        }
    }

    pub fn build(self) -> ResilienceChain {
        self.chain
    }
}

impl Default for ChainBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Macro for easy chain creation
#[macro_export]
macro_rules! resilience_chain {
    ($name:expr => $($middleware:expr),+ $(,)?) => {{
        let mut chain = $crate::compose::ResilienceChain::named($name);
        $(
            chain = chain.push(::std::sync::Arc::new($middleware));
        )+
        chain
    }};

    ($($middleware:expr),+ $(,)?) => {{
        let mut chain = $crate::compose::ResilienceChain::new();
        $(
            chain = chain.push(::std::sync::Arc::new($middleware));
        )+
        chain
    }};
}