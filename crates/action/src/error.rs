use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Retry-strategy hint attached by the action body to a failing
/// `Retryable` or `Fatal` error.
///
/// This is a **user-supplied** hint about *how* the engine should retry,
/// not a framework-level error classifier. For the cross-crate taxonomy
/// tag (`ACTION:VALIDATION`, `ACTION:CANCELLED`, ...), use
/// `<ActionError as nebula_error::Classify>::code()`.
///
/// The two concepts used to collide under the name `ErrorCode` — the
/// rename to `RetryHintCode` disambiguates them at the type level.
///
/// Engine matches on these hints for smarter retry strategies:
/// - `RateLimited` → respect Retry-After header
/// - `AuthExpired` → refresh credential before retry
/// - `UpstreamTimeout` → increase timeout on retry
///
/// # Examples
///
/// ```
/// use nebula_action::RetryHintCode;
///
/// let hint = RetryHintCode::RateLimited;
/// assert_eq!(serde_json::to_string(&hint).unwrap(), "\"RateLimited\"");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RetryHintCode {
    /// Remote API returned 429 Too Many Requests.
    RateLimited,
    /// Concurrent modification conflict (optimistic lock failure).
    Conflict,
    /// Credential expired — engine may refresh and retry.
    AuthExpired,
    /// Remote service is down or unreachable.
    UpstreamUnavailable,
    /// Remote call timed out.
    UpstreamTimeout,
    /// Input data invalid for the remote service (not action validation).
    InvalidInput,
    /// Usage quota exhausted (API call limit).
    QuotaExhausted,
    /// Action panicked during execution (caught by runtime).
    ActionPanicked,
}

/// Error type for all action operations.
///
/// Distinguishes retryable from fatal errors so the engine can decide
/// retry policy (backoff, max attempts, budget) without the action
/// needing to know about resilience patterns.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum ActionError {
    /// Transient failure — engine may retry based on its policy.
    ///
    /// The `backoff_hint` is a suggestion from the action; the engine
    /// may ignore it in favor of its own retry configuration.
    #[error("retryable: {error}")]
    Retryable {
        /// Full error chain wrapped in `Arc` for `Clone` support.
        error: Arc<anyhow::Error>,
        /// Machine-readable error code for engine decisions.
        code: Option<RetryHintCode>,
        /// Suggested delay before retry (engine may override).
        backoff_hint: Option<Duration>,
        /// Partial result produced before failure.
        partial_output: Option<serde_json::Value>,
    },

    /// Permanent failure — never retry.
    ///
    /// Invalid credentials, schema mismatch, business logic rejection.
    #[error("fatal: {error}")]
    Fatal {
        /// Full error chain wrapped in `Arc` for `Clone` support.
        error: Arc<anyhow::Error>,
        /// Machine-readable error code for engine decisions.
        code: Option<RetryHintCode>,
        /// Optional structured details about the failure.
        details: Option<serde_json::Value>,
    },

    /// Input validation failed before execution began.
    #[error("validation: {0}")]
    Validation(String),

    /// Action requested a capability it was not granted.
    #[error("sandbox violation: capability `{capability}` denied for action `{action_id}`")]
    SandboxViolation {
        /// The capability that was denied.
        capability: String,
        /// The action that requested the capability.
        action_id: String,
    },

    /// Execution cancelled via cancellation token.
    #[error("cancelled")]
    Cancelled,

    /// Output exceeds the configured data limit.
    #[error("data limit exceeded: {actual_bytes} bytes > {limit_bytes} bytes limit")]
    DataLimitExceeded {
        /// Maximum allowed output size in bytes.
        limit_bytes: u64,
        /// Actual output size in bytes.
        actual_bytes: u64,
    },
}

