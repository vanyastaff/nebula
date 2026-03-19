//! Fallback strategies for graceful degradation.
//!
//! Provides fallback mechanisms to maintain service availability when primary operations fail.
//!
//! # Example
//!
//! ```rust
//! use nebula_resilience::patterns::fallback::ValueFallback;
//!
//! // Return a default value on failure
//! let fallback = ValueFallback::new("default response".to_string());
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{ResilienceError, ResilienceResult};

// =============================================================================
// FALLBACK STRATEGY TRAIT
// =============================================================================

/// Fallback strategy trait.
///
/// Implement this trait to define custom fallback behavior.
pub trait FallbackStrategy<T>: Send + Sync {
    /// Execute fallback logic
    fn fallback<'a>(
        &'a self,
        error: ResilienceError,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>>;

    /// Check if fallback should be attempted for this error
    fn should_fallback(&self, error: &ResilienceError) -> bool {
        // Default: fallback for all errors except InvalidConfig
        !matches!(error, ResilienceError::InvalidConfig { .. })
    }
}

/// Simple value fallback.
///
/// Returns a predetermined value when the primary operation fails.
#[derive(Debug, Clone)]
#[must_use = "ValueFallback should be used as a fallback strategy"]
pub struct ValueFallback<T: Clone + Send + Sync> {
    value: T,
}

impl<T: Clone + Send + Sync> ValueFallback<T> {
    /// Create new value fallback.
    pub const fn new(value: T) -> Self {
        Self { value }
    }

    /// Returns a reference to the fallback value.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }
}

impl<T: Clone + Send + Sync> FallbackStrategy<T> for ValueFallback<T> {
    fn fallback<'a>(
        &'a self,
        _error: ResilienceError,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        let value = self.value.clone();
        Box::pin(async move { Ok(value) })
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
    pub const fn new(function: F) -> Self {
        Self {
            function,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T, F, Fut> FallbackStrategy<T> for FunctionFallback<T, F, Fut>
where
    T: Send + Sync + 'static,
    F: Fn(ResilienceError) -> Fut + Send + Sync,
    Fut: Future<Output = ResilienceResult<T>> + Send,
{
    fn fallback<'a>(
        &'a self,
        error: ResilienceError,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        Box::pin((self.function)(error))
    }
}

/// Cache fallback - returns cached value on error
pub struct CacheFallback<T: Clone + Send + Sync> {
    cache: Arc<RwLock<Option<T>>>,
    ttl: Option<std::time::Duration>,
    stale_if_error: bool,
    last_update: Arc<RwLock<Option<std::time::Instant>>>,
}

impl<T: Clone + Send + Sync> Default for CacheFallback<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Send + Sync> CacheFallback<T> {
    /// Create new cache fallback
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(None)),
            ttl: None,
            stale_if_error: false,
            last_update: Arc::new(RwLock::new(None)),
        }
    }

    /// Set TTL for cached value
    #[must_use = "builder methods must be chained or built"]
    pub const fn with_ttl(mut self, ttl: std::time::Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Allow serving stale cached value when TTL is exceeded.
    ///
    /// When enabled, expired cache entries can still be returned during fallback
    /// instead of failing closed with `FallbackFailed`.
    #[must_use = "builder methods must be chained or built"]
    pub const fn with_stale_if_error(mut self, enabled: bool) -> Self {
        self.stale_if_error = enabled;
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
            && let Some(last_update) = *self.last_update.read().await
        {
            return last_update.elapsed() < ttl;
        }
        true
    }
}

impl<T: Clone + Send + Sync + 'static> FallbackStrategy<T> for CacheFallback<T> {
    fn fallback<'a>(
        &'a self,
        _error: ResilienceError,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        Box::pin(async move {
            if !self.is_valid().await {
                return self.stale_fallback().await;
            }

            self.cache
                .read()
                .await
                .clone()
                .ok_or_else(|| ResilienceError::FallbackFailed {
                    reason: "No cached value available".to_string(),
                    original_error: None,
                })
        })
    }
}

impl<T: Clone + Send + Sync> CacheFallback<T> {
    /// Handle stale cache fallback when the cache is expired.
    async fn stale_fallback(&self) -> ResilienceResult<T> {
        if self.stale_if_error {
            let cached_value = self.cache.read().await.clone();
            if let Some(value) = cached_value {
                return Ok(value);
            }
            return Err(ResilienceError::FallbackFailed {
                reason: "Cache expired and no stale value available".to_string(),
                original_error: None,
            });
        }
        Err(ResilienceError::FallbackFailed {
            reason: "Cache expired".to_string(),
            original_error: None,
        })
    }
}

/// Chain fallback - tries multiple fallbacks in sequence
pub struct ChainFallback<T> {
    fallbacks: Vec<Arc<dyn FallbackStrategy<T>>>,
}

impl<T> Default for ChainFallback<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ChainFallback<T> {
    /// Create new chain fallback
    #[must_use]
    pub fn new() -> Self {
        Self {
            fallbacks: Vec::new(),
        }
    }

    /// Add a fallback to the chain.
    // Reason: `add` is a builder method, not the `Add` trait operator.
    #[allow(clippy::should_implement_trait)]
    #[must_use = "builder methods must be chained or built"]
    pub fn add(mut self, fallback: Arc<dyn FallbackStrategy<T>>) -> Self {
        self.fallbacks.push(fallback);
        self
    }
}

impl<T: Send + Sync + 'static> FallbackStrategy<T> for ChainFallback<T> {
    fn fallback<'a>(
        &'a self,
        error: ResilienceError,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        Box::pin(async move {
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
        })
    }
}

/// Priority fallback - selects fallback based on error type
pub struct PriorityFallback<T> {
    fallbacks: HashMap<String, Arc<dyn FallbackStrategy<T>>>,
    default: Option<Arc<dyn FallbackStrategy<T>>>,
}

impl<T> Default for PriorityFallback<T> {
    fn default() -> Self {
        Self::new()
    }
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

impl<T: Send + Sync + 'static> FallbackStrategy<T> for PriorityFallback<T> {
    fn fallback<'a>(
        &'a self,
        error: ResilienceError,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<T>> + Send + 'a>> {
        Box::pin(async move {
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
        })
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

impl FallbackStrategy<String> for AnyStringFallbackStrategy {
    fn fallback<'a>(
        &'a self,
        error: ResilienceError,
    ) -> Pin<Box<dyn Future<Output = ResilienceResult<String>> + Send + 'a>> {
        match self {
            Self::Value(strategy) => strategy.fallback(error),
            Self::Cache(strategy) => strategy.fallback(error),
            Self::Chain(strategy) => strategy.fallback(error),
            Self::Priority(strategy) => strategy.fallback(error),
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

    /// Execute with fallback.
    ///
    /// # Errors
    ///
    /// Returns the fallback strategy's error if both the operation and fallback fail,
    /// or the original error if the fallback strategy declines to handle it.
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
