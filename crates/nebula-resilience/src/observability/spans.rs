//! Distributed tracing spans for resilience patterns
//!
//! This module provides helpers for creating OpenTelemetry-compatible
//! tracing spans for resilience pattern operations.
//!
//! # Type-Safe Spans
//!
//! The module includes typed span guards with compile-time pattern validation:
//!
//! ```rust,ignore
//! use nebula_resilience::observability::{SpanGuard, RetryPattern};
//!
//! // Type-safe span with compile-time pattern category
//! let span = PatternSpanGuard::<RetryPattern>::new("api_call");
//! // ... perform operation
//! span.success(); // or span.failure(&error)
//! ```

use nebula_log::{debug, error, info, warn};
use std::future::Future;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

// =============================================================================
// SEALED PATTERN CATEGORY TRAIT
// =============================================================================

mod sealed {
    pub trait SealedPatternCategory {}
}

/// Pattern category marker trait for typed spans.
pub trait PatternCategory: sealed::SealedPatternCategory + Send + Sync + 'static {
    /// Pattern name for span naming.
    fn name() -> &'static str;

    /// Pattern description.
    fn description() -> &'static str;

    /// Whether to log start events.
    #[must_use] 
    fn log_start() -> bool {
        true
    }

    /// Whether to log success events.
    #[must_use] 
    fn log_success() -> bool {
        true
    }
}

// =============================================================================
// PATTERN CATEGORY MARKERS
// =============================================================================

/// Retry pattern category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPattern;

impl sealed::SealedPatternCategory for RetryPattern {}

impl PatternCategory for RetryPattern {
    fn name() -> &'static str {
        "retry"
    }

    fn description() -> &'static str {
        "Retry pattern span"
    }
}

/// Circuit breaker pattern category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitBreakerPattern;

impl sealed::SealedPatternCategory for CircuitBreakerPattern {}

impl PatternCategory for CircuitBreakerPattern {
    fn name() -> &'static str {
        "circuit_breaker"
    }

    fn description() -> &'static str {
        "Circuit breaker pattern span"
    }
}

/// Timeout pattern category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutPattern;

impl sealed::SealedPatternCategory for TimeoutPattern {}

impl PatternCategory for TimeoutPattern {
    fn name() -> &'static str {
        "timeout"
    }

    fn description() -> &'static str {
        "Timeout pattern span"
    }
}

/// Bulkhead pattern category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulkheadPattern;

impl sealed::SealedPatternCategory for BulkheadPattern {}

impl PatternCategory for BulkheadPattern {
    fn name() -> &'static str {
        "bulkhead"
    }

    fn description() -> &'static str {
        "Bulkhead pattern span"
    }
}

/// Rate limiter pattern category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimiterPattern;

impl sealed::SealedPatternCategory for RateLimiterPattern {}

impl PatternCategory for RateLimiterPattern {
    fn name() -> &'static str {
        "rate_limiter"
    }

    fn description() -> &'static str {
        "Rate limiter pattern span"
    }

    fn log_start() -> bool {
        false // Rate limiter spans can be high-volume
    }
}

/// Fallback pattern category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FallbackPattern;

impl sealed::SealedPatternCategory for FallbackPattern {}

impl PatternCategory for FallbackPattern {
    fn name() -> &'static str {
        "fallback"
    }

    fn description() -> &'static str {
        "Fallback pattern span"
    }
}

// =============================================================================
// TYPED SPAN GUARD
// =============================================================================

/// span guard with compile-time pattern category.
///
/// Provides RAII-style span management with automatic logging.
pub struct PatternSpanGuard<P: PatternCategory> {
    operation: String,
    start: Instant,
    completed: bool,
    _pattern: PhantomData<P>,
}

impl<P: PatternCategory> PatternSpanGuard<P> {
    /// Create a new typed span guard.
    #[must_use]
    pub fn new(operation: impl Into<String>) -> Self {
        let operation = operation.into();
        let pattern = P::name();

        if P::log_start() {
            info!("Starting {pattern} for {operation}");
        }

        Self {
            operation,
            start: Instant::now(),
            completed: false,
            _pattern: PhantomData,
        }
    }