impl nebula_error::Classify for ActionError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::Retryable { .. } => nebula_error::ErrorCategory::External,
            Self::Fatal { .. } => nebula_error::ErrorCategory::Internal,
            Self::Validation(_) => nebula_error::ErrorCategory::Validation,
            Self::SandboxViolation { .. } => nebula_error::ErrorCategory::Authorization,
            Self::Cancelled => nebula_error::ErrorCategory::Cancelled,
            Self::DataLimitExceeded { .. } => nebula_error::ErrorCategory::Exhausted,
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new(match self {
            Self::Retryable { .. } => "ACTION:RETRYABLE",
            Self::Fatal { .. } => "ACTION:FATAL",
            Self::Validation(_) => "ACTION:VALIDATION",
            Self::SandboxViolation { .. } => "ACTION:SANDBOX_VIOLATION",
            Self::Cancelled => "ACTION:CANCELLED",
            Self::DataLimitExceeded { .. } => "ACTION:DATA_LIMIT",
        })
    }

    fn is_retryable(&self) -> bool {
        ActionError::is_retryable(self)
    }

    fn retry_hint(&self) -> Option<nebula_error::RetryHint> {
        self.backoff_hint().map(nebula_error::RetryHint::after)
    }
}

impl From<nebula_credential::CredentialAccessError> for ActionError {
    fn from(err: nebula_credential::CredentialAccessError) -> Self {
        match err {
            nebula_credential::CredentialAccessError::AccessDenied {
                capability,
                action_id,
            } => ActionError::SandboxViolation {
                capability,
                action_id,
            },
            other => ActionError::fatal_from(other),
        }
    }
}

impl ActionError {
    /// Create a retryable error with no backoff hint.
    ///
    /// Accepts any type that implements `Display + Debug + Send + Sync`.
    /// For typed errors, use [`Self::retryable_from`] to preserve the error chain.
    pub fn retryable(
        error: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
    ) -> Self {
        Self::Retryable {
            error: Arc::new(anyhow::anyhow!("{error}")),
            code: None,
            backoff_hint: None,
            partial_output: None,
        }
    }

    /// Create a retryable error from a typed error, preserving the full chain.
    pub fn retryable_from(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Retryable {
            error: Arc::new(error.into()),
            code: None,
            backoff_hint: None,
            partial_output: None,
        }
    }

    /// Create a retryable error with a suggested backoff duration.
    pub fn retryable_with_backoff(
        error: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
        backoff: Duration,
    ) -> Self {
        Self::Retryable {
            error: Arc::new(anyhow::anyhow!("{error}")),
            code: None,
            backoff_hint: Some(backoff),
            partial_output: None,
        }
    }

    /// Create a retryable error with a retry-strategy hint.
    pub fn retryable_with_hint(
        error: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
        hint: RetryHintCode,
    ) -> Self {
        Self::Retryable {
            error: Arc::new(anyhow::anyhow!("{error}")),
            code: Some(hint),
            backoff_hint: None,
            partial_output: None,
        }
    }

    /// Create a retryable error carrying a partial result.
    pub fn retryable_with_partial(
        error: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
        partial: serde_json::Value,
    ) -> Self {
        Self::Retryable {
            error: Arc::new(anyhow::anyhow!("{error}")),
            code: None,
            backoff_hint: None,
            partial_output: Some(partial),
        }
    }

    /// Create a fatal (non-retryable) error.
    ///
    /// Accepts any type that implements `Display + Debug + Send + Sync`.
    /// For typed errors, use [`Self::fatal_from`] to preserve the error chain.
    pub fn fatal(error: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static) -> Self {
        Self::Fatal {
            error: Arc::new(anyhow::anyhow!("{error}")),
            code: None,
            details: None,
        }
    }

    /// Create a fatal error from a typed error, preserving the full chain.
    pub fn fatal_from(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Fatal {
            error: Arc::new(error.into()),
            code: None,
            details: None,
        }
    }

    /// Create a fatal error with structured details.
    pub fn fatal_with_details(
        error: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
        details: serde_json::Value,
    ) -> Self {
        Self::Fatal {
            error: Arc::new(anyhow::anyhow!("{error}")),
            code: None,
            details: Some(details),
        }
    }

