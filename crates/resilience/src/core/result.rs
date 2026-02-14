//! Result types and error handling utilities

use std::fmt;

use super::error::{ErrorClass, ResilienceError};

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
        self.is_err()
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

/// Macro for adding context to resilience results
#[macro_export]
macro_rules! resilience_context {
    ($result:expr, $msg:literal) => {
        $result.with_context(|| $msg)
    };
    ($result:expr, $fmt:expr, $($arg:tt)*) => {
        $result.with_context(|| format!($fmt, $($arg)*))
    };
}
