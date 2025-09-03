//! Performance timing utilities

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use pin_project::pin_project;

/// A timer that measures execution time
#[derive(Debug)]
pub struct Timer {
    name: String,
    start: Instant,
    level: tracing::Level,
    threshold: Option<Duration>,
}

impl Timer {
    /// Create a new timer
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            start: Instant::now(),
            level: tracing::Level::INFO,
            threshold: None,
        }
    }

    /// Set the log level
    pub fn level(mut self, level: tracing::Level) -> Self {
        self.level = level;
        self
    }

    /// Only log if duration exceeds threshold
    pub fn threshold(mut self, duration: Duration) -> Self {
        self.threshold = Some(duration);
        self
    }

    /// Get elapsed time
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Complete the timer
    pub fn complete(self) -> Duration {
        let elapsed = self.elapsed();

        if let Some(threshold) = self.threshold {
            if elapsed < threshold {
                return elapsed;
            }
        }

        let ms = elapsed.as_millis();
        match self.level {
            tracing::Level::ERROR => tracing::error!(name = %self.name, ms, "Timer completed"),
            tracing::Level::WARN => tracing::warn!(name = %self.name, ms, "Timer completed"),
            tracing::Level::INFO => tracing::info!(name = %self.name, ms, "Timer completed"),
            tracing::Level::DEBUG => tracing::debug!(name = %self.name, ms, "Timer completed"),
            tracing::Level::TRACE => tracing::trace!(name = %self.name, ms, "Timer completed"),
        }

        elapsed
    }
}

/// RAII guard for automatic timing
pub struct TimerGuard {
    timer: Option<Timer>,
}

impl TimerGuard {
    /// Create a new timer guard
    pub fn new(name: impl Into<String>) -> Self {
        Self { timer: Some(Timer::new(name)) }
    }
}

impl Drop for TimerGuard {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.complete();
        }
    }
}

/// Extension trait for timing futures
pub trait Timed: Sized {
    /// Time the execution of this future
    fn timed(self, name: impl Into<String>) -> TimedFuture<Self> {
        TimedFuture { inner: self, timer: Timer::new(name) }
    }
}

impl<F> Timed for F where F: Future {}

/// A future that times its execution
#[pin_project]
pub struct TimedFuture<F> {
    #[pin]
    inner: F,
    timer: Timer,
}

impl<F: Future> Future for TimedFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let result = this.inner.poll(cx);

        if result.is_ready() {
            let elapsed = this.timer.start.elapsed();
            let ms = elapsed.as_millis();
            tracing::info!(name = %this.timer.name, ms, "Future completed");
        }

        result
    }
}
