//! Runtime error types.

/// Errors from the runtime layer.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// Action not found in the registry.
    #[error("action not found: {key}")]
    ActionNotFound {
        /// The action key that was looked up.
        key: String,
    },

    /// Action execution failed.
    #[error("action error: {0}")]
    ActionError(#[from] nebula_action::ActionError),

    /// Data limit exceeded.
    #[error("data limit exceeded: {actual_bytes} bytes > {limit_bytes} bytes")]
    DataLimitExceeded {
        /// Maximum allowed output size.
        limit_bytes: u64,
        /// Actual output size.
        actual_bytes: u64,
    },

    /// Internal runtime error.
    #[error("runtime error: {0}")]
    Internal(String),
}

impl RuntimeError {
    /// Whether this error originated from a retryable action error.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::ActionError(e) => e.is_retryable(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::ActionError;

    #[test]
    fn action_not_found_display() {
        let err = RuntimeError::ActionNotFound {
            key: "http.request".into(),
        };
        assert_eq!(err.to_string(), "action not found: http.request");
    }

    #[test]
    fn retryable_propagation() {
        let err = RuntimeError::ActionError(ActionError::retryable("timeout"));
        assert!(err.is_retryable());

        let err = RuntimeError::ActionError(ActionError::fatal("bad schema"));
        assert!(!err.is_retryable());
    }

    #[test]
    fn data_limit_not_retryable() {
        let err = RuntimeError::DataLimitExceeded {
            limit_bytes: 1_000,
            actual_bytes: 5_000,
        };
        assert!(!err.is_retryable());
    }
}