    /// Get the pattern name.
    #[must_use] 
    pub fn pattern(&self) -> &'static str {
        P::name()
    }

    /// Get the operation name.
    #[must_use] 
    pub fn operation(&self) -> &str {
        &self.operation
    }

    /// Get elapsed time.
    #[must_use] 
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Record success and consume the guard.
    pub fn success(mut self) {
        self.completed = true;
        let duration = self.start.elapsed();
        let pattern = P::name();

        if P::log_success() {
            info!(
                "{} succeeded for {} in {:?}",
                pattern, self.operation, duration
            );
        }
    }

    /// Record success with a result value.
    pub fn success_with<T>(self, result: T) -> T {
        self.success();
        result
    }

    /// Record failure and consume the guard.
    pub fn failure(mut self, error: &impl std::fmt::Display) {
        self.completed = true;
        let duration = self.start.elapsed();
        let pattern = P::name();

        error!(
            "{} failed for {} after {:?}: {}",
            pattern, self.operation, duration, error
        );
    }

    /// Record failure and return an error.
    pub fn failure_with<E: std::fmt::Display>(self, error: E) -> E {
        self.failure(&error);
        error
    }
}

impl<P: PatternCategory> Drop for PatternSpanGuard<P> {
    fn drop(&mut self) {
        if !self.completed && !std::thread::panicking() {
            let duration = self.start.elapsed();
            let pattern = P::name();

            debug!(
                "{} completed for {} in {:?} (no explicit outcome)",
                pattern, self.operation, duration
            );
        }
    }
}

// =============================================================================
// TYPED SPAN ASYNC HELPER
// =============================================================================

/// Execute an async operation with a typed span.
pub async fn with_typed_span<P, F, Fut, T, E>(operation: &str, f: F) -> Result<T, E>
where
    P: PatternCategory,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let span = PatternSpanGuard::<P>::new(operation);

    match f().await {
        Ok(result) => {
            span.success();
            Ok(result)
        }
        Err(e) => {
            span.failure(&e);
            Err(e)
        }
    }
}

// =============================================================================
// ORIGINAL IMPLEMENTATION
// =============================================================================

/// Create a new span for a resilience operation
///
/// This is a lightweight wrapper that can be extended with OpenTelemetry
/// when the `telemetry` feature is enabled in nebula-log.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_resilience::observability::create_span;
///
/// async fn my_operation() -> Result<(), Error> {
///     create_span("retry", "api_call", || async {
///         // Your operation here
///         Ok(())
///     }).await
/// }
/// ```
pub async fn create_span<F, Fut, T, E>(pattern: &str, operation: &str, f: F) -> Result<T, E>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let start = Instant::now();
    info!("Starting {pattern} for {operation}");

    match f().await {
        Ok(result) => {
            let duration = start.elapsed();
            info!("{pattern} succeeded for {operation} in {duration:?}");
            Ok(result)
        }
        Err(e) => {
            let duration = start.elapsed();
            error!("{pattern} failed for {operation} after {duration:?}: {e}");
            Err(e)
        }
    }
}

/// Record a successful operation
pub fn record_success(pattern: &str, operation: &str, duration: std::time::Duration) {
    debug!("{pattern} success for {operation} in {duration:?}");
}

/// Record a failed operation
pub fn record_error(pattern: &str, operation: &str, error: &impl std::fmt::Display) {
    warn!("{pattern} error for {operation}: {error}");
}

/// Span guard that automatically logs completion
pub struct SpanGuard {
    pattern: String,
    operation: String,
    start: Instant,
}

impl SpanGuard {
    /// Create a new span guard
    #[must_use]
    pub fn new(pattern: impl Into<String>, operation: impl Into<String>) -> Self {
        let pattern = pattern.into();
        let operation = operation.into();
        info!("Starting {pattern} for {operation}");

        Self {
            pattern,
            operation,
            start: Instant::now(),
        }
    }

