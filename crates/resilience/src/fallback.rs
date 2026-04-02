//! Fallback strategies for graceful degradation.
//!
//! Provides fallback mechanisms to maintain service availability when primary operations fail.
//! All strategies operate on [`CallError<E>`] — the same error type used by every other pattern.
//!
//! # Example
//!
//! ```rust
//! use nebula_resilience::fallback::ValueFallback;
//!
//! // Return a default value on failure
//! let fallback = ValueFallback::new("default response".to_string());
//! ```

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::{CallError, CallErrorKind};

// =============================================================================
// FALLBACK STRATEGY TRAIT
// =============================================================================

/// Fallback strategy trait, generic over both the value and error type.
///
/// Implement this trait to define custom fallback behavior.
pub trait FallbackStrategy<T, E>: Send + Sync {
    /// Execute fallback logic, returning either a recovered value or the error.
    fn fallback<'a>(
        &'a self,
        error: CallError<E>,
    ) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send + 'a>>;

    /// Check if fallback should be attempted for this error.
    ///
    /// Default: attempt fallback for all errors.
    fn should_fallback(&self, _error: &CallError<E>) -> bool {
        true
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

impl<T: Clone + Send + Sync, E> FallbackStrategy<T, E> for ValueFallback<T> {
    fn fallback<'a>(
        &'a self,
        _error: CallError<E>,
    ) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send + 'a>> {
        let value = self.value.clone();
        Box::pin(async move { Ok(value) })
    }
}

/// Function fallback — executes a closure to produce a fallback value.
pub struct FunctionFallback<T, F, Fut>
where
    F: Fn(CallError<()>) -> Fut + Send + Sync,
    Fut: Future<Output = Result<T, CallError<()>>> + Send,
{
    function: F,
    _phantom: std::marker::PhantomData<T>,
}

impl<T, F, Fut> fmt::Debug for FunctionFallback<T, F, Fut>
where
    F: Fn(CallError<()>) -> Fut + Send + Sync,
    Fut: Future<Output = Result<T, CallError<()>>> + Send,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FunctionFallback").finish_non_exhaustive()
    }
}

impl<T, F, Fut> FunctionFallback<T, F, Fut>
where
    F: Fn(CallError<()>) -> Fut + Send + Sync,
    Fut: Future<Output = Result<T, CallError<()>>> + Send,
{
    /// Create new function fallback.
    #[must_use]
    pub const fn new(function: F) -> Self {
        Self {
            function,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T, E, F, Fut> FallbackStrategy<T, E> for FunctionFallback<T, F, Fut>
where
    T: Send + Sync + 'static,
    F: Fn(CallError<()>) -> Fut + Send + Sync,
    Fut: Future<Output = Result<T, CallError<()>>> + Send,
{
    /// Execute the fallback function.
    ///
    /// The original `Operation(E)` is erased to `Operation(())` before being passed
    /// to the closure — the closure cannot inspect the caller's error type. If the
    /// closure returns `Err(CallError::Operation(()))`, it is converted to
    /// `Err(CallError::Cancelled)` since the original `E` cannot be reconstructed.
    fn fallback<'a>(
        &'a self,
        error: CallError<E>,
    ) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send + 'a>> {
        let erased = error.map_operation(|_| ());
        Box::pin(async move {
            match (self.function)(erased).await {
                Ok(value) => Ok(value),
                Err(e) => Err(e.flat_map_inner(
                    |()| {
                        CallError::fallback_failed_with(
                            "fallback returned Operation(()) — original error was erased",
                        )
                    },
                    |_, ()| {
                        CallError::fallback_failed_with(
                            "fallback returned RetriesExhausted(()) — original error was erased",
                        )
                    },
                )),
            }
        })
    }
}

/// A cached value together with the instant it was stored.
struct CacheEntry<T> {
    value: T,
    updated_at: std::time::Instant,
}

/// Cache fallback — returns a previously cached value on error.
pub struct CacheFallback<T: Clone + Send + Sync> {
    cache: Arc<RwLock<Option<CacheEntry<T>>>>,
    ttl: Option<std::time::Duration>,
    stale_if_error: bool,
}

impl<T: Clone + Send + Sync> fmt::Debug for CacheFallback<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CacheFallback")
            .field("ttl", &self.ttl)
            .field("stale_if_error", &self.stale_if_error)
            .finish_non_exhaustive()
    }
}

