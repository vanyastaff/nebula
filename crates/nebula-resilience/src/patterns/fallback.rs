//! Fallback strategies for graceful degradation

use async_trait::async_trait;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{ResilienceError, ResilienceResult};

/// Fallback strategy trait
#[async_trait]
pub trait FallbackStrategy<T>: Send + Sync {
    /// Execute fallback logic
    async fn fallback(&self, error: ResilienceError) -> ResilienceResult<T>;

    /// Check if fallback should be attempted for this error
    fn should_fallback(&self, error: &ResilienceError) -> bool {
        // Default: fallback for all errors except InvalidConfig
        !matches!(error, ResilienceError::InvalidConfig { .. })
    }
}

/// Simple value fallback
pub struct ValueFallback<T: Clone + Send + Sync> {
    value: T,
}

impl<T: Clone + Send + Sync> ValueFallback<T> {
    /// Create new value fallback
    pub fn new(value: T) -> Self {
        Self { value }
    }
}

#[async_trait]
impl<T: Clone + Send + Sync> FallbackStrategy<T> for ValueFallback<T> {
    async fn fallback(&self, _error: ResilienceError) -> ResilienceResult<T> {
        Ok(self.value.clone())
    }
}

/// Function fallback
pub struct FunctionFallback<T, F, Fut>
where
    F: Fn(ResilienceError) -> Fut + Send + Sync,
    Fut: Future<Output = ResilienceResult<T>> + Send,
{
    function: F,
    _phantom: std::marker::PhantomData<T>,
}

