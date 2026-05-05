//! Core error and result types for nebula-resilience.

use std::{borrow::Cow, time::Duration};

/// Returned by all resilience operations.
///
/// `E` is the caller's own error type — never forced to map into a resilience error.
/// Errors produced by the patterns themselves (circuit open, bulkhead full, etc.)
/// are separate variants.
///
/// # Examples
///
/// ```rust,no_run
/// use std::time::Duration;
///
/// use nebula_resilience::{CallError, CallErrorKind};
///
/// let err: CallError<&str> = CallError::Timeout(Duration::from_millis(50));
/// assert!(err.is_retryable());
/// assert_eq!(err.kind(), CallErrorKind::Timeout);
///
/// let err: CallError<&str> = CallError::Operation("downstream failed");
/// assert_eq!(err.operation(), Some(&"downstream failed"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CallError<E> {
    /// The operation itself returned an error (possibly after retries exhausted).
    Operation(E),
    /// Circuit breaker is open — request rejected immediately.
    CircuitOpen,
    /// Bulkhead is at capacity — request rejected.
    BulkheadFull,
    /// Timeout elapsed before the operation completed.
    Timeout(Duration),
    /// All retry attempts exhausted; contains the last operation error.
    RetriesExhausted {
        /// Total number of attempts made.
        attempts: u32,
        /// Last error returned by the operation.
        last: E,
    },
    /// Operation was cancelled via `CancellationContext`.
    Cancelled {
        /// Optional human-readable reason for cancellation.
        /// `Cow` avoids heap allocation for static reasons (the common case).
        reason: Option<Cow<'static, str>>,
    },
    /// Load shed — system is overloaded, request rejected without queuing.
    LoadShed,
    /// Rate limit exceeded.
    RateLimited {
        /// Optional hint for when to retry. `None` means unknown.
        retry_after: Option<Duration>,
    },
    /// The fallback strategy itself failed after the primary operation failed.
    FallbackFailed {
        /// Human-readable reason for the fallback failure.
        /// `Cow` avoids heap allocation for static reasons (the common case).
        reason: Option<Cow<'static, str>>,
    },
    /// Fallback failed while handling a primary failure.
    ///
    /// This variant preserves both sides of the failure so callers and telemetry
    /// can distinguish "what originally failed" from "why graceful degradation
    /// also failed".
    FallbackFailedWithContext {
        /// Original failure that selected fallback.
        primary: Box<Self>,
        /// Failure returned by the fallback path.
        fallback: Box<Self>,
    },
}

impl<E: std::fmt::Display> std::fmt::Display for CallError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Operation(e) => write!(f, "operation error: {e}"),
            Self::CircuitOpen => write!(f, "circuit breaker is open"),
            Self::BulkheadFull => write!(f, "bulkhead is at capacity"),
            Self::Timeout(d) => write!(f, "operation timed out after {d:?}"),
            Self::RetriesExhausted { attempts, last } => {
                write!(f, "operation failed after {attempts} attempt(s): {last}")
            },
            Self::Cancelled { reason: Some(r) } => write!(f, "operation cancelled: {r}"),
            Self::Cancelled { reason: None } => write!(f, "operation cancelled"),
            Self::LoadShed => write!(f, "request load-shed due to overload"),
            Self::RateLimited {
                retry_after: Some(d),
            } => write!(f, "rate limit exceeded (retry after {d:?})"),
            Self::RateLimited { retry_after: None } => write!(f, "rate limit exceeded"),
            Self::FallbackFailed { reason: Some(r) } => write!(f, "fallback failed: {r}"),
            Self::FallbackFailed { reason: None } => write!(f, "fallback failed"),
            Self::FallbackFailedWithContext { primary, fallback } => write!(
                f,
                "fallback failed after primary failure ({primary}): {fallback}"
            ),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for CallError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Operation(e) => Some(e),
            Self::RetriesExhausted { last, .. } => Some(last),
            Self::FallbackFailedWithContext { fallback, .. } => Some(fallback.as_ref()),
            _ => None,
        }
    }
}

impl<E> CallError<E> {
    // ── Constructors ─────────────────────────────────────────────────────

    /// Cancelled without a reason.
    #[must_use]
    pub const fn cancelled() -> Self {
        Self::Cancelled { reason: None }
    }

