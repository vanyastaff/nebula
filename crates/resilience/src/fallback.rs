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
    /// Produce a recovery value for an error that
    /// [`should_fallback()`](Self::should_fallback) accepted.
    ///
    /// This is the trait's required method: a custom strategy implements
    /// `recover` and normally leaves [`fallback()`](Self::fallback) alone.
    /// Policy code should call [`fallback()`](Self::fallback), not `recover`
    /// directly — `fallback` is what gates recovery on `should_fallback`, so
    /// calling `recover` bypasses the decision that keeps cancellation and
    /// backpressure rejections from being reported as graceful degradation.
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

// =============================================================================
// SHARED ORCHESTRATION
// =============================================================================

/// Outcome of attempting a fallback after a primary failure.
///
/// Produced by [`orchestrate_fallback`]; the caller translates it into its own
/// completion event (`PipelineOutcome` for the pipeline, the bare event pair
/// for [`FallbackExecutor`]).
pub(crate) enum FallbackOutcome<T, E> {
    /// The strategy was not asked: it declined the error.
    Declined(CallError<E>),
    /// The strategy recovered a value.
    Recovered(T),
    /// The strategy was asked and failed.
    Failed(CallError<E>),
}

/// The one fallback orchestration: decide, emit `FallbackAttempted`, recover,
/// emit the result event.
///
/// `FallbackExecutor` and `ResiliencePipeline::call_with_fallback*` both drive
/// fallback through this function rather than keeping parallel copies of the
/// sequence, because two copies let the event contract drift between the
/// standalone and pipeline entry points.
///
/// Cancellation and context-deadline errors are never offered to the strategy:
/// shutdown and action deadlines must not be reported as successful graceful
/// degradation. The caller keeps responsibility for the stronger check that
/// requires its own cancellation handle (the pipeline's `select!` against a
/// live token).
pub(crate) async fn orchestrate_fallback<T, E>(
    strategy: &dyn FallbackStrategy<T, E>,
    sink: &dyn MetricsSink,
    error: CallError<E>,
) -> FallbackOutcome<T, E>
where
    T: Send + Sync,
    E: Send,
{
    if matches!(error, CallError::Cancelled { .. }) || !strategy.should_fallback(&error) {
        return FallbackOutcome::Declined(error);
    }

    let primary_error = error.kind();
    sink.record(ResilienceEvent::FallbackAttempted { primary_error });
    match strategy.recover(error).await {
        Ok(value) => {
            sink.record(ResilienceEvent::FallbackSucceeded { primary_error });
            FallbackOutcome::Recovered(value)
        },
        Err(fallback_error) => {
            sink.record(ResilienceEvent::FallbackFailed {
                primary_error,
                fallback_error: fallback_error.kind(),
            });
            FallbackOutcome::Failed(fallback_error)
        },
    }
}

/// Runs an operation and, on eligible failure, a [`FallbackStrategy`].
///
/// The executor shape matches the crate's other drivers ([`HedgeExecutor`],
/// [`TimeoutExecutor`](crate::TimeoutExecutor)): a configured object whose
/// `call` wraps a caller-supplied operation, rather than a free function that
/// needs the strategy threaded through every call site.
///
/// # Examples
///
/// ```rust
/// use std::{sync::Arc, time::Duration};
///
/// use nebula_resilience::{
///     CallError,
///     fallback::{FallbackExecutor, ValueFallback},
/// };
///
/// # #[tokio::main]
/// # async fn main() {
/// let op: FallbackExecutor<u32, &str> =
///     FallbackExecutor::new(Arc::new(ValueFallback::new(99u32)));
///
/// // The primary operation fails, so the fallback value is returned.
/// let recovered = op
///     .call(|| async { Err::<u32, _>(CallError::Timeout(Duration::from_secs(1))) })
///     .await;
/// assert_eq!(recovered.unwrap(), 99);
/// # }
/// ```
pub struct FallbackExecutor<T, E> {
    fallback_strategy: Arc<dyn FallbackStrategy<T, E>>,
    sink: Arc<dyn MetricsSink>,
}

impl<T, E> FallbackExecutor<T, E> {
    /// Delegate one primary failure to the strategy through the shared
    /// orchestration and translate its outcome.
    async fn apply(&self, error: CallError<E>) -> Result<T, CallError<E>>
    where
        T: Send + Sync,
        E: Send,
    {
        match orchestrate_fallback(self.fallback_strategy.as_ref(), self.sink.as_ref(), error).await
        {
            FallbackOutcome::Declined(error) | FallbackOutcome::Failed(error) => Err(error),
            FallbackOutcome::Recovered(value) => Ok(value),
        }
    }
}

impl<T, E> fmt::Debug for FallbackExecutor<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FallbackExecutor").finish_non_exhaustive()
    }
}

impl<T, E> FallbackExecutor<T, E> {
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
            Err(error) => self.apply(error).await,
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
                if context.is_cancelled() {
                    return Err(context.cancelled_error());
                }
                if matches!(error, CallError::Cancelled { .. }) {
                    return Err(error);
                }
                if matches!(error, CallError::Timeout(_)) && context.is_deadline_expired() {
                    return Err(error);
                }
                // The fallback phase stays bounded by the same context as the
                // primary: a deadline that expires during recovery turns into
                // `Timeout` rather than waiting for a slow strategy.
                context.run_result(self.apply(error)).await
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "fallback_tests.rs"]
mod tests;
