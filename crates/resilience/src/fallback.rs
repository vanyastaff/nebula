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

use std::{fmt, future::Future, pin::Pin, sync::Arc};

use tokio::sync::RwLock;

use crate::{
    MetricsSink, NoopSink, PolicyContext, ResilienceEvent,
    error::{CallError, CallErrorKind},
};

// =============================================================================
// FALLBACK STRATEGY TRAIT
// =============================================================================

/// Fallback strategy trait, generic over both the value and error type.
///
/// Implement this trait to define custom fallback behavior.
pub trait FallbackStrategy<T, E>: Send + Sync {
    /// Execute recovery logic after [`fallback()`](Self::fallback) has accepted the error.
    ///
    /// Custom strategies normally implement this method and leave [`fallback()`](Self::fallback)
    /// alone. Calling `recover` directly intentionally bypasses
    /// [`should_fallback()`](Self::should_fallback); policy code should call `fallback` so
    /// cancellation and overload-style errors are not accidentally converted into successful
    /// graceful degradation.
    #[doc(hidden)]
    fn recover<'a>(
        &'a self,
        error: CallError<E>,
    ) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send + 'a>>;

    /// Execute fallback logic, returning either a recovered value or the original error.
    ///
    /// This is the safe entry point: it always checks [`should_fallback()`](Self::should_fallback)
    /// before invoking strategy-specific recovery.
    fn fallback<'a>(
        &'a self,
        error: CallError<E>,
    ) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send + 'a>>
    where
        E: Send + 'a,
    {
        if self.should_fallback(&error) {
            self.recover(error)
        } else {
            Box::pin(async move { Err(error) })
        }
    }

    /// Check if fallback should be attempted for this error.
    ///
    /// Default: attempt fallback only for primary operation failures and timeouts.
    ///
    /// Cancellation and overload-style policy rejections are not recovered by default,
    /// because treating them as successful fallback can hide shutdown and backpressure.
    fn should_fallback(&self, error: &CallError<E>) -> bool {
        matches!(
            error,
            CallError::Operation(_)
                | CallError::RetriesExhausted { .. }
                | CallError::Timeout(_)
                | CallError::CircuitOpen
        )
    }
}

/// Simple value fallback.
///
/// Returns a predetermined value when the primary operation fails.
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_resilience::{
///     CallError,
///     fallback::{FallbackStrategy, ValueFallback},
/// };
///
/// # #[tokio::main]
/// # async fn main() {
/// let fb = ValueFallback::new(42u32);
///
/// // On failure the fallback returns the configured value.
/// let recovered: Result<u32, CallError<&str>> = fb
///     .fallback(CallError::Timeout(Duration::from_secs(1)))
///     .await;
/// assert_eq!(recovered.unwrap(), 42);
/// # }
/// ```
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
    fn recover<'a>(
        &'a self,
        _error: CallError<E>,
    ) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send + 'a>> {
        let value = self.value.clone();
        Box::pin(async move { Ok(value) })
    }
}