    /// Cancelled with a static reason (zero heap allocation).
    #[must_use]
    pub const fn cancelled_with(reason: &'static str) -> Self {
        Self::Cancelled {
            reason: Some(Cow::Borrowed(reason)),
        }
    }

    /// Fallback failed without a reason.
    #[must_use]
    pub const fn fallback_failed() -> Self {
        Self::FallbackFailed { reason: None }
    }

    /// Fallback failed with a static reason (zero heap allocation).
    #[must_use]
    pub const fn fallback_failed_with(reason: &'static str) -> Self {
        Self::FallbackFailed {
            reason: Some(Cow::Borrowed(reason)),
        }
    }

    /// Fallback failed while trying to recover `primary`.
    ///
    /// Use this when a fallback strategy returns a distinct failure and the
    /// caller needs to retain the original failure for diagnosis.
    #[must_use]
    pub fn fallback_failed_with_context(primary: Self, fallback: Self) -> Self {
        Self::FallbackFailedWithContext {
            primary: Box::new(primary),
            fallback: Box::new(fallback),
        }
    }

    /// Rate limited without a retry-after hint.
    #[must_use]
    pub const fn rate_limited() -> Self {
        Self::RateLimited { retry_after: None }
    }

    /// Rate limited with a retry-after hint.
    #[must_use]
    pub const fn rate_limited_after(retry_after: Duration) -> Self {
        Self::RateLimited {
            retry_after: Some(retry_after),
        }
    }

    // ── Predicates ───────────────────────────────────────────────────────

