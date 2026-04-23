//! Runtime error types.

/// Errors from the runtime layer.
#[derive(Debug, thiserror::Error, nebula_error::Classify)]
pub enum RuntimeError {
    /// Action not found in the registry.
    #[classify(category = "not_found", code = "RUNTIME:ACTION_NOT_FOUND")]
    #[error("action not found: {key}")]
    ActionNotFound {
        /// The action key that was looked up.
        key: String,
    },

    /// The action key string failed to parse into a valid `ActionKey`.
    #[classify(
        category = "validation",
        code = "RUNTIME:INVALID_ACTION_KEY",
        retryable = false
    )]
    #[error("invalid action key '{key}': {reason}")]
    InvalidActionKey {
        /// The raw key string that failed to parse.
        key: String,
        /// The parse error reason.
        reason: String,
    },

    /// Action execution failed.
    #[classify(
        category = "external",
        code = "RUNTIME:ACTION_ERROR",
        retryable = false
    )]
    #[error("action error: {0}")]
    ActionError(#[from] nebula_action::ActionError),

    /// Data limit exceeded.
    #[classify(category = "exhausted", code = "RUNTIME:DATA_LIMIT", retryable = false)]
    #[error("data limit exceeded: {actual_bytes} bytes > {limit_bytes} bytes")]
    DataLimitExceeded {
        /// Maximum allowed output size.
        limit_bytes: u64,
        /// Actual output size.
        actual_bytes: u64,
    },

    /// The action key resolves to a trigger, which has its own start/stop
    /// lifecycle and is not executable via `ActionRuntime::execute_action`.
    /// Triggers run via the trigger runtime (separate from action execution).
    #[classify(
        category = "unsupported",
        code = "RUNTIME:TRIGGER_NOT_EXECUTABLE",
        retryable = false
    )]
    #[error("trigger '{key}' is not executable via ActionRuntime — use the trigger runtime")]
    TriggerNotExecutable {
        /// The action key that was looked up.
        key: String,
    },

    /// The action key resolves to a resource, which has its own
    /// configure/cleanup lifecycle scoped to a downstream subtree.
    /// Resources are not executable via `ActionRuntime::execute_action`.
    #[classify(
        category = "unsupported",
        code = "RUNTIME:RESOURCE_NOT_EXECUTABLE",
        retryable = false
    )]
    #[error("resource '{key}' is not executable via ActionRuntime — use the resource graph")]
    ResourceNotExecutable {
        /// The action key that was looked up.
        key: String,
    },

    /// Internal runtime error.
    #[classify(category = "internal", code = "RUNTIME:INTERNAL")]
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
    use nebula_action::ActionError;

    use super::*;

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