    /// Create a fatal error with a retry-strategy hint.
    ///
    /// Fatal errors are not retried, but the hint is preserved for
    /// observability and metrics (e.g., `QuotaExhausted` vs
    /// `InvalidInput` in an error dashboard).
    pub fn fatal_with_hint(
        error: impl std::fmt::Display + std::fmt::Debug + Send + Sync + 'static,
        hint: RetryHintCode,
    ) -> Self {
        Self::Fatal {
            error: Arc::new(anyhow::anyhow!("{error}")),
            code: Some(hint),
            details: None,
        }
    }

    /// Retry-strategy hint attached by the action body, if any.
    ///
    /// Returns `Some` only for `Retryable` and `Fatal` variants where
    /// the action explicitly attached a [`RetryHintCode`]. For the
    /// cross-crate classifier tag covering every variant, use
    /// `<ActionError as nebula_error::Classify>::code()`.
    ///
    /// Returned by value because [`RetryHintCode`] is `Copy`.
    pub fn retry_hint_code(&self) -> Option<RetryHintCode> {
        match self {
            Self::Retryable { code, .. } | Self::Fatal { code, .. } => *code,
            _ => None,
        }
    }

    /// Create a validation error.
    pub fn validation(msg: impl Into<String>) -> Self {
        Self::Validation(msg.into())
    }

    /// Returns `true` if the engine should consider retrying this error.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable { .. })
    }

    /// Returns `true` if this error is permanent and should never be retried.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::Fatal { .. }
                | Self::Validation(_)
                | Self::SandboxViolation { .. }
                | Self::DataLimitExceeded { .. }
        )
    }

    /// Extract the backoff hint, if present.
    pub fn backoff_hint(&self) -> Option<Duration> {
        match self {
            Self::Retryable { backoff_hint, .. } => *backoff_hint,
            _ => None,
        }
    }

    /// Extract the partial output, if present.
    pub fn partial_output(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Retryable { partial_output, .. } => partial_output.as_ref(),
            _ => None,
        }
    }
}

/// Extension trait for converting `Result<T, E>` into `Result<T, ActionError>`.
///
/// Provides ergonomic `.retryable()?` and `.fatal()?` conversions that
/// eliminate verbose `.map_err(|e| ActionError::retryable_from(e))` chains
/// in action bodies.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_action::prelude::*;
///
/// fn fetch_data() -> Result<String, ActionError> {
///     let value: i32 = "42".parse().fatal()?;
///     Ok(format!("got {value}"))
/// }
/// ```
pub trait ActionErrorExt<T> {
    /// Convert error to retryable [`ActionError`] (transient — engine may retry).
    ///
    /// Use for network errors, timeouts, and other transient failures where
    /// retrying the same operation may succeed.
    fn retryable(self) -> Result<T, ActionError>;

    /// Convert error to fatal [`ActionError`] (permanent — never retry).
    ///
    /// Use for validation errors, schema mismatches, and other permanent
    /// failures where retrying would produce the same error.
    fn fatal(self) -> Result<T, ActionError>;

    /// Convert error to retryable [`ActionError`] with a retry-strategy hint.
    ///
    /// The hint enables the engine to apply smarter retry strategies
    /// (e.g., refresh credentials on [`RetryHintCode::AuthExpired`]).
    fn retryable_with_hint(self, hint: RetryHintCode) -> Result<T, ActionError>;

    /// Convert error to fatal [`ActionError`] with a retry-strategy hint.
    ///
    /// Fatal errors are not retried, but the hint is preserved for
    /// observability (e.g., `QuotaExhausted` vs `InvalidInput`).
    fn fatal_with_hint(self, hint: RetryHintCode) -> Result<T, ActionError>;
}