    /// Returns true if the error class suggests a retry might succeed.
    ///
    /// `Timeout`, `RateLimited`, and `BulkheadFull` are considered retryable because
    /// they represent transient resource pressure, not permanent failures.
    ///
    /// `Operation` is never automatically retryable — classification is delegated
    /// to the inner error's [`Classify`](nebula_error::Classify) implementation.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout(_) | Self::RateLimited { .. } | Self::BulkheadFull
        )
    }

    /// Returns true if the error represents a cancellation.
    #[must_use]
    pub const fn is_cancellation(&self) -> bool {
        matches!(self, Self::Cancelled { .. })
    }

    /// Extract the inner operation error, if this is an `Operation` or `RetriesExhausted` variant.
    #[must_use]
    pub fn into_operation(self) -> Option<E> {
        match self {
            Self::Operation(e) | Self::RetriesExhausted { last: e, .. } => Some(e),
            _ => None,
        }
    }

    /// Reference to the inner operation error, if this is an `Operation` or `RetriesExhausted`
    /// variant.
    #[must_use]
    pub const fn operation(&self) -> Option<&E> {
        match self {
            Self::Operation(e) | Self::RetriesExhausted { last: e, .. } => Some(e),
            _ => None,
        }
    }

    /// Map the inner operation error, leaving pattern errors unchanged.
    pub fn map_operation<F, E2>(self, mut f: F) -> CallError<E2>
    where
        F: FnMut(E) -> E2,
    {
        self.map_operation_inner(&mut f)
    }

    fn map_operation_inner<F, E2>(self, f: &mut F) -> CallError<E2>
    where
        F: FnMut(E) -> E2,
    {
        match self {
            Self::Operation(e) => CallError::Operation(f(e)),
            Self::RetriesExhausted { attempts, last } => CallError::RetriesExhausted {
                attempts,
                last: f(last),
            },
            Self::CircuitOpen => CallError::CircuitOpen,
            Self::BulkheadFull => CallError::BulkheadFull,
            Self::Timeout(d) => CallError::Timeout(d),
            Self::Cancelled { reason } => CallError::Cancelled { reason },
            Self::LoadShed => CallError::LoadShed,
            Self::RateLimited { retry_after } => CallError::RateLimited { retry_after },
            Self::FallbackFailed { reason } => CallError::FallbackFailed { reason },
            Self::FallbackFailedWithContext { primary, fallback } => {
                CallError::FallbackFailedWithContext {
                    primary: Box::new(primary.map_operation_inner(f)),
                    fallback: Box::new(fallback.map_operation_inner(f)),
                }
            },
        }
    }

    /// Transform the inner error with separate handlers for `Operation` and
    /// `RetriesExhausted`. All other (fieldless) variants pass through unchanged.
    ///
    /// Unlike [`map_operation`](Self::map_operation), the handlers return
    /// `CallError<E2>` directly, allowing variant changes (e.g., converting
    /// `Operation(())` into `Cancelled`).
    pub fn flat_map_inner<E2>(
        self,
        mut on_operation: impl FnMut(E) -> CallError<E2>,
        mut on_retries: impl FnMut(u32, E) -> CallError<E2>,
    ) -> CallError<E2> {
        self.flat_map_inner_impl(&mut on_operation, &mut on_retries)
    }

    fn flat_map_inner_impl<E2, F, R>(
        self,
        on_operation: &mut F,
        on_retries: &mut R,
    ) -> CallError<E2>
    where
        F: FnMut(E) -> CallError<E2>,
        R: FnMut(u32, E) -> CallError<E2>,
    {
        match self {
            Self::Operation(e) => on_operation(e),
            Self::RetriesExhausted { attempts, last } => on_retries(attempts, last),
            Self::CircuitOpen => CallError::CircuitOpen,
            Self::BulkheadFull => CallError::BulkheadFull,
            Self::Timeout(d) => CallError::Timeout(d),
            Self::Cancelled { reason } => CallError::Cancelled { reason },
            Self::LoadShed => CallError::LoadShed,
            Self::RateLimited { retry_after } => CallError::RateLimited { retry_after },
            Self::FallbackFailed { reason } => CallError::FallbackFailed { reason },
            Self::FallbackFailedWithContext { primary, fallback } => {
                CallError::FallbackFailedWithContext {
                    primary: Box::new(primary.flat_map_inner_impl(on_operation, on_retries)),
                    fallback: Box::new(fallback.flat_map_inner_impl(on_operation, on_retries)),
                }
            },
        }
    }

    /// Returns the retry-after hint for `RateLimited` errors, if available.
    #[must_use]
    pub const fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after } => *retry_after,
            _ => None,
        }
    }

    /// Returns the primary and fallback failures when both are preserved.
    #[must_use]
    pub fn fallback_context(&self) -> Option<(&Self, &Self)> {
        match self {
            Self::FallbackFailedWithContext { primary, fallback } => Some((primary, fallback)),
            _ => None,
        }
    }

    pub(crate) fn into_erased_for_fallback(self) -> (CallError<()>, Self) {
        match self {
            Self::Operation(e) => (CallError::Operation(()), Self::Operation(e)),
            Self::RetriesExhausted { attempts, last } => (
                CallError::RetriesExhausted { attempts, last: () },
                Self::RetriesExhausted { attempts, last },
            ),
            Self::CircuitOpen => (CallError::CircuitOpen, Self::CircuitOpen),
            Self::BulkheadFull => (CallError::BulkheadFull, Self::BulkheadFull),
            Self::Timeout(duration) => (CallError::Timeout(duration), Self::Timeout(duration)),
            Self::Cancelled { reason } => (
                CallError::Cancelled {
                    reason: reason.clone(),
                },
                Self::Cancelled { reason },
            ),
            Self::LoadShed => (CallError::LoadShed, Self::LoadShed),
            Self::RateLimited { retry_after } => (
                CallError::RateLimited { retry_after },
                Self::RateLimited { retry_after },
            ),
            Self::FallbackFailed { reason } => (
                CallError::FallbackFailed {
                    reason: reason.clone(),
                },
                Self::FallbackFailed { reason },
            ),
            Self::FallbackFailedWithContext { primary, fallback } => (
                CallError::FallbackFailed {
                    reason: Some(Cow::Borrowed("fallback context erased")),
                },
                Self::FallbackFailedWithContext { primary, fallback },
            ),
        }
    }
}