    /// Record success and consume the guard
    pub fn success(self) {
        let duration = self.start.elapsed();
        info!(
            "{} succeeded for {} in {:?}",
            self.pattern, self.operation, duration
        );
    }

    /// Record failure and consume the guard
    pub fn failure(self, error: &impl std::fmt::Display) {
        let duration = self.start.elapsed();
        error!(
            "{} failed for {} after {:?}: {}",
            self.pattern, self.operation, duration, error
        );
    }
}

impl Drop for SpanGuard {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let duration = self.start.elapsed();
            debug!(
                "{} completed for {} in {:?}",
                self.pattern, self.operation, duration
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_span_success() {
        let result = create_span("test", "operation", || async { Ok::<_, &str>(42) }).await;

        assert_eq!(result, Ok(42));
    }

    #[tokio::test]
    async fn test_create_span_failure() {
        let result = create_span("test", "operation", || async {
            Err::<i32, _>("test error")
        })
        .await;

        assert_eq!(result, Err("test error"));
    }

    #[test]
    fn test_span_guard() {
        let guard = SpanGuard::new("test", "operation");
        guard.success();
    }

    #[test]
    fn test_span_guard_failure() {
        let guard = SpanGuard::new("test", "operation");
        guard.failure(&"test error");
    }

    // =========================================================================
    // TYPED SPAN TESTS
    // =========================================================================

    #[test]
    fn test_pattern_categories() {
        assert_eq!(RetryPattern::name(), "retry");
        assert!(RetryPattern::log_start());
        assert!(RetryPattern::log_success());

        assert_eq!(CircuitBreakerPattern::name(), "circuit_breaker");
        assert_eq!(TimeoutPattern::name(), "timeout");
        assert_eq!(BulkheadPattern::name(), "bulkhead");

        assert_eq!(RateLimiterPattern::name(), "rate_limiter");
        assert!(!RateLimiterPattern::log_start()); // High-volume, no start logs

        assert_eq!(FallbackPattern::name(), "fallback");
    }

    #[test]
    fn test_typed_span_guard_success() {
        let span = PatternSpanGuard::<RetryPattern>::new("test_operation");

        assert_eq!(span.pattern(), "retry");
        assert_eq!(span.operation(), "test_operation");
        assert!(span.elapsed() >= Duration::ZERO);

        span.success();
    }

    #[test]
    fn test_typed_span_guard_failure() {
        let span = PatternSpanGuard::<CircuitBreakerPattern>::new("db_query");

        assert_eq!(span.pattern(), "circuit_breaker");

        span.failure(&"Connection refused");
    }

    #[test]
    fn test_typed_span_guard_success_with() {
        let span = PatternSpanGuard::<TimeoutPattern>::new("api_call");
        let result = span.success_with(42);
        assert_eq!(result, 42);
    }

    #[test]
    fn test_typed_span_guard_failure_with() {
        let span = PatternSpanGuard::<BulkheadPattern>::new("task");
        let error = span.failure_with("capacity exceeded");
        assert_eq!(error, "capacity exceeded");
    }

    #[test]
    fn test_typed_span_guard_drop_without_completion() {
        // This should log a debug message on drop
        let _span = PatternSpanGuard::<FallbackPattern>::new("test");
        // Span drops here without explicit success/failure
    }

    #[tokio::test]
    async fn test_with_typed_span_success() {
        let result = with_typed_span::<RetryPattern, _, _, _, &str>("api_call", || async {
            Ok::<i32, &str>(42)
        })
        .await;

        assert_eq!(result, Ok(42));
    }

    #[tokio::test]
    async fn test_with_typed_span_failure() {
        let result = with_typed_span::<TimeoutPattern, _, _, _, &str>("slow_call", || async {
            Err::<i32, &str>("timeout")
        })
        .await;

        assert_eq!(result, Err("timeout"));
    }
}