/// Function fallback — executes a closure to produce a fallback value.
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_resilience::{
///     CallError,
///     fallback::{FallbackStrategy, FunctionFallback},
/// };
///
/// # #[tokio::main]
/// # async fn main() {
/// // The closure receives the original error (operation type erased) and
/// // returns either a recovered value or a `CallError`.
/// let fb = FunctionFallback::new(|_err: CallError<()>| async { Ok::<u32, CallError<()>>(7) });
///
/// let recovered: Result<u32, CallError<&str>> = fb
///     .fallback(CallError::Timeout(Duration::from_secs(1)))
///     .await;
/// assert_eq!(recovered.unwrap(), 7);
/// # }
/// ```
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
    E: Send + 'static,
    F: Fn(CallError<()>) -> Fut + Send + Sync,
    Fut: Future<Output = Result<T, CallError<()>>> + Send,
{
    /// Execute the fallback function.
    ///
    /// The original `Operation(E)` is erased to `Operation(())` before being passed
    /// to the closure — the closure cannot inspect the caller's error type. If the
    /// fallback closure fails, the returned error is wrapped with
    /// [`CallError::FallbackFailedWithContext`] so the primary error is preserved
    /// for telemetry and workflow decisions.
    fn recover<'a>(
        &'a self,
        error: CallError<E>,
    ) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send + 'a>> {
        let (erased, primary) = error.into_erased_for_fallback();
        Box::pin(async move {
            match (self.function)(erased).await {
                Ok(value) => Ok(value),
                Err(e) => {
                    let fallback = e.flat_map_inner(
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
                    );
                    Err(CallError::fallback_failed_with_context(primary, fallback))
                },
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
///
/// # Examples
///
/// ```rust
/// use std::time::Duration;
///
/// use nebula_resilience::{
///     CallError,
///     fallback::{CacheFallback, FallbackStrategy},
/// };
///
/// # #[tokio::main]
/// # async fn main() {
/// let fb: CacheFallback<String> = CacheFallback::new()
///     .with_ttl(Duration::from_secs(60))
///     .with_stale_if_error(true);
///
/// // Populate the cache after a successful primary call.
/// fb.update("last good response".into()).await;
///
/// // On a subsequent failure the cached value is returned.
/// let recovered: Result<String, CallError<&str>> = fb
///     .fallback(CallError::Timeout(Duration::from_secs(1)))
///     .await;
/// assert_eq!(recovered.unwrap(), "last good response");
/// # }
/// ```
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
    fn recover<'a>(
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
///
/// # Examples
///
/// ```rust
/// use std::{sync::Arc, time::Duration};
///
/// use nebula_resilience::{
///     CallError,
///     fallback::{CacheFallback, ChainFallback, FallbackStrategy, ValueFallback},
/// };
///
/// # #[tokio::main]
/// # async fn main() {
/// let cache: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(CacheFallback::new());
/// let default: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(ValueFallback::new(0u32));
///
/// // Try the cache first; if it has no value, fall back to a constant.
/// let chain = ChainFallback::new().then(cache).then(default);
///
/// let recovered = chain
///     .fallback(CallError::Timeout(Duration::from_secs(1)))
///     .await;
/// assert_eq!(recovered.unwrap(), 0);
/// # }
/// ```
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
    fn recover<'a>(
        &'a self,
        error: CallError<E>,
    ) -> Pin<Box<dyn Future<Output = Result<T, CallError<E>>> + Send + 'a>> {
        Box::pin(async move {
            let mut last_error = error;

            for fallback in &self.fallbacks {
                match fallback.fallback(last_error).await {
                    Ok(value) => return Ok(value),
                    Err(e) => last_error = e,
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
///
/// # Examples
///
/// ```rust
/// use std::{sync::Arc, time::Duration};
///
/// use nebula_resilience::{
///     CallError,
///     error::CallErrorKind,
///     fallback::{FallbackStrategy, PriorityFallback, ValueFallback},
/// };
///
/// # #[tokio::main]
/// # async fn main() {
/// let on_timeout: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(ValueFallback::new(1u32));
/// let on_other: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(ValueFallback::new(0u32));
///
/// let pf = PriorityFallback::new()
///     .register(CallErrorKind::Timeout, on_timeout)
///     .with_default(on_other);
///
/// let recovered = pf
///     .fallback(CallError::Timeout(Duration::from_secs(1)))
///     .await;
/// assert_eq!(recovered.unwrap(), 1);
/// # }
/// ```
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
    fn recover<'a>(
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
///
/// # Examples
///
/// ```rust
/// use std::{sync::Arc, time::Duration};
///
/// use nebula_resilience::{
///     CallError,
///     fallback::{FallbackOperation, ValueFallback},
/// };
///
/// # #[tokio::main]
/// # async fn main() {
/// let op: FallbackOperation<u32, &str> =
///     FallbackOperation::new(Arc::new(ValueFallback::new(99u32)));
///
/// // The primary operation fails, so the fallback value is returned.
/// let recovered = op
///     .call(|| async { Err::<u32, _>(CallError::Timeout(Duration::from_secs(1))) })
///     .await;
/// assert_eq!(recovered.unwrap(), 99);
/// # }
/// ```
pub struct FallbackOperation<T, E> {
    fallback_strategy: Arc<dyn FallbackStrategy<T, E>>,
    sink: Arc<dyn MetricsSink>,
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
        Self {
            fallback_strategy,
            sink: Arc::new(NoopSink),
        }
    }

    /// Attach a metrics/event sink for standalone fallback lifecycle events.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_sink(mut self, sink: impl MetricsSink + 'static) -> Self {
        self.sink = Arc::new(sink);
        self
    }

    /// Attach a shared metrics/event sink for standalone fallback lifecycle events.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_shared_sink(mut self, sink: Arc<dyn MetricsSink>) -> Self {
        self.sink = sink;
        self
    }

    fn record(&self, event: ResilienceEvent) {
        self.sink.record(event);
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
        E: Send,
    {
        match operation().await {
            Ok(value) => Ok(value),
            Err(error) => {
                if self.fallback_strategy.should_fallback(&error) {
                    let primary_error = error.kind();
                    self.record(ResilienceEvent::FallbackAttempted { primary_error });
                    match self.fallback_strategy.fallback(error).await {
                        Ok(value) => {
                            self.record(ResilienceEvent::FallbackSucceeded { primary_error });
                            Ok(value)
                        },
                        Err(error) => {
                            self.record(ResilienceEvent::FallbackFailed { primary_error });
                            Err(error)
                        },
                    }
                } else {
                    Err(error)
                }
            },
        }
    }

    /// Call with fallback under a shared policy context.
    ///
    /// Context cancellation/deadline bounds both the primary operation and the
    /// fallback future. Context cancellation and context deadline expiry are not
    /// recovered through fallback, preventing shutdown or action deadlines from
    /// being reported as successful graceful degradation.
    ///
    /// # Errors
    ///
    /// Returns `Err(CallError::Cancelled)` if the context is cancelled,
    /// `Err(CallError::Timeout)` if the context deadline expires, the fallback
    /// strategy's error if both primary and fallback fail, or the original error
    /// if fallback declines it.
    pub async fn call_with_policy_context<F, Fut>(
        &self,
        context: &PolicyContext,
        operation: F,
    ) -> Result<T, CallError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, CallError<E>>> + Send,
        T: Send + Sync,
        E: Send,
    {
        match context.run_result(operation()).await {
            Ok(value) => Ok(value),
            Err(error) => {
                if matches!(error, CallError::Cancelled { .. }) || context.is_cancelled() {
                    return Err(context.cancelled_error());
                }
                if matches!(error, CallError::Timeout(_)) && context.is_deadline_expired() {
                    return Err(error);
                }
                if self.fallback_strategy.should_fallback(&error) {
                    let primary_error = error.kind();
                    self.record(ResilienceEvent::FallbackAttempted { primary_error });
                    match context
                        .run_result(self.fallback_strategy.fallback(error))
                        .await
                    {
                        Ok(value) => {
                            self.record(ResilienceEvent::FallbackSucceeded { primary_error });
                            Ok(value)
                        },
                        Err(error) => {
                            self.record(ResilienceEvent::FallbackFailed { primary_error });
                            Err(error)
                        },
                    }
                } else {
                    Err(error)
                }
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::*;
    use crate::{
        CallError, CancellationContext, PolicyContext, RecordingSink, ResilienceEventKind,
    };

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

    #[test]
    fn value_fallback_declines_cancellation_and_overload_by_default() {
        let fb = ValueFallback::<u32>::new(0u32);
        let load_shed: CallError<&str> = CallError::LoadShed;
        let rate_limited: CallError<&str> = CallError::rate_limited();
        let bulkhead: CallError<&str> = CallError::BulkheadFull;

        assert!(!fb.should_fallback(&cancelled_error()));
        assert!(!fb.should_fallback(&load_shed));
        assert!(!fb.should_fallback(&rate_limited));
        assert!(!fb.should_fallback(&bulkhead));
    }

    #[tokio::test]
    async fn value_fallback_direct_call_declines_cancellation() {
        let fb = ValueFallback::<u32>::new(0u32);
        let result = fb.fallback(cancelled_error()).await;
        assert!(matches!(result, Err(CallError::Cancelled { .. })));
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
        let first: Arc<dyn FallbackStrategy<u32, &str>> = Arc::new(CacheFallback::new());
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
        assert!(matches!(
            result,
            Err(CallError::FallbackFailedWithContext { .. })
        ));
    }

    #[tokio::test]
    async fn function_fallback_failure_preserves_primary_context() {
        let fallback = FunctionFallback::new(|_err: CallError<()>| async {
            Err::<u32, _>(CallError::cancelled_with("fallback unavailable"))
        });

        let result = fallback
            .fallback(CallError::Operation("primary failed"))
            .await;
        let err = result.unwrap_err();
        let (primary, fallback) = err.fallback_context().unwrap();

        assert!(matches!(primary, CallError::Operation("primary failed")));
        assert!(matches!(fallback, CallError::Cancelled { .. }));
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
        // Fallbackable error without a specific handler → default
        assert_eq!(pf.fallback(CallError::Operation("boom")).await.unwrap(), 0);
        // Cancellation is not recovered by the default fallback.
        assert!(matches!(
            pf.fallback(cancelled_error()).await,
            Err(CallError::Cancelled { .. })
        ));
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

    #[tokio::test]
    async fn fallback_operation_emits_standalone_lifecycle_events() {
        let sink = RecordingSink::new();
        let op: FallbackOperation<u32, &str> =
            FallbackOperation::new(Arc::new(ValueFallback::new(99u32))).with_sink(sink.clone());

        let result = op.call(|| async { Err::<u32, _>(timeout_error()) }).await;

        assert_eq!(result.unwrap(), 99);
        assert_eq!(sink.count(ResilienceEventKind::FallbackAttempted), 1);
        assert_eq!(sink.count(ResilienceEventKind::FallbackSucceeded), 1);
        assert_eq!(sink.count(ResilienceEventKind::FallbackFailed), 0);
    }

    #[tokio::test]
    async fn fallback_operation_emits_failure_event_on_fallback_failure() {
        let sink = RecordingSink::new();
        let fallback = FunctionFallback::new(|_err: CallError<()>| async {
            Err::<u32, _>(CallError::fallback_failed_with("cache unavailable"))
        });
        let op: FallbackOperation<u32, &str> =
            FallbackOperation::new(Arc::new(fallback)).with_sink(sink.clone());

        let result = op.call(|| async { Err::<u32, _>(timeout_error()) }).await;

        assert!(matches!(
            result,
            Err(CallError::FallbackFailedWithContext { .. })
        ));
        assert_eq!(sink.count(ResilienceEventKind::FallbackAttempted), 1);
        assert_eq!(sink.count(ResilienceEventKind::FallbackSucceeded), 0);
        assert_eq!(sink.count(ResilienceEventKind::FallbackFailed), 1);
    }

    #[tokio::test]
    async fn fallback_operation_context_cancellation_skips_fallback() {
        let op: FallbackOperation<u32, &str> =
            FallbackOperation::new(Arc::new(ValueFallback::new(99u32)));
        let cancellation = CancellationContext::with_reason("shutdown");
        let context = PolicyContext::from_cancellation(cancellation.clone());
        cancellation.cancel();

        let result = op
            .call_with_policy_context(&context, || async { Ok::<u32, CallError<&str>>(42) })
            .await;

        assert!(matches!(result, Err(CallError::Cancelled { .. })));
    }

    #[tokio::test]
    async fn fallback_operation_context_deadline_bounds_fallback() {
        let fallback = FunctionFallback::new(|_err: CallError<()>| async {
            tokio::time::sleep(Duration::from_mins(1)).await;
            Ok::<u32, CallError<()>>(99)
        });
        let op: FallbackOperation<u32, &str> = FallbackOperation::new(Arc::new(fallback));
        let context = PolicyContext::with_timeout(Duration::from_millis(1));

        let result = op
            .call_with_policy_context(&context, || async { Err::<u32, _>(timeout_error()) })
            .await;

        assert!(matches!(result, Err(CallError::Timeout(_))));
    }
}