impl<E: nebula_error::Classify> nebula_error::Classify for CallError<E> {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::Operation(e) | Self::RetriesExhausted { last: e, .. } => e.category(),
            Self::CircuitOpen | Self::LoadShed | Self::BulkheadFull => {
                nebula_error::ErrorCategory::Exhausted
            },
            Self::Timeout(_) => nebula_error::ErrorCategory::Timeout,
            Self::Cancelled { .. } => nebula_error::ErrorCategory::Cancelled,
            Self::RateLimited { .. } => nebula_error::ErrorCategory::RateLimit,
            Self::FallbackFailed { .. } | Self::FallbackFailedWithContext { .. } => {
                nebula_error::ErrorCategory::Internal
            },
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        match self {
            Self::Operation(e) | Self::RetriesExhausted { last: e, .. } => e.code(),
            Self::CircuitOpen => nebula_error::ErrorCode::new("RESILIENCE:CIRCUIT_OPEN"),
            Self::BulkheadFull => nebula_error::ErrorCode::new("RESILIENCE:BULKHEAD_FULL"),
            Self::Timeout(_) => nebula_error::ErrorCode::new("RESILIENCE:TIMEOUT"),
            Self::Cancelled { .. } => nebula_error::ErrorCode::new("RESILIENCE:CANCELLED"),
            Self::LoadShed => nebula_error::ErrorCode::new("RESILIENCE:LOAD_SHED"),
            Self::RateLimited { .. } => nebula_error::ErrorCode::new("RESILIENCE:RATE_LIMITED"),
            Self::FallbackFailed { .. } | Self::FallbackFailedWithContext { .. } => {
                nebula_error::ErrorCode::new("RESILIENCE:FALLBACK_FAILED")
            },
        }
    }

    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout(_) | Self::RateLimited { .. } | Self::BulkheadFull
        )
    }

    fn retry_hint(&self) -> Option<nebula_error::RetryHint> {
        self.retry_after().map(nebula_error::RetryHint::after)
    }
}

/// Returned from pattern constructors when configuration is invalid.
#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid resilience config: {message}")]
pub struct ConfigError {
    /// Name of the invalid configuration field.
    pub field: &'static str,
    /// Human-readable description of the validation error.
    /// `Cow` avoids heap allocation for static messages (95%+ of call sites).
    pub message: Cow<'static, str>,
}

impl nebula_error::Classify for ConfigError {
    fn category(&self) -> nebula_error::ErrorCategory {
        nebula_error::ErrorCategory::Validation
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new("RESILIENCE:CONFIG")
    }
}

