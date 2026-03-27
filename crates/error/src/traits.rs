//! Core classification trait.
//!
//! The [`Classify`] trait is the central contract of the nebula-error crate.
//! Every domain error type that participates in the Nebula error infrastructure
//! must implement it — either manually or via `#[derive(Classify)]`.

use crate::{ErrorCategory, ErrorCode, ErrorSeverity, RetryHint};

/// Core trait for classifying errors by category, code, severity,
/// and retryability.
///
/// Implement this on your error enums (or use `#[derive(Classify)]`)
/// to integrate with [`NebulaError`](crate::NebulaError) and the error
/// infrastructure.
///
/// Only [`category`](Classify::category) and [`code`](Classify::code)
/// are required. The remaining methods have sensible defaults:
///
/// - [`severity`](Classify::severity) defaults to [`ErrorSeverity::Error`].
/// - [`is_retryable`](Classify::is_retryable) delegates to
///   [`ErrorCategory::is_default_retryable`].
/// - [`retry_hint`](Classify::retry_hint) returns `None`.
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
/// assert!(!err.is_retryable()); // Internal is not default-retryable
/// assert!(err.retry_hint().is_none());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::codes;

    /// Minimal impl — only required methods.
    struct MinimalError {
        cat: ErrorCategory,
    }

    impl Classify for MinimalError {
        fn category(&self) -> ErrorCategory {
            self.cat
        }
        fn code(&self) -> ErrorCode {
            codes::INTERNAL.clone()
        }
    }

    /// Full impl — overrides all optional methods.
    struct FullError;

    impl Classify for FullError {
        fn category(&self) -> ErrorCategory {
            ErrorCategory::RateLimit
        }
        fn code(&self) -> ErrorCode {
            codes::RATE_LIMIT.clone()
        }
        fn severity(&self) -> ErrorSeverity {
            ErrorSeverity::Warning
        }
        fn is_retryable(&self) -> bool {
            true
        }
        fn retry_hint(&self) -> Option<RetryHint> {
            Some(RetryHint::after(Duration::from_secs(30)).with_max_attempts(5))
        }
    }

    #[test]
    fn default_severity_is_error() {
        let err = MinimalError {
            cat: ErrorCategory::Internal,
        };
        assert_eq!(err.severity(), ErrorSeverity::Error);
    }

    #[test]
    fn default_retryable_from_category() {
        let timeout = MinimalError {
            cat: ErrorCategory::Timeout,
        };
        assert!(timeout.is_retryable());

        let validation = MinimalError {
            cat: ErrorCategory::Validation,
        };
        assert!(!validation.is_retryable());
    }

    #[test]
    fn default_retry_hint_is_none() {
        let err = MinimalError {
            cat: ErrorCategory::Internal,
        };
        assert!(err.retry_hint().is_none());
    }

    #[test]
    fn custom_overrides() {
        let err = FullError;
        assert_eq!(err.category(), ErrorCategory::RateLimit);
        assert_eq!(err.code(), codes::RATE_LIMIT);
        assert_eq!(err.severity(), ErrorSeverity::Warning);
        assert!(err.is_retryable());

        let hint = err.retry_hint().expect("should have retry hint");
        assert_eq!(hint.after, Some(Duration::from_secs(30)));
        assert_eq!(hint.max_attempts, Some(5));
    }
}