impl<T, F, Fut> FunctionFallback<T, F, Fut>
where
    F: Fn(ResilienceError) -> Fut + Send + Sync,
    Fut: Future<Output = ResilienceResult<T>> + Send,
{
    /// Create new function fallback
    pub fn new(function: F) -> Self {
        Self {
            function,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T, F, Fut> FallbackStrategy<T> for FunctionFallback<T, F, Fut>
where
    T: Send + Sync,
    F: Fn(ResilienceError) -> Fut + Send + Sync,
    Fut: Future<Output = ResilienceResult<T>> + Send,
{
    async fn fallback(&self, error: ResilienceError) -> ResilienceResult<T> {
        (self.function)(error).await
    }
}

/// Cache fallback - returns cached value on error
pub struct CacheFallback<T: Clone + Send + Sync> {
    cache: Arc<RwLock<Option<T>>>,
    ttl: Option<std::time::Duration>,
    last_update: Arc<RwLock<Option<std::time::Instant>>>,
}

impl<T: Clone + Send + Sync> CacheFallback<T> {
    /// Create new cache fallback
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(None)),
            ttl: None,
            last_update: Arc::new(RwLock::new(None)),
        }
    }

    /// Set TTL for cached value
    #[must_use = "builder methods must be chained or built"]
    pub fn with_ttl(mut self, ttl: std::time::Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Update cached value
    pub async fn update(&self, value: T) {
        *self.cache.write().await = Some(value);
        *self.last_update.write().await = Some(std::time::Instant::now());
    }

    /// Check if cache is valid
    async fn is_valid(&self) -> bool {
        if let Some(ttl) = self.ttl
            && let Some(last_update) = *self.last_update.read().await {
                return last_update.elapsed() < ttl;
            }
        true
    }
}

#[async_trait]
impl<T: Clone + Send + Sync> FallbackStrategy<T> for CacheFallback<T> {
    async fn fallback(&self, _error: ResilienceError) -> ResilienceResult<T> {
        if !self.is_valid().await {
            return Err(ResilienceError::FallbackFailed {
                reason: "Cache expired".to_string(),
                original_error: None,
            });
        }

        self.cache
            .read()
            .await
            .clone()
            .ok_or_else(|| ResilienceError::FallbackFailed {
                reason: "No cached value available".to_string(),
                original_error: None,
            })
    }
}

/// Chain fallback - tries multiple fallbacks in sequence
pub struct ChainFallback<T> {
    fallbacks: Vec<Arc<dyn FallbackStrategy<T>>>,
}

impl<T> ChainFallback<T> {
    /// Create new chain fallback
    #[must_use] 
    pub fn new() -> Self {
        Self {
            fallbacks: Vec::new(),
        }
    }

    /// Add a fallback to the chain
    #[must_use = "builder methods must be chained or built"]
    pub fn add(mut self, fallback: Arc<dyn FallbackStrategy<T>>) -> Self {
        self.fallbacks.push(fallback);
        self
    }
}

#[async_trait]
impl<T: Send + Sync> FallbackStrategy<T> for ChainFallback<T> {
    async fn fallback(&self, error: ResilienceError) -> ResilienceResult<T> {
        let mut last_error = error;

        for fallback in &self.fallbacks {
            if fallback.should_fallback(&last_error) {
                match fallback.fallback(last_error.clone()).await {
                    Ok(value) => return Ok(value),
                    Err(e) => last_error = e,
                }
            }
        }

        Err(last_error)
    }
}

/// Priority fallback - selects fallback based on error type
pub struct PriorityFallback<T> {
    fallbacks: HashMap<String, Arc<dyn FallbackStrategy<T>>>,
    default: Option<Arc<dyn FallbackStrategy<T>>>,
}

impl<T> PriorityFallback<T> {
    /// Create new priority fallback
    #[must_use] 
    pub fn new() -> Self {
        Self {
            fallbacks: HashMap::new(),
            default: None,
        }
    }

    /// Register fallback for specific error type
    #[must_use = "builder methods must be chained or built"]
    pub fn register(mut self, error_type: &str, fallback: Arc<dyn FallbackStrategy<T>>) -> Self {
        self.fallbacks.insert(error_type.to_string(), fallback);
        self
    }

    /// Set default fallback
    #[must_use = "builder methods must be chained or built"]
    pub fn with_default(mut self, fallback: Arc<dyn FallbackStrategy<T>>) -> Self {
        self.default = Some(fallback);
        self
    }
}

#[async_trait]
impl<T: Send + Sync> FallbackStrategy<T> for PriorityFallback<T> {
    async fn fallback(&self, error: ResilienceError) -> ResilienceResult<T> {
        let error_type = match &error {
            ResilienceError::Timeout { .. } => "timeout",
            ResilienceError::CircuitBreakerOpen { .. } => "circuit_breaker",
            ResilienceError::BulkheadFull { .. } => "bulkhead",
            ResilienceError::RetryLimitExceeded { .. } => "retry",
            ResilienceError::RateLimitExceeded { .. } => "rate_limit",
            _ => "other",
        };

        if let Some(fallback) = self.fallbacks.get(error_type) {
            return fallback.fallback(error).await;
        }

        if let Some(default) = &self.default {
            return default.fallback(error).await;
        }

        Err(error)
    }
}

/// Enum wrapper for dyn-compatible string fallback strategies
#[derive(Clone)]
pub enum AnyStringFallbackStrategy {
    /// Simple value fallback
    Value(Arc<ValueFallback<String>>),
    /// Cache-based fallback
    Cache(Arc<CacheFallback<String>>),
    /// Chain of fallback strategies
    Chain(Arc<ChainFallback<String>>),
    /// Priority-based fallback selection
    Priority(Arc<PriorityFallback<String>>),
}

#[async_trait]
impl FallbackStrategy<String> for AnyStringFallbackStrategy {
    async fn fallback(&self, error: ResilienceError) -> ResilienceResult<String> {
        match self {
            Self::Value(strategy) => strategy.fallback(error).await,
            Self::Cache(strategy) => strategy.fallback(error).await,
            Self::Chain(strategy) => strategy.fallback(error).await,
            Self::Priority(strategy) => strategy.fallback(error).await,
        }
    }

    fn should_fallback(&self, error: &ResilienceError) -> bool {
        match self {
            Self::Value(strategy) => strategy.should_fallback(error),
            Self::Cache(strategy) => strategy.should_fallback(error),
            Self::Chain(strategy) => strategy.should_fallback(error),
            Self::Priority(strategy) => strategy.should_fallback(error),
        }
    }
}

/// Fallback with operation - combines primary and fallback operations
pub struct FallbackOperation<T> {
    fallback_strategy: Arc<dyn FallbackStrategy<T>>,
}

impl<T> FallbackOperation<T> {
    /// Create new fallback operation
    pub fn new(fallback_strategy: Arc<dyn FallbackStrategy<T>>) -> Self {
        Self { fallback_strategy }
    }

    /// Execute with fallback
    pub async fn execute<F, Fut>(&self, operation: F) -> ResilienceResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ResilienceResult<T>>,
        T: Send + Sync,
    {
        match operation().await {
            Ok(value) => Ok(value),
            Err(error) => {
                if self.fallback_strategy.should_fallback(&error) {
                    self.fallback_strategy.fallback(error).await
                } else {
                    Err(error)
                }
            }
        }
    }
}
