//! Execution error types.

use nebula_core::NodeId;
use thiserror::Error;

use crate::status::ExecutionStatus;

/// Errors that can occur during workflow execution.
#[derive(Debug, Error)]
pub enum ExecutionError {
    /// A state transition is not valid for the current status.
    #[error("invalid transition from {from} to {to}")]
    InvalidTransition {
        /// Current status.
        from: String,
        /// Attempted target status.
        to: String,
    },

    /// A referenced node does not exist in the execution state.
    #[error("node not found: {0}")]
    NodeNotFound(NodeId),

    /// The execution plan failed validation.
    #[error("plan validation: {0}")]
    PlanValidation(String),

    /// A budget limit was exceeded.
    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),

    /// An idempotency key has already been used.
    #[error("duplicate idempotency key: {0}")]
    DuplicateIdempotencyKey(String),

    /// A serialization or deserialization error.
    #[error("serialization: {0}")]
    Serialization(#[from] serde_json::Error),

    /// The execution was cancelled.
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
        let id = NodeId::v4();
        let err = ExecutionError::NodeNotFound(id);
        assert!(err.to_string().contains("node not found"));
    }

    #[test]
    fn plan_validation_display() {
        let err = ExecutionError::PlanValidation("no nodes in workflow".into());
        assert_eq!(err.to_string(), "plan validation: no nodes in workflow");
    }

    #[test]
    fn from_serde_error() {
        let serde_err = serde_json::from_str::<String>("not valid json").unwrap_err();
        let err = ExecutionError::from(serde_err);
        assert!(err.to_string().starts_with("serialization:"));
    }

    #[test]
    fn cancelled_display() {
        let err = ExecutionError::Cancelled;
        assert_eq!(err.to_string(), "execution cancelled");
    }
}
