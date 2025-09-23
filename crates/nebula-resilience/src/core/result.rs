//! Result types and error handling utilities

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::error::{ResilienceError, ErrorClass};

/// Result type for resilience operations
pub type ResilienceResult<T> = Result<T, ResilienceError>;

/// Extension trait for Result types
pub trait ResultExt<T> {
    /// Map error with context
    fn with_context<C, F>(self, f: F) -> ResilienceResult<T>
    where
        C: fmt::Display + Send + Sync + 'static,
        F: FnOnce() -> C;

    /// Convert to resilience result
    fn into_resilience(self) -> ResilienceResult<T>;

    /// Check if error is retryable
    fn is_retryable_error(&self) -> bool;

    /// Get error classification
    fn error_class(&self) -> Option<ErrorClass>;

    /// Add timeout context
    fn timeout_context(self, duration: std::time::Duration) -> ResilienceResult<T>;

    /// Wrap error with custom message
    fn wrap_err(self, msg: impl Into<String>) -> ResilienceResult<T>;
}

impl<T, E> ResultExt<T> for Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn with_context<C, F>(self, f: F) -> ResilienceResult<T>
    where
        C: fmt::Display + Send + Sync + 'static,
        F: FnOnce() -> C,
    {
        self.map_err(|e| ResilienceError::Custom {
            message: format!("{}: {}", f(), e),
            retryable: false,
            source: Some(Box::new(e)),
        })
    }

    fn into_resilience(self) -> ResilienceResult<T> {
        self.map_err(|e| ResilienceError::Custom {
            message: e.to_string(),
            retryable: false,
            source: Some(Box::new(e)),
        })
    }

    fn is_retryable_error(&self) -> bool {
        matches!(self, Err(_))
    }

    fn error_class(&self) -> Option<ErrorClass> {
        if self.is_err() {
            Some(ErrorClass::Unknown)
        } else {
            None
        }
    }

    fn timeout_context(self, duration: std::time::Duration) -> ResilienceResult<T> {
        self.map_err(|e| ResilienceError::Timeout {
            duration,
            context: Some(e.to_string()),
        })
    }

    fn wrap_err(self, msg: impl Into<String>) -> ResilienceResult<T> {
        self.map_err(|e| ResilienceError::Custom {
            message: format!("{}: {}", msg.into(), e),
            retryable: false,
            source: Some(Box::new(e)),
        })
    }
}

/// Async result utilities
pub trait AsyncResultExt<T>: Future<Output = ResilienceResult<T>> {
    /// Add timeout to async operation
    fn with_timeout(self, duration: std::time::Duration) -> TimeoutFuture<Self>
    where
        Self: Sized,
    {
        TimeoutFuture {
            inner: Box::pin(self),
            deadline: std::time::Instant::now() + duration,
        }
    }
}

impl<F, T> AsyncResultExt<T> for F where F: Future<Output = ResilienceResult<T>> {}

/// Future that can timeout
pub struct TimeoutFuture<F> {
    inner: Pin<Box<F>>,
    deadline: std::time::Instant,
}

impl<F, T> Future for TimeoutFuture<F>
where
    F: Future<Output = ResilienceResult<T>>,
{
    type Output = ResilienceResult<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if std::time::Instant::now() >= self.deadline {
            return Poll::Ready(Err(ResilienceError::Timeout {
                duration: self.deadline.elapsed(),
                context: Some("Operation exceeded deadline".to_string()),
            }));
        }

        self.inner.as_mut().poll(cx)
    }
}

/// Builder for collecting multiple errors
#[derive(Debug, Default)]
pub struct ErrorCollector {
    errors: Vec<ResilienceError>,
    strategy: ErrorStrategy,
}

/// Error collection strategy
#[derive(Debug, Default, Clone, Copy)]
pub enum ErrorStrategy {
    /// Return first error
    #[default]
    FirstError,
    /// Return last error
    LastError,
    /// Return most severe error
    MostSevere,
    /// Combine all errors
    CombineAll,
}

impl ErrorCollector {
    /// Create new error collector
    pub fn new() -> Self {
        Self::default()
    }

    /// Set error strategy
    pub fn with_strategy(mut self, strategy: ErrorStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Add an error
    pub fn add_error(&mut self, error: ResilienceError) {
        self.errors.push(error);
    }

    /// Add result, collecting error if present
    pub fn add_result<T>(&mut self, result: ResilienceResult<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(e) => {
                self.add_error(e);
                None
            }
        }
    }

    /// Check if any errors collected
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Get collected error based on strategy
    pub fn into_result<T>(self) -> ResilienceResult<T> {
        if self.errors.is_empty() {
            return Err(ResilienceError::Custom {
                message: "No errors collected".to_string(),
                retryable: false,
                source: None,
            });
        }

        match self.strategy {
            ErrorStrategy::FirstError => Err(self.errors.into_iter().next().unwrap()),
            ErrorStrategy::LastError => Err(self.errors.into_iter().last().unwrap()),
            ErrorStrategy::MostSevere => {
                let most_severe = self
                    .errors
                    .into_iter()
                    .max_by_key(|e| match e.classify() {
                        ErrorClass::Configuration => 4,
                        ErrorClass::Permanent => 3,
                        ErrorClass::ResourceExhaustion => 2,
                        ErrorClass::Transient => 1,
                        ErrorClass::Unknown => 0,
                    })
                    .unwrap();
                Err(most_severe)
            }
            ErrorStrategy::CombineAll => {
                let messages: Vec<String> = self.errors.iter().map(|e| e.to_string()).collect();
                Err(ResilienceError::Custom {
                    message: format!("Multiple errors: {}", messages.join("; ")),
                    retryable: self.errors.iter().any(|e| e.is_retryable()),
                    source: None,
                })
            }
        }
    }
}

/// Helper macros
#[macro_export]
macro_rules! resilience_bail {
    ($msg:literal) => {
        return Err($crate::core::ResilienceError::Custom {
            message: $msg.to_string(),
            retryable: false,
            source: None,
        })
    };
    ($fmt:expr, $($arg:tt)*) => {
        return Err($crate::core::ResilienceError::Custom {
            message: format!($fmt, $($arg)*),
            retryable: false,
            source: None,
        })
    };
}

#[macro_export]
macro_rules! resilience_context {
    ($result:expr, $msg:literal) => {
        $result.with_context(|| $msg)
    };
    ($result:expr, $fmt:expr, $($arg:tt)*) => {
        $result.with_context(|| format!($fmt, $($arg)*))
    };
}