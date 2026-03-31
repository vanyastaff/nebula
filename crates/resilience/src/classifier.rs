//! Error classification for resilience patterns.
//!
//! [`ErrorClass`] describes how resilience patterns should treat an error:
//! whether it should trip a circuit breaker, trigger a retry, or be silently
//! ignored. [`ErrorClassifier`] maps application errors to an [`ErrorClass`].
//!
//! # Classification priority
//!
//! When used with [`RetryConfig`](crate::retry::RetryConfig):
//! 1. [`ErrorClassifier`] (set via [`with_classifier`](crate::retry::RetryConfig::with_classifier)
//!    or [`retry_if`](crate::retry::RetryConfig::retry_if) shorthand)
//! 2. [`Classify::is_retryable()`](nebula_error::Classify::is_retryable) — default fallback
//!    when no classifier is set
//!
//! # Examples
//!
//! ```rust
//! use nebula_resilience::classifier::{ErrorClass, ErrorClassifier, FnClassifier};
//!
//! #[derive(Debug)]
//! enum ApiError { RateLimited, BadRequest, ServerDown }
//!
//! let classifier = FnClassifier::new(|e: &ApiError| match e {
//!     ApiError::RateLimited => ErrorClass::Overload,
//!     ApiError::BadRequest => ErrorClass::Permanent,
//!     ApiError::ServerDown => ErrorClass::Unavailable,
//! });
//!
//! assert!(classifier.classify(&ApiError::RateLimited).is_retryable());
//! assert!(!classifier.classify(&ApiError::BadRequest).is_retryable());
//! assert!(classifier.classify(&ApiError::ServerDown).counts_as_failure());
//! ```

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════════
// ERROR CLASS
// ═══════════════════════════════════════════════════════════════════════════════

/// How a resilience pattern should treat an error.
///
/// Each class maps to specific behavior in circuit breakers and retry:
///
/// | Class | Trips CB? | Retryable? | Semantics |
/// |-------------|-----------|------------|----------------------------------------------|
/// | Transient | Yes | Yes | Temporary issue, likely recovers on retry |
/// | Permanent | No | No | Invalid request — downstream is healthy |
/// | Timeout | Yes* | Yes | Slow response — `count_timeouts_as_failures` |
/// | Cancelled | No | No | Operation cancelled — nobody's fault |
/// | Overload | No | Yes | Upstream backpressure — not a failure |
/// | Unavailable | Yes | Yes | Downstream is down — circuit should open |
/// | Unknown | Yes | Yes | Unclassified — conservative defaults |
///
/// *Timeout behavior in CB is controlled by
/// [`count_timeouts_as_failures`](crate::circuit_breaker::CircuitBreakerConfig::count_timeouts_as_failures).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ErrorClass {
    /// Temporary issue — retry, counts as CB failure.
    Transient,
    /// Invalid request — downstream is healthy, don't trip CB.
    Permanent,
    /// Slow response — retry with backoff, CB behavior configurable.
    Timeout,
    /// Operation cancelled — don't retry, don't trip CB.
    Cancelled,
    /// Upstream backpressure (rate limit, quota) — retry, don't trip CB.
    Overload,
    /// Downstream is down — retry, trip CB.
    ///
    /// When used with a pipeline that has both retry and circuit breaker,
    /// `Unavailable` errors trip the CB. Subsequent retry attempts see
    /// `CallError::CircuitOpen` (not retryable), stopping the retry loop.
    /// This is the correct behavior: the CB protects the downstream.
    Unavailable,
    /// Unclassified error — conservative: retry + trip CB.
    Unknown,
}

impl ErrorClass {
    /// Whether this error class should count as a failure for circuit breakers.
    ///
    /// `Permanent`, `Cancelled`, and `Overload` do NOT count — the downstream
    /// service is healthy (or the fault is on our side).
    #[must_use]
    pub const fn counts_as_failure(self) -> bool {
        matches!(
            self,
            Self::Transient | Self::Timeout | Self::Unavailable | Self::Unknown
        )
    }

    /// Whether this error class suggests a retry might succeed.
    ///
    /// `Permanent` and `Cancelled` are never retried — the request is invalid
    /// or explicitly abandoned.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::Transient | Self::Timeout | Self::Overload | Self::Unavailable | Self::Unknown
        )
    }
}