impl<T: Clone + Send + Sync> Default for CacheFallback<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone + Send + Sync> CacheFallback<T> {
    /// Create new cache fallback.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(None)),
            ttl: None,
            stale_if_error: false,
        }
    }

    /// Set TTL for cached value.
    #[must_use = "builder methods must be chained or built"]
    pub const fn with_ttl(mut self, ttl: std::time::Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }

    /// Allow serving stale cached value when TTL is exceeded.
    ///
    /// When enabled, expired cache entries can still be returned during fallback
    /// instead of propagating the original error.
    #[must_use = "builder methods must be chained or built"]
    pub const fn with_stale_if_error(mut self, enabled: bool) -> Self {
        self.stale_if_error = enabled;
        self
    }

    /// Update cached value.
    pub async fn update(&self, value: T) {
        *self.cache.write().await = Some(CacheEntry {
            value,
            updated_at: std::time::Instant::now(),
        });
    }
}

impl<T: Clone + Send + Sync + 'static, E: Send + 'static> FallbackStrategy<T, E>
    for CacheFallback<T>
{
    fn fallback<'a>(
        &'a self,
        error: CallError<E>,
    ) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send + 'a>> {
        Box::pin(async move {
            let guard = self.cache.read().await;
            let Some(entry) = guard.as_ref() else {
                drop(guard);
                return Err(error);
            };
            let expired = self
                .ttl
                .is_some_and(|ttl| entry.updated_at.elapsed() >= ttl);
            if expired && !self.stale_if_error {
                drop(guard);
                Err(error)
            } else {
                let value = entry.value.clone();
                drop(guard);
                Ok(value)
            }
        })
    }
}

/// Chain fallback — tries multiple fallbacks in sequence.
///
/// Each strategy's [`should_fallback()`](FallbackStrategy::should_fallback) is checked
/// before calling [`fallback()`](FallbackStrategy::fallback). If a strategy declines
/// (returns `false`), the **same error** is passed unchanged to the next strategy in the
/// chain — the declining strategy does not get to modify or wrap the error.
pub struct ChainFallback<T, E> {
    fallbacks: Vec<Arc<dyn FallbackStrategy<T, E>>>,
}

impl<T, E> fmt::Debug for ChainFallback<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChainFallback")
            .field("count", &self.fallbacks.len())
            .finish_non_exhaustive()
    }
}

impl<T, E> Default for ChainFallback<T, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, E> ChainFallback<T, E> {
    /// Create new chain fallback.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fallbacks: Vec::new(),
        }
    }

    /// Append a fallback to the chain.
    #[must_use = "builder methods must be chained or built"]
    pub fn then(mut self, fallback: Arc<dyn FallbackStrategy<T, E>>) -> Self {
        self.fallbacks.push(fallback);
        self
    }
}

impl<T: Send + Sync + 'static, E: Send + 'static> FallbackStrategy<T, E> for ChainFallback<T, E> {
    fn fallback<'a>(
        &'a self,
        error: CallError<E>,
    ) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send + 'a>> {
        Box::pin(async move {
            let mut last_error = error;

            for fallback in &self.fallbacks {
                if fallback.should_fallback(&last_error) {
                    match fallback.fallback(last_error).await {
                        Ok(value) => return Ok(value),
                        Err(e) => last_error = e,
                    }
                }
            }

            Err(last_error)
        })
    }
}

