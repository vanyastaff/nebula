//! Distributed tracing spans for resilience patterns
//!
//! This module provides helpers for creating OpenTelemetry-compatible
//! tracing spans for resilience pattern operations.

use nebula_log::{debug, error, info, warn};
use std::future::Future;
use std::time::Instant;

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
pub async fn create_span<F, Fut, T, E>(
    pattern: &str,
    operation: &str,
    f: F,
) -> Result<T, E>
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
        let result = create_span("test", "operation", || async {
            Ok::<_, &str>(42)
        })
        .await;

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
}