impl<T, E> ActionErrorExt<T> for Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn retryable(self) -> Result<T, ActionError> {
        self.map_err(ActionError::retryable_from)
    }

    fn fatal(self) -> Result<T, ActionError> {
        self.map_err(ActionError::fatal_from)
    }

    fn retryable_with_hint(self, hint: RetryHintCode) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::retryable_with_hint(e, hint))
    }

    fn fatal_with_hint(self, hint: RetryHintCode) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::fatal_with_hint(e, hint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_error_is_retryable() {
        let err = ActionError::retryable("connection reset");
        assert!(err.is_retryable());
        assert!(!err.is_fatal());
        assert!(err.backoff_hint().is_none());
    }

    #[test]
    fn retryable_with_backoff_carries_hint() {
        let err = ActionError::retryable_with_backoff("rate limited", Duration::from_secs(5));
        assert!(err.is_retryable());
        assert_eq!(err.backoff_hint(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn retryable_with_partial_carries_output() {
        let partial = serde_json::json!({"processed": 3});
        let err = ActionError::retryable_with_partial("partial failure", partial.clone());
        assert!(err.is_retryable());
        assert_eq!(err.partial_output(), Some(&partial));
    }

    #[test]
    fn fatal_error_is_not_retryable() {
        let err = ActionError::fatal("invalid credentials");
        assert!(err.is_fatal());
        assert!(!err.is_retryable());
    }

    #[test]
    fn fatal_with_details() {
        let details = serde_json::json!({"field": "password"});
        let err = ActionError::fatal_with_details("auth failed", details.clone());
        match &err {
            ActionError::Fatal { details: d, .. } => assert_eq!(d, &Some(details)),
            _ => panic!("expected Fatal"),
        }
    }

    #[test]
    fn validation_error_is_fatal() {
        let err = ActionError::validation("email is required");
        assert!(err.is_fatal());
        assert!(!err.is_retryable());
    }

    #[test]
    fn sandbox_violation_is_fatal() {
        let err = ActionError::SandboxViolation {
            capability: "Network".into(),
            action_id: "custom.action".into(),
        };
        assert!(err.is_fatal());
        assert!(!err.is_retryable());
    }

    #[test]
    fn cancelled_is_neither_retryable_nor_fatal() {
        let err = ActionError::Cancelled;
        assert!(!err.is_retryable());
        // Cancelled is special — not retryable, not "fatal" in the business sense
        assert!(!err.is_fatal());
    }

    #[test]
    fn data_limit_exceeded_is_fatal() {
        let err = ActionError::DataLimitExceeded {
            limit_bytes: 1_000_000,
            actual_bytes: 5_000_000,
        };
        assert!(err.is_fatal());
    }

    #[test]
    fn retry_hint_code_serializes_to_string() {
        let hint = RetryHintCode::RateLimited;
        let json = serde_json::to_string(&hint).unwrap();
        assert_eq!(json, "\"RateLimited\"");
    }

    #[test]
    fn retry_hint_code_deserializes_from_string() {
        let hint: RetryHintCode = serde_json::from_str("\"AuthExpired\"").unwrap();
        assert_eq!(hint, RetryHintCode::AuthExpired);
    }

    #[test]
    fn retry_hint_code_is_copy() {
        let hint = RetryHintCode::RateLimited;
        let copy = hint;
        assert_eq!(hint, copy); // both still valid — Copy
    }

    #[test]
    fn retry_hint_code_debug_format() {
        assert_eq!(
            format!("{:?}", RetryHintCode::UpstreamTimeout),
            "UpstreamTimeout"
        );
    }

    #[test]
    fn display_formatting() {
        let err = ActionError::retryable("timeout");
        assert_eq!(err.to_string(), "retryable: timeout");

        let err = ActionError::fatal("bad schema");
        assert_eq!(err.to_string(), "fatal: bad schema");

        let err = ActionError::validation("missing field");
        assert_eq!(err.to_string(), "validation: missing field");

        let err = ActionError::Cancelled;
        assert_eq!(err.to_string(), "cancelled");
    }

    #[test]
    fn retryable_with_hint_attaches_hint() {
        let err = ActionError::retryable_with_hint("rate limited", RetryHintCode::RateLimited);
        assert_eq!(err.retry_hint_code(), Some(RetryHintCode::RateLimited));
        assert!(err.is_retryable());
    }

    #[test]
    fn fatal_with_hint_attaches_hint() {
        let err = ActionError::fatal_with_hint("expired", RetryHintCode::AuthExpired);
        assert_eq!(err.retry_hint_code(), Some(RetryHintCode::AuthExpired));
        assert!(err.is_fatal());
    }

    #[test]
    fn retryable_from_preserves_error_chain() {
        let io_err = std::io::Error::new(std::io::ErrorKind::TimedOut, "timeout");
        let err = ActionError::retryable_from(io_err);
        assert!(err.to_string().contains("timeout"));
        assert!(err.is_retryable());
    }

    #[test]
    fn clone_preserves_error() {
        let err = ActionError::retryable("test");
        let cloned = err.clone();
        assert_eq!(err.to_string(), cloned.to_string());
    }

    #[test]
    fn retry_hint_code_is_none_when_not_supplied() {
        let err = ActionError::retryable("no hint");
        assert!(err.retry_hint_code().is_none());
    }

    #[test]
    fn retry_hint_code_is_none_for_non_retryable_fatal_variants() {
        // Variants other than Retryable/Fatal never carry a user hint —
        // use Classify::code() for the framework tag instead.
        assert!(ActionError::validation("x").retry_hint_code().is_none());
        assert!(ActionError::Cancelled.retry_hint_code().is_none());
        assert!(
            ActionError::DataLimitExceeded {
                limit_bytes: 1,
                actual_bytes: 2,
            }
            .retry_hint_code()
            .is_none()
        );
    }

    // ── ActionErrorExt ──────────────────────────────────────────────────────

    #[test]
    fn ext_retryable_converts_io_error() {
        let result: Result<(), std::io::Error> = Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "connection refused",
        ));
        let err = result.retryable().unwrap_err();
        assert!(err.is_retryable());
        assert!(err.to_string().contains("connection refused"));
    }

    #[test]
    fn ext_fatal_converts_io_error() {
        let result: Result<(), std::io::Error> = Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "corrupt",
        ));
        let err = result.fatal().unwrap_err();
        assert!(err.is_fatal());
    }

    #[test]
    fn ext_retryable_with_hint_sets_hint() {
        let result: Result<i32, std::io::Error> = Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "rate limited",
        ));
        let err = result
            .retryable_with_hint(RetryHintCode::RateLimited)
            .unwrap_err();
        assert_eq!(err.retry_hint_code(), Some(RetryHintCode::RateLimited));
        assert!(err.is_retryable());
    }

    #[test]
    fn ext_fatal_with_hint_sets_hint() {
        let result: Result<i32, std::io::Error> =
            Err(std::io::Error::new(std::io::ErrorKind::Other, "expired"));
        let err = result
            .fatal_with_hint(RetryHintCode::AuthExpired)
            .unwrap_err();
        assert_eq!(err.retry_hint_code(), Some(RetryHintCode::AuthExpired));
        assert!(err.is_fatal());
    }

    #[test]
    fn ext_ok_passes_through() {
        let result: Result<i32, std::io::Error> = Ok(42);
        assert_eq!(result.retryable().unwrap(), 42);
    }

    #[test]
    fn ext_chaining_preserves_error_chain() {
        fn do_io() -> Result<Vec<u8>, std::io::Error> {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))
        }
        fn do_work() -> Result<String, ActionError> {
            let _data = do_io().retryable()?;
            Ok("ok".into())
        }
        let err = do_work().unwrap_err();
        assert!(err.is_retryable());
        assert!(err.to_string().contains("missing"));
    }
}