impl ConfigError {
    /// Create a new configuration error.
    #[must_use]
    pub fn new(field: &'static str, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

/// Fieldless discriminant of [`CallError`] for dispatch without matching on data.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
#[non_exhaustive]
pub enum CallErrorKind {
    /// [`CallError::Operation`]
    Operation,
    /// [`CallError::CircuitOpen`]
    CircuitOpen,
    /// [`CallError::BulkheadFull`]
    BulkheadFull,
    /// [`CallError::Timeout`]
    Timeout,
    /// [`CallError::RetriesExhausted`]
    RetriesExhausted,
    /// [`CallError::Cancelled`]
    Cancelled,
    /// [`CallError::LoadShed`]
    LoadShed,
    /// [`CallError::RateLimited`]
    RateLimited,
    /// [`CallError::FallbackFailed`] or [`CallError::FallbackFailedWithContext`]
    FallbackFailed,
}

impl<E> CallError<E> {
    /// Returns the fieldless discriminant of this error.
    #[must_use]
    pub const fn kind(&self) -> CallErrorKind {
        match self {
            Self::Operation(_) => CallErrorKind::Operation,
            Self::CircuitOpen => CallErrorKind::CircuitOpen,
            Self::BulkheadFull => CallErrorKind::BulkheadFull,
            Self::Timeout(_) => CallErrorKind::Timeout,
            Self::RetriesExhausted { .. } => CallErrorKind::RetriesExhausted,
            Self::Cancelled { .. } => CallErrorKind::Cancelled,
            Self::LoadShed => CallErrorKind::LoadShed,
            Self::RateLimited { .. } => CallErrorKind::RateLimited,
            Self::FallbackFailed { .. } | Self::FallbackFailedWithContext { .. } => {
                CallErrorKind::FallbackFailed
            },
        }
    }
}

/// Convenience alias.
pub type CallResult<T, E> = Result<T, CallError<E>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    enum MyErr {
        Timeout,
    }

    impl std::fmt::Display for MyErr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Timeout => write!(f, "timeout"),
            }
        }
    }

    #[test]
    fn operation_is_not_retryable() {
        let e: CallError<MyErr> = CallError::Operation(MyErr::Timeout);
        assert!(!e.is_retryable());
    }

    #[test]
    fn circuit_open_is_not_retryable() {
        let e: CallError<MyErr> = CallError::CircuitOpen;
        assert!(!e.is_retryable());
    }

    #[test]
    fn timeout_is_retryable() {
        let e: CallError<MyErr> = CallError::Timeout(Duration::from_secs(1));
        assert!(e.is_retryable());
    }

    #[test]
    fn rate_limited_is_retryable() {
        let e: CallError<MyErr> = CallError::rate_limited();
        assert!(e.is_retryable());
    }

    #[test]
    fn rate_limited_retry_after_accessor() {
        let e: CallError<MyErr> = CallError::rate_limited_after(Duration::from_secs(5));
        assert_eq!(e.retry_after(), Some(Duration::from_secs(5)));
        assert!(e.is_retryable());

        let e2: CallError<MyErr> = CallError::rate_limited();
        assert_eq!(e2.retry_after(), None);
    }

    #[test]
    fn bulkhead_full_is_retryable() {
        let e: CallError<MyErr> = CallError::BulkheadFull;
        assert!(e.is_retryable());
    }

    #[test]
    fn cancelled_is_not_retryable() {
        let e: CallError<MyErr> = CallError::cancelled_with("shutdown");
        assert!(!e.is_retryable());
    }

    #[test]
    fn map_operation_preserves_inner_error() {
        let e: CallError<MyErr> = CallError::Operation(MyErr::Timeout);
        let mapped: CallError<String> = e.map_operation(|e| format!("{e:?}"));
        assert!(matches!(mapped, CallError::Operation(s) if s == "Timeout"));
    }

    #[test]
    fn into_operation_extracts_inner() {
        let e: CallError<MyErr> = CallError::Operation(MyErr::Timeout);
        assert_eq!(e.into_operation(), Some(MyErr::Timeout));
    }

    #[test]
    fn into_operation_returns_none_for_pattern_errors() {
        let e: CallError<MyErr> = CallError::CircuitOpen;
        assert_eq!(e.into_operation(), None);
    }

    #[test]
    fn operation_ref_extracts_inner() {
        let e: CallError<MyErr> = CallError::RetriesExhausted {
            attempts: 3,
            last: MyErr::Timeout,
        };
        assert_eq!(e.operation(), Some(&MyErr::Timeout));
    }

    #[test]
    fn kind_returns_correct_discriminant() {
        assert_eq!(
            CallError::<MyErr>::CircuitOpen.kind(),
            CallErrorKind::CircuitOpen
        );
        assert_eq!(
            CallError::Operation(MyErr::Timeout).kind(),
            CallErrorKind::Operation
        );
    }

    #[test]
    fn fallback_context_preserves_primary_and_fallback_errors() {
        let err = CallError::fallback_failed_with_context(
            CallError::Operation(MyErr::Timeout),
            CallError::fallback_failed_with("cache unavailable"),
        );

        assert_eq!(err.kind(), CallErrorKind::FallbackFailed);
        let (primary, fallback) = err.fallback_context().unwrap();
        assert!(matches!(primary, CallError::Operation(MyErr::Timeout)));
        assert!(matches!(fallback, CallError::FallbackFailed { .. }));
    }

    #[test]
    fn fallback_context_display_includes_primary_message() {
        let err = CallError::fallback_failed_with_context(
            CallError::RetriesExhausted {
                attempts: 2,
                last: MyErr::Timeout,
            },
            CallError::fallback_failed_with("cache unavailable"),
        );

        let display = err.to_string();

        assert!(display.contains("operation failed after 2 attempt(s): timeout"));
        assert!(display.contains("fallback failed: cache unavailable"));
    }

    #[test]
    fn map_operation_maps_nested_fallback_context() {
        let err = CallError::fallback_failed_with_context(
            CallError::Operation(MyErr::Timeout),
            CallError::RetriesExhausted {
                attempts: 2,
                last: MyErr::Timeout,
            },
        );

        let mapped = err.map_operation(|e| format!("{e:?}"));
        let (primary, fallback) = mapped.fallback_context().unwrap();

        assert!(matches!(primary, CallError::Operation(s) if s == "Timeout"));
        assert!(matches!(fallback, CallError::RetriesExhausted { last, .. } if last == "Timeout"));
    }
}
