//! Retryable trait for domain errors
//!
//! This module provides a simple trait that domain errors can implement
//! to work with nebula-resilience retry utilities.
//!
//! This is the ONLY shared interface between crates - no central error types.

use std::error::Error;
use std::time::Duration;

/// Trait for errors that support retry logic
///
/// Domain errors implement this trait to indicate whether they can be retried
/// and what delay should be used.
///
/// # Examples
///
/// ```
/// use nebula_resilience::Retryable;
/// use thiserror::Error;
/// use std::time::Duration;
///
/// #[derive(Error, Debug)]
/// pub enum DatabaseError {
///     #[error("Connection timeout")]
///     Timeout,
///
///     #[error("Invalid query")]
///     InvalidQuery,
/// }
///
/// impl Retryable for DatabaseError {
///     fn is_retryable(&self) -> bool {
///         matches!(self, Self::Timeout)
///     }
///
///     fn retry_delay(&self) -> Duration {
///         Duration::from_millis(500)
///     }
/// }
/// ```
pub trait Retryable: Error {
    /// Check if this error can be retried
    ///
    /// Default: `true`
    fn is_retryable(&self) -> bool {
        true
    }

    /// Get retry delay for this error
    ///
    /// Default: 100ms
    fn retry_delay(&self) -> Duration {
        Duration::from_millis(100)
    }

    /// Get maximum retry attempts for this error
    ///
    /// Default: `None` (use policy default)
    fn max_retries(&self) -> Option<u32> {
        None
    }
}

// ============================================================================
// BLANKET IMPLEMENTATIONS
// ============================================================================

/// `std::io::Error` is retryable for certain error kinds
impl Retryable for std::io::Error {
    fn is_retryable(&self) -> bool {
        use std::io::ErrorKind::{
            ConnectionAborted, ConnectionReset, Interrupted, TimedOut, WouldBlock,
        };
        matches!(
            self.kind(),
            Interrupted | WouldBlock | TimedOut | ConnectionReset | ConnectionAborted
        )
    }

    fn retry_delay(&self) -> Duration {
        Duration::from_millis(100)
    }
}

/// `fmt::Error` is not retryable
impl Retryable for std::fmt::Error {
    fn is_retryable(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thiserror::Error;

    #[derive(Error, Debug)]
    enum TestError {
        #[error("Transient error")]
        Transient,
        #[error("Permanent error")]
        Permanent,
    }

    impl Retryable for TestError {
        fn is_retryable(&self) -> bool {
            matches!(self, Self::Transient)
        }

        fn retry_delay(&self) -> Duration {
            match self {
                Self::Transient => Duration::from_millis(200),
                Self::Permanent => Duration::from_millis(0),
            }
        }
    }

    #[test]
    fn test_retryable_transient() {
        let err = TestError::Transient;
        assert!(err.is_retryable());
        assert_eq!(err.retry_delay(), Duration::from_millis(200));
    }

    #[test]
    fn test_retryable_permanent() {
        let err = TestError::Permanent;
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_io_error_retryable() {
        let err = std::io::Error::from(std::io::ErrorKind::TimedOut);
        assert!(err.is_retryable());
    }

    #[test]
    fn test_io_error_not_retryable() {
        let err = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(!err.is_retryable());
    }
}
