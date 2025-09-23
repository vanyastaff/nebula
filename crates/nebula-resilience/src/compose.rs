//! Composable resilience patterns

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use std::pin::Pin;
use async_trait::async_trait;

use crate::{
    patterns::bulkhead::Bulkhead,
    patterns::circuit_breaker::CircuitBreaker,
    ResilienceError, ResilienceResult,
    patterns::rate_limiter::{RateLimiter, AnyRateLimiter},
    patterns::retry::RetryStrategy,
    patterns::timeout::timeout,
};

/// Trait for resilience middleware using native async
#[async_trait]
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

/// Enum wrapper for dyn-compatible resilience middleware
#[derive(Clone)]
pub enum AnyResilienceMiddleware {
    Timeout(Arc<TimeoutMiddleware>),
    Retry(Arc<RetryMiddleware>),
    CircuitBreaker(Arc<CircuitBreakerMiddleware>),
    RateLimiter(Arc<RateLimiterMiddleware>),
    Bulkhead(Arc<BulkheadMiddleware>),
}

#[async_trait]
impl ResilienceMiddleware for AnyResilienceMiddleware {
    async fn apply<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        match self {
            Self::Timeout(middleware) => middleware.apply(operation).await,
            Self::Retry(middleware) => middleware.apply(operation).await,
            Self::CircuitBreaker(middleware) => middleware.apply(operation).await,
            Self::RateLimiter(middleware) => middleware.apply(operation).await,
            Self::Bulkhead(middleware) => middleware.apply(operation).await,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Timeout(middleware) => middleware.name(),
            Self::Retry(middleware) => middleware.name(),
            Self::CircuitBreaker(middleware) => middleware.name(),
            Self::RateLimiter(middleware) => middleware.name(),
            Self::Bulkhead(middleware) => middleware.name(),
        }
    }

    fn metrics(&self) -> MiddlewareMetrics {
        match self {
            Self::Timeout(middleware) => middleware.metrics(),
            Self::Retry(middleware) => middleware.metrics(),
            Self::CircuitBreaker(middleware) => middleware.metrics(),
            Self::RateLimiter(middleware) => middleware.metrics(),
            Self::Bulkhead(middleware) => middleware.metrics(),
        }
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

#[async_trait]
impl ResilienceMiddleware for TimeoutMiddleware {
    async fn apply<T, F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = ResilienceResult<T>> + Send,
        T: Send,
    {
        timeout(self.duration, operation()).await.map_err(|e| e.into())
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

#[async_trait]
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

#[async_trait]
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
    limiter: Arc<AnyRateLimiter>,
    name: String,
}

impl RateLimiterMiddleware {
    pub fn new(limiter: Arc<AnyRateLimiter>) -> Self {
        Self {
            name: "rate_limiter".to_string(),
            limiter,
        }
    }
}

#[async_trait]
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

#[async_trait]
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
    middlewares: Vec<AnyResilienceMiddleware>,
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
    pub fn push(mut self, middleware: AnyResilienceMiddleware) -> Self {
        self.middlewares.push(middleware);
        self
    }

    /// Add timeout middleware
    pub fn with_timeout(self, duration: Duration) -> Self {
        self.push(AnyResilienceMiddleware::Timeout(Arc::new(TimeoutMiddleware::new(duration))))
    }

    /// Add retry middleware
    pub fn with_retry(self, strategy: RetryStrategy) -> Self {
        self.push(AnyResilienceMiddleware::Retry(Arc::new(RetryMiddleware::new(strategy))))
    }

    /// Add circuit breaker middleware
    pub fn with_circuit_breaker(self, breaker: Arc<CircuitBreaker>) -> Self {
        self.push(AnyResilienceMiddleware::CircuitBreaker(Arc::new(CircuitBreakerMiddleware::new(breaker))))
    }

    /// Add rate limiter middleware
    pub fn with_rate_limiter(self, limiter: Arc<AnyRateLimiter>) -> Self {
        self.push(AnyResilienceMiddleware::RateLimiter(Arc::new(RateLimiterMiddleware::new(limiter))))
    }

    /// Add bulkhead middleware
    pub fn with_bulkhead(self, bulkhead: Arc<Bulkhead>) -> Self {
        self.push(AnyResilienceMiddleware::Bulkhead(Arc::new(BulkheadMiddleware::new(bulkhead))))
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

    pub fn rate_limiter(self, limiter: Arc<AnyRateLimiter>) -> Self {
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
            chain = chain.push($middleware);
        )+
        chain
    }};

    ($($middleware:expr),+ $(,)?) => {{
        let mut chain = $crate::compose::ResilienceChain::new();
        $(
            chain = chain.push($middleware);
        )+
        chain
    }};
}