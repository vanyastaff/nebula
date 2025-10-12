//! Helper functions and utilities for metrics collection

use std::time::Instant;

/// Time a block of code and record duration as histogram
#[cfg(feature = "observability")]
pub fn timed_block<F, R>(name: &str, f: F) -> R
where
    F: FnOnce() -> R,
{
    let guard = TimingGuard::new(name);
    let result = f();
    drop(guard);
    result
}

#[cfg(not(feature = "observability"))]
pub fn timed_block<F, R>(_name: &str, f: F) -> R
where
    F: FnOnce() -> R,
{
    f()
}

/// Async version of timed_block
#[cfg(feature = "observability")]
pub async fn timed_block_async<F, Fut, R>(name: &str, f: F) -> R
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = R>,
{
    let guard = TimingGuard::new(name);
    let result = f().await;
    drop(guard);
    result
}

#[cfg(not(feature = "observability"))]
pub async fn timed_block_async<F, Fut, R>(_name: &str, f: F) -> R
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = R>,
{
    f().await
}

/// RAII timing guard
#[cfg(feature = "observability")]
pub struct TimingGuard {
    name: String,
    start: Instant,
}

#[cfg(feature = "observability")]
impl TimingGuard {
    /// Create a new timing guard
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            start: Instant::now(),
        }
    }
}

#[cfg(feature = "observability")]
impl Drop for TimingGuard {
    fn drop(&mut self) {
        let duration = self.start.elapsed().as_secs_f64();
        let name = self.name.clone();
        metrics::histogram!(name).record(duration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timed_block() {
        let result = timed_block("test_operation", || 42);
        assert_eq!(result, 42);
    }
}
