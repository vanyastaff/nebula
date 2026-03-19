//! Core error and result types for nebula-resilience.

use std::time::Duration;

/// Returned by all resilience operations.
///
/// `E` is the caller's own error type — never forced to map into a resilience error.
/// Errors produced by the patterns themselves (circuit open, bulkhead full, etc.)
/// are separate variants.
#[derive(Debug)]
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
        reason: Option<String>,
    },
    /// Load shed — system is overloaded, request rejected without queuing.
    LoadShed,
    /// Rate limit exceeded.
    RateLimited,
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
            }
            Self::Cancelled { reason: Some(r) } => write!(f, "operation cancelled: {r}"),
            Self::Cancelled { reason: None } => write!(f, "operation cancelled"),
            Self::LoadShed => write!(f, "request load-shed due to overload"),
            Self::RateLimited => write!(f, "rate limit exceeded"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for CallError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Operation(e) => Some(e),
            Self::RetriesExhausted { last, .. } => Some(last),
            _ => None,
        }
    }
}

impl<E> CallError<E> {
    /// Returns true if the error class suggests a retry might succeed.
    ///
    /// `Timeout`, `RateLimited`, and `BulkheadFull` are considered retriable because
    /// they represent transient resource pressure, not permanent failures.
    ///
    /// `Operation` is never automatically retriable — the caller must supply a predicate
    /// via `RetryConfig::retry_if` to classify their own errors.
    #[must_use]
    pub const fn is_retriable(&self) -> bool {
        matches!(
            self,
            Self::Timeout(_) | Self::RateLimited | Self::BulkheadFull
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

    /// Reference to the inner operation error, if this is an `Operation` or `RetriesExhausted` variant.
    #[must_use]
    pub const fn operation(&self) -> Option<&E> {
        match self {
            Self::Operation(e) | Self::RetriesExhausted { last: e, .. } => Some(e),
            _ => None,
        }
    }

    /// Map the inner operation error, leaving pattern errors unchanged.
    pub fn map_operation<F, E2>(self, f: F) -> CallError<E2>
    where
        F: FnOnce(E) -> E2,
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
            Self::RateLimited => CallError::RateLimited,
        }
    }
}

/// Returned from pattern constructors when configuration is invalid.
#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid resilience config: {message}")]
pub struct ConfigError {
    /// Name of the invalid configuration field.
    pub field: &'static str,
    /// Human-readable description of the validation error.
    pub message: String,
}

impl ConfigError {
    /// Create a new configuration error.
    pub fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

/// Fieldless discriminant of [`CallError`] for dispatch without matching on data.
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
            Self::RateLimited => CallErrorKind::RateLimited,
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

    #[test]
    fn operation_is_not_retriable() {
        let e: CallError<MyErr> = CallError::Operation(MyErr::Timeout);
        assert!(!e.is_retriable());
    }

    #[test]
    fn circuit_open_is_not_retriable() {
        let e: CallError<MyErr> = CallError::CircuitOpen;
        assert!(!e.is_retriable());
    }

    #[test]
    fn timeout_is_retriable() {
        let e: CallError<MyErr> = CallError::Timeout(std::time::Duration::from_secs(1));
        assert!(e.is_retriable());
    }

    #[test]
    fn rate_limited_is_retriable() {
        let e: CallError<MyErr> = CallError::RateLimited;
        assert!(e.is_retriable());
    }

    #[test]
    fn bulkhead_full_is_retriable() {
        let e: CallError<MyErr> = CallError::BulkheadFull;
        assert!(e.is_retriable());
    }

    #[test]
    fn cancelled_is_not_retriable() {
        let e: CallError<MyErr> = CallError::Cancelled {
            reason: Some("shutdown".into()),
        };
        assert!(!e.is_retriable());
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
}