/// Priority fallback — selects fallback based on error kind.
///
/// Uses a `Vec` internally — `CallErrorKind` has few variants, so linear
/// scan is faster than `HashMap` and avoids hashing overhead.
pub struct PriorityFallback<T, E> {
    fallbacks: Vec<(CallErrorKind, Arc<dyn FallbackStrategy<T, E>>)>,
    default: Option<Arc<dyn FallbackStrategy<T, E>>>,
}

impl<T, E> fmt::Debug for PriorityFallback<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PriorityFallback")
            .field("registered_kinds", &self.fallbacks.len())
            .field("has_default", &self.default.is_some())
            .finish_non_exhaustive()
    }
}

impl<T, E> Default for PriorityFallback<T, E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, E> PriorityFallback<T, E> {
    /// Create new priority fallback.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fallbacks: Vec::new(),
            default: None,
        }
    }

    /// Register a fallback for a specific error kind.
    ///
    /// If a fallback is already registered for this kind, it is replaced.
    #[must_use = "builder methods must be chained or built"]
    pub fn register(
        mut self,
        kind: CallErrorKind,
        fallback: Arc<dyn FallbackStrategy<T, E>>,
    ) -> Self {
        if let Some(existing) = self.fallbacks.iter_mut().find(|(k, _)| *k == kind) {
            existing.1 = fallback;
        } else {
            self.fallbacks.push((kind, fallback));
        }
        self
    }

    /// Set default fallback.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_default(mut self, fallback: Arc<dyn FallbackStrategy<T, E>>) -> Self {
        self.default = Some(fallback);
        self
    }
}

impl<T: Send + Sync + 'static, E: Send + 'static> FallbackStrategy<T, E>
    for PriorityFallback<T, E>
{
    fn fallback<'a>(
        &'a self,
        error: CallError<E>,
    ) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send + 'a>> {
        Box::pin(async move {
            let kind = error.kind();

            if let Some((_, fallback)) = self.fallbacks.iter().find(|(k, _)| *k == kind) {
                return fallback.fallback(error).await;
            }

            if let Some(default) = &self.default {
                return default.fallback(error).await;
            }

            Err(error)
        })
    }
}

/// Fallback with operation — combines primary and fallback operations.
pub struct FallbackOperation<T, E> {
    fallback_strategy: Arc<dyn FallbackStrategy<T, E>>,
}

impl<T, E> fmt::Debug for FallbackOperation<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FallbackOperation").finish_non_exhaustive()
    }
}

impl<T, E> FallbackOperation<T, E> {
    /// Create new fallback operation.
    #[must_use]
    pub fn new(fallback_strategy: Arc<dyn FallbackStrategy<T, E>>) -> Self {
        Self { fallback_strategy }
    }

    /// Call with fallback.
    ///
    /// # Errors
    ///
    /// Returns the fallback strategy's error if both the operation and fallback fail,
    /// or the original error if the fallback strategy declines to handle it.
    pub async fn call<F, Fut>(&self, operation: F) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, CallError<E>>>,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::CallError;

