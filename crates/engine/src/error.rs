//! Engine error types.

use nebula_core::id::{ActionId, NodeId};

/// Errors from the engine layer.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// No action key mapping for the given action ID.
    #[error("no action key mapping for action_id {action_id}")]
    ActionKeyNotFound {
        /// The action ID that could not be resolved.
        action_id: ActionId,
    },

    /// A referenced node was not found in the workflow.
    #[error("node not found: {node_id}")]
    NodeNotFound {
        /// The missing node ID.
        node_id: NodeId,
    },

    /// Execution planning failed.
    #[error("planning failed: {0}")]
    PlanningFailed(String),

    /// A node failed during execution.
    #[error("node {node_id} failed: {error}")]
    NodeFailed {
        /// The node that failed.
        node_id: NodeId,
        /// The error message.
        error: String,
    },

    /// The execution was cancelled.
    #[error("execution cancelled")]
    Cancelled,

    /// A budget limit was exceeded.
    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),

    /// Error from the runtime layer.
    #[error("runtime error: {0}")]
    Runtime(#[from] nebula_runtime::RuntimeError),

    /// Error from the execution state layer.
    #[error("execution error: {0}")]
    Execution(#[from] nebula_execution::ExecutionError),

    /// A task panicked during execution.
    #[error("task panicked: {0}")]
    TaskPanicked(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_failed_display() {
        let err = EngineError::PlanningFailed("no nodes".into());
        assert_eq!(err.to_string(), "planning failed: no nodes");
    }

    #[test]
    fn cancelled_display() {
        let err = EngineError::Cancelled;
        assert_eq!(err.to_string(), "execution cancelled");
    }

    #[test]
    fn budget_exceeded_display() {
        let err = EngineError::BudgetExceeded("max retries".into());
        assert_eq!(err.to_string(), "budget exceeded: max retries");
    }

    #[test]
    fn node_failed_display() {
        let node_id = NodeId::v4();
        let err = EngineError::NodeFailed {
            node_id,
            error: "timeout".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("timeout"));
        assert!(msg.contains("failed"));
    }
}
