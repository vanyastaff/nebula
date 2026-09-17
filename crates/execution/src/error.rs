//! Execution error types.

use nebula_core::NodeKey;
use thiserror::Error;

use crate::status::ExecutionStatus;

/// Errors that can occur during workflow execution.
#[derive(Debug, Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum ExecutionError {
    /// A state transition is not valid for the current status.
    #[classify(category = "validation", code = "EXECUTION:INVALID_TRANSITION")]
    #[error("invalid transition from {from} to {to}")]
    InvalidTransition {
        /// Current status.
        from: String,
        /// Attempted target status.
        to: String,
    },

    /// A referenced node does not exist in the execution state.
    #[classify(category = "not_found", code = "EXECUTION:NODE_NOT_FOUND")]
    #[error("node not found: {0}")]
    NodeNotFound(NodeKey),

    /// The execution plan failed validation.
    #[classify(category = "validation", code = "EXECUTION:PLAN_VALIDATION")]
    #[error("plan validation: {0}")]
    PlanValidation(String),

    /// A budget limit was exceeded.
    #[classify(category = "exhausted", code = "EXECUTION:BUDGET_EXCEEDED")]
    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),

    /// An idempotency key has already been used.
    #[classify(category = "conflict", code = "EXECUTION:DUPLICATE_KEY")]
    #[error("duplicate idempotency key: {0}")]
    DuplicateIdempotencyKey(String),

    /// A serialization or deserialization error.
    ///
    /// The inner `serde_json::Error` is kept as the typed `#[source]`/`#[from]`
    /// cause but is deliberately absent from `Display`: a raw decode error's
    /// `Display` quotes the value it choked on, and the engine's
    /// `durable_error_envelope` persists an `EngineError`'s top-level
    /// `Display` without walking `.source()` — so an interpolated value here
    /// would reach `executions.state` and the journal the same way the
    /// #1016 envelope exists to prevent. No producer constructs this variant
    /// today, but the type must not carry that footgun for the next one that does.
    #[classify(category = "internal", code = "EXECUTION:SERIALIZATION")]
    #[error("serialization failed")]
    Serialization(#[from] serde_json::Error),

    /// The execution was cancelled.
    #[classify(category = "cancelled", code = "EXECUTION:CANCELLED")]
    #[error("execution cancelled")]
    Cancelled,
}

impl ExecutionError {
    /// Create an invalid-transition error from execution statuses.
    pub fn invalid_execution_transition(from: ExecutionStatus, to: ExecutionStatus) -> Self {
        Self::InvalidTransition {
            from: from.to_string(),
            to: to.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;

    use super::*;

    #[test]
    fn invalid_transition_display() {
        let err = ExecutionError::InvalidTransition {
            from: "running".into(),
            to: "created".into(),
        };
        assert_eq!(
            err.to_string(),
            "invalid transition from running to created"
        );
    }

    #[test]
    fn node_not_found_display() {
        let id = node_key!("test");
        let err = ExecutionError::NodeNotFound(id);
        assert!(err.to_string().contains("node not found"));
    }

    #[test]
    fn plan_validation_display() {
        let err = ExecutionError::PlanValidation("no nodes in workflow".into());
        assert_eq!(err.to_string(), "plan validation: no nodes in workflow");
    }

    /// The decode error's own text is not published: only the framework-
    /// authored constant phrase reaches `Display`, and the marker proves the
    /// premise that a raw `serde_json::Error` would otherwise quote it.
    #[test]
    fn from_serde_error_does_not_publish_the_decoders_own_text() {
        const MARKER: &str = "MARKER-9f3a-secret";
        let serde_err = serde_json::from_value::<bool>(serde_json::json!(MARKER))
            .expect_err("a string is not a bool");
        assert!(
            serde_err.to_string().contains(MARKER),
            "premise: the raw decode error quotes the source text: {serde_err}"
        );

        let err = ExecutionError::from(serde_err);
        assert_eq!(err.to_string(), "serialization failed");
        assert!(!err.to_string().contains(MARKER), "{err}");

        // The typed cause still chains, for anything that walks `.source()`.
        assert!(std::error::Error::source(&err).is_some());
    }

    #[test]
    fn cancelled_display() {
        let err = ExecutionError::Cancelled;
        assert_eq!(err.to_string(), "execution cancelled");
    }
}