    fn timeout_error() -> CallError<&'static str> {
        CallError::Timeout(Duration::from_secs(1))
    }

    fn cancelled_error() -> CallError<&'static str> {
        CallError::cancelled()
    }

    // -----------------------------------------------------------------------
    // ValueFallback
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn value_fallback_returns_configured_value() {
        let fb = ValueFallback::new(42u32);
        let result = fb.fallback(timeout_error()).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn value_fallback_should_fallback_true_for_timeout() {
        let fb = ValueFallback::<u32>::new(0u32);
        assert!(fb.should_fallback(&timeout_error()));
    }

    // -----------------------------------------------------------------------
    // CacheFallback
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn cache_fallback_returns_error_when_empty() {
        let fb: CacheFallback<String> = CacheFallback::new();
        let result: Result<String, CallError<&str>> = fb
            .fallback(CallError::Timeout(Duration::from_secs(1)))
            .await;
        assert!(matches!(result, Err(CallError::Timeout(_))));
    }

    #[tokio::test]
    async fn cache_fallback_returns_cached_value() {
        let fb = CacheFallback::new();
        fb.update("hello".to_string()).await;
        let result: Result<String, CallError<&str>> = fb.fallback(timeout_error()).await;
        assert_eq!(result.unwrap(), "hello");
    }

    #[tokio::test]
    async fn cache_fallback_expires_when_ttl_exceeded() {
        let fb = CacheFallback::new().with_ttl(Duration::from_millis(1));
        fb.update("stale".to_string()).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        let result: Result<String, CallError<&str>> = fb.fallback(timeout_error()).await;
        assert!(matches!(result, Err(CallError::Timeout(_))));
    }

    #[tokio::test]
    async fn cache_fallback_stale_if_error_serves_expired_value() {
        let fb = CacheFallback::new()
            .with_ttl(Duration::from_millis(1))
            .with_stale_if_error(true);
        fb.update("stale".to_string()).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        let result: Result<String, CallError<&str>> = fb.fallback(timeout_error()).await;
        assert_eq!(result.unwrap(), "stale");
    }

    // -----------------------------------------------------------------------
    // ChainFallback
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn chain_fallback_tries_in_order_and_returns_first_success() {
        let first: Arc<dyn FallbackStrategy<u32, &str>> =
            Arc::new(FunctionFallback::new(|_err| async {
                Err(CallError::cancelled())
            }));
        let second: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(ValueFallback::new(99u32));

        let chain = ChainFallback::new().then(first).then(second);
        let result = chain.fallback(timeout_error()).await;
        assert_eq!(result.unwrap(), 99);
    }

    #[tokio::test]
    async fn chain_fallback_returns_last_error_when_all_fail() {
        let failing: Arc<dyn FallbackStrategy<u32, &str>> =
            Arc::new(FunctionFallback::new(|_err| async {
                Err(CallError::cancelled_with("fail"))
            }));
        let chain = ChainFallback::new()
            .then(Arc::clone(&failing))
            .then(Arc::clone(&failing));
        let result = chain.fallback(timeout_error()).await;
        assert!(matches!(result, Err(CallError::Cancelled { .. })));
    }

    // -----------------------------------------------------------------------
    // PriorityFallback / CallErrorKind
    // -----------------------------------------------------------------------

    #[test]
    fn error_kind_from_timeout() {
        assert_eq!(timeout_error().kind(), CallErrorKind::Timeout);
    }

    #[test]
    fn error_kind_from_cancelled() {
        assert_eq!(cancelled_error().kind(), CallErrorKind::Cancelled);
    }

    #[tokio::test]
    async fn priority_fallback_dispatches_to_matching_kind() {
        let timeout_fb: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(ValueFallback::new(1u32));
        let default_fb: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(ValueFallback::new(0u32));

        let pf = PriorityFallback::new()
            .register(CallErrorKind::Timeout, timeout_fb)
            .with_default(default_fb);

        // Timeout → registered handler
        assert_eq!(pf.fallback(timeout_error()).await.unwrap(), 1);
        // Other error → default
        assert_eq!(pf.fallback(cancelled_error()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn priority_fallback_returns_error_when_no_match_and_no_default() {
        let pf: PriorityFallback<u32, &str> = PriorityFallback::new();
        let result = pf.fallback(timeout_error()).await;
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // FallbackOperation
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn fallback_operation_returns_primary_result_on_success() {
        let op: FallbackOperation<u32, &str> =
            FallbackOperation::new(Arc::new(ValueFallback::new(0u32)));
        let result = op.call(|| async { Ok(42u32) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn fallback_operation_invokes_fallback_on_error() {
        let op: FallbackOperation<u32, &str> =
            FallbackOperation::new(Arc::new(ValueFallback::new(99u32)));
        let result = op.call(|| async { Err::<u32, _>(timeout_error()) }).await;
        assert_eq!(result.unwrap(), 99);
    }
}