impl fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transient => f.write_str("transient"),
            Self::Permanent => f.write_str("permanent"),
            Self::Timeout => f.write_str("timeout"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::Overload => f.write_str("overload"),
            Self::Unavailable => f.write_str("unavailable"),
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// ERROR CLASSIFIER TRAIT
// ═══════════════════════════════════════════════════════════════════════════════

/// Maps application errors to an [`ErrorClass`] for resilience decisions.
///
/// Implement this trait to control how circuit breakers, retry, and pipeline
/// steps treat specific error types.
///
/// This trait is designed to be implemented by downstream crates.
/// New methods will always have default implementations to avoid breaking changes.
pub trait ErrorClassifier<E>: Send + Sync {
    /// Classify an error into an [`ErrorClass`].
    fn classify(&self, error: &E) -> ErrorClass;
}

// Blanket impl for Arc<C> — allows Arc<dyn ErrorClassifier<E>> to be used directly.
impl<E, C: ErrorClassifier<E> + ?Sized> ErrorClassifier<E> for Arc<C> {
    fn classify(&self, error: &E) -> ErrorClass {
        (**self).classify(error)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// BUILT-IN IMPLEMENTATIONS
// ═══════════════════════════════════════════════════════════════════════════════

/// Classifies all errors as [`ErrorClass::Transient`] — always retryable,
/// always counts as CB failure.
///
/// Useful for tests and simple cases where all errors are treated equally.
#[derive(Debug, Clone, Copy, Default)]
pub struct AlwaysTransient;

impl<E> ErrorClassifier<E> for AlwaysTransient {
    fn classify(&self, _: &E) -> ErrorClass {
        ErrorClass::Transient
    }
}

/// Classifies all errors as [`ErrorClass::Permanent`] — never retried,
/// never trips CB.
#[derive(Debug, Clone, Copy, Default)]
pub struct AlwaysPermanent;

impl<E> ErrorClassifier<E> for AlwaysPermanent {
    fn classify(&self, _: &E) -> ErrorClass {
        ErrorClass::Permanent
    }
}

/// Closure-based classifier for ad-hoc classification.
///
/// # Examples
///
/// ```rust
/// use nebula_resilience::classifier::{ErrorClass, FnClassifier};
///
/// let classifier = FnClassifier::new(|e: &&str| {
///     if e.contains("timeout") { ErrorClass::Timeout }
///     else { ErrorClass::Transient }
/// });
/// ```
pub struct FnClassifier<E, F> {
    f: F,
    // `fn(&E)` instead of `E` — contravariant over `E`, which is correct for
    // a classifier that only borrows `&E`. Using plain `PhantomData<E>` would
    // be invariant and overly restrictive for lifetime inference.
    _phantom: PhantomData<fn(&E)>,
}

impl<E, F: Fn(&E) -> ErrorClass + Send + Sync> FnClassifier<E, F> {
    /// Create a new closure-based classifier.
    pub const fn new(f: F) -> Self {
        Self {
            f,
            _phantom: PhantomData,
        }
    }
}

impl<E, F: Fn(&E) -> ErrorClass + Send + Sync> fmt::Debug for FnClassifier<E, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FnClassifier").finish_non_exhaustive()
    }
}

impl<E, F: Fn(&E) -> ErrorClass + Send + Sync> ErrorClassifier<E> for FnClassifier<E, F> {
    fn classify(&self, error: &E) -> ErrorClass {
        (self.f)(error)
    }
}

/// Bridges [`nebula_error::Classify`] to [`ErrorClassifier`].
///
/// Maps [`ErrorCategory`](nebula_error::ErrorCategory) to [`ErrorClass`]:
///
/// | `ErrorCategory` | `ErrorClass` | Rationale |
/// |---------------------------|-----------------|-------------------------------------|
/// | `External` | `Transient` | Downstream hiccup, retry may help |
/// | `Validation`, `NotFound` | `Permanent` | Bad request, downstream healthy |
/// | `Authentication`, `Authorization` | `Permanent` | Credentials wrong, not downstream |
/// | `Conflict`, `Unsupported` | `Permanent` | Client error, not retryable |
/// | `DataTooLarge` | `Permanent` | Payload issue, not retryable |
/// | `Timeout` | `Timeout` | Slow, CB configurable |
/// | `Cancelled` | `Cancelled` | Explicit cancellation |
/// | `RateLimit`, `Exhausted` | `Overload` | Backpressure, not a failure |
/// | `Unavailable` | `Unavailable` | Downstream down, trip CB |
/// | `Internal` | `Unknown` | Bug — conservative defaults |
/// | `_` (future variants) | `Unknown` | Safe default |
#[derive(Debug, Clone, Copy, Default)]
pub struct NebulaClassifier;

impl<E: nebula_error::Classify + Send + Sync> ErrorClassifier<E> for NebulaClassifier {
    fn classify(&self, error: &E) -> ErrorClass {
        use nebula_error::ErrorCategory;
        match error.category() {
            ErrorCategory::External => ErrorClass::Transient,
            ErrorCategory::Validation
            | ErrorCategory::Authentication
            | ErrorCategory::Authorization
            | ErrorCategory::NotFound
            | ErrorCategory::Conflict
            | ErrorCategory::Unsupported
            | ErrorCategory::DataTooLarge => ErrorClass::Permanent,
            ErrorCategory::Timeout => ErrorClass::Timeout,
            ErrorCategory::Cancelled => ErrorClass::Cancelled,
            ErrorCategory::RateLimit | ErrorCategory::Exhausted => ErrorClass::Overload,
            ErrorCategory::Unavailable => ErrorClass::Unavailable,
            // Internal and any future variants use conservative defaults.
            _ => ErrorClass::Unknown,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_class_counts_as_failure() {
        assert!(ErrorClass::Transient.counts_as_failure());
        assert!(ErrorClass::Timeout.counts_as_failure());
        assert!(ErrorClass::Unavailable.counts_as_failure());
        assert!(ErrorClass::Unknown.counts_as_failure());

        assert!(!ErrorClass::Permanent.counts_as_failure());
        assert!(!ErrorClass::Cancelled.counts_as_failure());
        assert!(!ErrorClass::Overload.counts_as_failure());
    }

    #[test]
    fn error_class_is_retryable() {
        assert!(ErrorClass::Transient.is_retryable());
        assert!(ErrorClass::Timeout.is_retryable());
        assert!(ErrorClass::Overload.is_retryable());
        assert!(ErrorClass::Unavailable.is_retryable());
        assert!(ErrorClass::Unknown.is_retryable());

        assert!(!ErrorClass::Permanent.is_retryable());
        assert!(!ErrorClass::Cancelled.is_retryable());
    }

    #[test]
    fn fn_classifier_works() {
        let classifier = FnClassifier::new(|e: &&str| {
            if e.contains("timeout") {
                ErrorClass::Timeout
            } else {
                ErrorClass::Transient
            }
        });

        assert_eq!(
            classifier.classify(&"connection timeout"),
            ErrorClass::Timeout
        );
        assert_eq!(
            classifier.classify(&"connection refused"),
            ErrorClass::Transient
        );
    }

    #[test]
    fn always_transient_classifies_all_as_transient() {
        let classifier = AlwaysTransient;
        assert_eq!(classifier.classify(&"any error"), ErrorClass::Transient);
    }

    #[test]
    fn always_permanent_classifies_all_as_permanent() {
        let classifier = AlwaysPermanent;
        assert_eq!(classifier.classify(&"any error"), ErrorClass::Permanent);
    }

    #[test]
    fn nebula_classifier_maps_categories() {
        use nebula_error::{Classify, ErrorCategory, ErrorCode, codes};

        #[derive(Debug)]
        struct Err(ErrorCategory);
        impl Classify for Err {
            fn category(&self) -> ErrorCategory {
                self.0
            }
            fn code(&self) -> ErrorCode {
                codes::INTERNAL
            }
        }

        let c = NebulaClassifier;
        assert_eq!(
            c.classify(&Err(ErrorCategory::External)),
            ErrorClass::Transient
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::Validation)),
            ErrorClass::Permanent
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::Authentication)),
            ErrorClass::Permanent
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::Authorization)),
            ErrorClass::Permanent
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::NotFound)),
            ErrorClass::Permanent
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::Conflict)),
            ErrorClass::Permanent
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::Unsupported)),
            ErrorClass::Permanent
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::DataTooLarge)),
            ErrorClass::Permanent
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::Timeout)),
            ErrorClass::Timeout
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::Cancelled)),
            ErrorClass::Cancelled
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::RateLimit)),
            ErrorClass::Overload
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::Exhausted)),
            ErrorClass::Overload
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::Unavailable)),
            ErrorClass::Unavailable
        );
        assert_eq!(
            c.classify(&Err(ErrorCategory::Internal)),
            ErrorClass::Unknown
        );
    }

    #[test]
    fn arc_classifier_delegates() {
        let classifier: Arc<dyn ErrorClassifier<&str>> = Arc::new(AlwaysTransient);
        assert_eq!(classifier.classify(&"err"), ErrorClass::Transient);
    }
}
