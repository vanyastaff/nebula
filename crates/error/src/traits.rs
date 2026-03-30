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

/// A predicate-based error classifier for filtering errors by category.
///
/// Used by the resilience layer to decide which errors to retry, route,
/// or escalate without requiring a full `Classify` implementation.
///
/// # Examples
///
/// ```
/// use nebula_error::{Classify, ErrorCategory, ErrorCode, ErrorClassifier, codes};
///
/// let transient_only = ErrorClassifier::new(|cat| matches!(
///     cat,
///     ErrorCategory::Timeout | ErrorCategory::RateLimit | ErrorCategory::External
/// ));
///
/// struct TimeoutErr;
/// impl Classify for TimeoutErr {
///     fn category(&self) -> ErrorCategory { ErrorCategory::Timeout }
///     fn code(&self) -> ErrorCode { codes::TIMEOUT.clone() }
/// }
///
/// assert!(transient_only.matches(&TimeoutErr));
/// ```
pub struct ErrorClassifier {
    predicate: Box<dyn Fn(ErrorCategory) -> bool + Send + Sync>,
}

impl ErrorClassifier {
    /// Creates a classifier from a category predicate.
    #[must_use]
    pub fn new(predicate: impl Fn(ErrorCategory) -> bool + Send + Sync + 'static) -> Self {
        Self {
            predicate: Box::new(predicate),
        }
    }

    /// Returns `true` if the error's category matches the predicate.
    pub fn matches(&self, error: &impl Classify) -> bool {
        (self.predicate)(error.category())
    }

    /// A built-in classifier that matches all default-retryable categories.
    #[must_use]
    pub fn retryable() -> Self {
        Self::new(|cat| cat.is_default_retryable())
    }

    /// A built-in classifier that matches all client errors.
    #[must_use]
    pub fn client_errors() -> Self {
        Self::new(|cat| cat.is_client_error())
    }

    /// A built-in classifier that matches all server errors.
    #[must_use]
    pub fn server_errors() -> Self {
        Self::new(|cat| cat.is_server_error())
    }
}

impl std::fmt::Debug for ErrorClassifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErrorClassifier").finish_non_exhaustive()
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

    #[test]
    fn error_classifier_retryable_matches_timeout() {
        let classifier = ErrorClassifier::retryable();
        let err = MinimalError {
            cat: ErrorCategory::Timeout,
        };
        assert!(classifier.matches(&err));
    }

    #[test]
    fn error_classifier_retryable_rejects_validation() {
        let classifier = ErrorClassifier::retryable();
        let err = MinimalError {
            cat: ErrorCategory::Validation,
        };
        assert!(!classifier.matches(&err));
    }

    #[test]
    fn error_classifier_custom_predicate() {
        let only_auth = ErrorClassifier::new(|cat| {
            matches!(
                cat,
                ErrorCategory::Authentication | ErrorCategory::Authorization
            )
        });
        let auth = MinimalError {
            cat: ErrorCategory::Authentication,
        };
        let timeout = MinimalError {
            cat: ErrorCategory::Timeout,
        };
        assert!(only_auth.matches(&auth));
        assert!(!only_auth.matches(&timeout));
    }
}
