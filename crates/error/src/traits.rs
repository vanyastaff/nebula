//! Core classification trait.
//!
//! Placeholder — full implementation in a later task.

use crate::{ErrorCategory, ErrorCode, ErrorSeverity, RetryHint};

/// Core trait for classifying errors by category, code, severity,
/// and retryability.
///
/// Implement this on your error enums (or use `#[derive(Classify)]`)
/// to integrate with `NebulaError` and the error infrastructure.
///
/// # Examples
///
/// ```
/// use nebula_error::{Classify, ErrorCategory, ErrorCode, ErrorSeverity, RetryHint, codes};
///
/// struct MyError;
///
/// impl Classify for MyError {
///     fn category(&self) -> ErrorCategory {
///         ErrorCategory::Internal
///     }
///     fn code(&self) -> ErrorCode {
///         codes::INTERNAL.clone()
///     }
/// }
///
/// let err = MyError;
/// assert_eq!(err.category(), ErrorCategory::Internal);
/// assert_eq!(err.severity(), ErrorSeverity::Error);
/// ```
pub trait Classify {
    /// The broad category of this error.
    fn category(&self) -> ErrorCategory;

    /// A machine-readable error code.
    fn code(&self) -> ErrorCode;

    /// Severity level. Defaults to [`ErrorSeverity::Error`].
    fn severity(&self) -> ErrorSeverity {
        ErrorSeverity::Error
    }

    /// Whether the error is retryable. Defaults to the category's
    /// [`is_default_retryable`](ErrorCategory::is_default_retryable).
    fn is_retryable(&self) -> bool {
        self.category().is_default_retryable()
    }

    /// Optional retry hint with backoff/attempt suggestions.
    fn retry_hint(&self) -> Option<RetryHint> {
        None
    }
}
