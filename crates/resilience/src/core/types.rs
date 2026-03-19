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
    RetriesExhausted { attempts: u32, last: E },
    /// Operation was cancelled via `CancellationContext`.
    Cancelled { reason: Option<String> },
    /// Load shed — system is overloaded, request rejected without queuing.
    LoadShed,
    /// Rate limit exceeded.
    RateLimited,
}

impl<E> CallError<E> {
    /// Returns true only if the error class suggests a retry might succeed.
    ///
    /// Note: `Operation` is never automatically retriable — the caller must
    /// supply a predicate via `RetryConfig::retry_if` to classify their errors.
    pub fn is_retriable(&self) -> bool {
        false // all pattern errors are non-retriable; operation retryability is predicate-driven
    }

    pub fn is_cancellation(&self) -> bool {
        matches!(self, Self::Cancelled { .. })
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
    pub field: &'static str,
    pub message: String,
}

impl ConfigError {
    pub fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
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
        NotFound,
    }

    #[test]
    fn call_error_is_retriable_for_operation() {
        let e: CallError<MyErr> = CallError::Operation(MyErr::Timeout);
        assert!(!e.is_retriable()); // CallError::Operation is never auto-retriable
    }

    #[test]
    fn call_error_is_retriable_for_circuit_open() {
        let e: CallError<MyErr> = CallError::CircuitOpen;
        assert!(!e.is_retriable()); // CB open — don't retry
    }

    #[test]
    fn call_error_map_operation() {
        let e: CallError<MyErr> = CallError::Operation(MyErr::Timeout);
        let mapped: CallError<String> = e.map_operation(|e| format!("{e:?}"));
        assert!(matches!(mapped, CallError::Operation(s) if s == "Timeout"));
    }

    #[test]
    fn cancelled_is_not_retriable() {
        let e: CallError<MyErr> = CallError::Cancelled {
            reason: Some("shutdown".into()),
        };
        assert!(!e.is_retriable());
    }
}
