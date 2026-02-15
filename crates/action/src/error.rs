use std::time::Duration;

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
        /// Human-readable error message.
        error: String,
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
        /// Human-readable error message.
        error: String,
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

impl ActionError {
    /// Create a retryable error with no backoff hint.
    pub fn retryable(msg: impl Into<String>) -> Self {
        Self::Retryable {
            error: msg.into(),
            backoff_hint: None,
            partial_output: None,
        }
    }

    /// Create a retryable error with a suggested backoff duration.
    pub fn retryable_with_backoff(msg: impl Into<String>, backoff: Duration) -> Self {
        Self::Retryable {
            error: msg.into(),
            backoff_hint: Some(backoff),
            partial_output: None,
        }
    }

    /// Create a retryable error carrying a partial result.
    pub fn retryable_with_partial(msg: impl Into<String>, partial: serde_json::Value) -> Self {
        Self::Retryable {
            error: msg.into(),
            backoff_hint: None,
            partial_output: Some(partial),
        }
    }

    /// Create a fatal (non-retryable) error.
    pub fn fatal(msg: impl Into<String>) -> Self {
        Self::Fatal {
            error: msg.into(),
            details: None,
        }
    }

    /// Create a fatal error with structured details.
    pub fn fatal_with_details(msg: impl Into<String>, details: serde_json::Value) -> Self {
        Self::Fatal {
            error: msg.into(),
            details: Some(details),
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
}
