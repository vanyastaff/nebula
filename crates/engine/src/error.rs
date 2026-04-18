//! Engine error types.

use nebula_action::ActionError;
use nebula_core::{NodeKey, id::ExecutionId};
use nebula_workflow::NodeState;

/// Errors from the engine layer.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// A referenced node was not found in the workflow.
    #[error("node not found: {node_key}")]
    NodeNotFound {
        /// The missing node ID.
        node_key: NodeKey,
    },

    /// Execution planning failed.
    #[error("planning failed: {0}")]
    PlanningFailed(String),

    /// A node failed during execution.
    #[error("node {node_key} failed: {error}")]
    NodeFailed {
        /// The node that failed.
        node_key: NodeKey,
        /// The error message.
        error: String,
    },

    /// The execution was cancelled.
    #[error("execution cancelled")]
    Cancelled,

    /// Parameter resolution failed (expression eval, reference lookup, etc.)
    #[error("parameter resolution failed for node {node_key}, param '{param_key}': {error}")]
    ParameterResolution {
        /// The node whose parameter could not be resolved.
        node_key: NodeKey,
        /// The parameter key that failed.
        param_key: String,
        /// The underlying error.
        error: String,
    },

    /// Parameter validation failed against the action's schema.
    #[error("parameter validation failed for node {node_key}: {errors}")]
    ParameterValidation {
        /// The node whose parameters failed validation.
        node_key: NodeKey,
        /// Combined validation error messages.
        errors: String,
    },

    /// Edge condition evaluation failed.
    #[error("edge evaluation failed from {from_node} to {to_node}: {error}")]
    EdgeEvaluationFailed {
        /// Source node of the edge.
        from_node: NodeKey,
        /// Target node of the edge.
        to_node: NodeKey,
        /// The underlying error.
        error: String,
    },

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

    /// A typed [`ActionError`] bubbled up from the action/dispatch layer.
    ///
    /// Used by the engine's pre-dispatch pipeline (e.g. proactive
    /// credential refresh) to surface typed errors through the normal
    /// `ErrorStrategy` decision path instead of logging-and-continuing.
    /// Downstream consumers can match on the inner variant to distinguish
    /// `CredentialRefreshFailed` from other failure modes.
    #[error("action failed: {0}")]
    Action(#[from] ActionError),

    /// The frontier loop exited while one or more nodes were still in a
    /// non-terminal state (e.g. `Pending` / `Running` / `Retrying`).
    ///
    /// Per `docs/PRODUCT_CANON.md` §11.1, the engine must be the single source
    /// of truth for execution status and must not silently report `Completed`
    /// on inconsistent state. This variant is produced when the frontier
    /// drains without `failed_node` or cancellation, yet `all_nodes_terminal`
    /// is false — almost always a scheduler bookkeeping bug.
    #[error(
        "frontier integrity violation: execution {execution_id} exited with \
         {} non-terminal node(s)",
        non_terminal_nodes.len()
    )]
    FrontierIntegrity {
        /// The execution whose frontier loop produced the inconsistent state.
        execution_id: ExecutionId,
        /// Nodes that were still non-terminal at the time the frontier
        /// loop exited, paired with their observed `NodeState`.
        non_terminal_nodes: Vec<(NodeKey, NodeState)>,
    },
}

impl nebula_error::Classify for EngineError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::NodeNotFound { .. } => nebula_error::ErrorCategory::NotFound,
            Self::PlanningFailed(_)
            | Self::ParameterResolution { .. }
            | Self::ParameterValidation { .. }
            | Self::EdgeEvaluationFailed { .. } => nebula_error::ErrorCategory::Validation,
            Self::NodeFailed { .. } | Self::TaskPanicked(_) | Self::FrontierIntegrity { .. } => {
                nebula_error::ErrorCategory::Internal
            },
            Self::Cancelled => nebula_error::ErrorCategory::Cancelled,
            Self::BudgetExceeded(_) => nebula_error::ErrorCategory::Exhausted,
            Self::Runtime(e) => nebula_error::Classify::category(e),
            Self::Execution(e) => nebula_error::Classify::category(e),
            Self::Action(e) => nebula_error::Classify::category(e),
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new(match self {
            Self::NodeNotFound { .. } => "ENGINE:NODE_NOT_FOUND",
            Self::PlanningFailed(_) => "ENGINE:PLANNING_FAILED",
            Self::NodeFailed { .. } => "ENGINE:NODE_FAILED",
            Self::Cancelled => "ENGINE:CANCELLED",
            Self::ParameterResolution { .. } => "ENGINE:PARAM_RESOLUTION",
            Self::ParameterValidation { .. } => "ENGINE:PARAM_VALIDATION",
            Self::EdgeEvaluationFailed { .. } => "ENGINE:EDGE_EVAL",
            Self::BudgetExceeded(_) => "ENGINE:BUDGET_EXCEEDED",
            Self::Runtime(e) => return nebula_error::Classify::code(e),
            Self::Execution(e) => return nebula_error::Classify::code(e),
            Self::Action(e) => return nebula_error::Classify::code(e),
            Self::TaskPanicked(_) => "ENGINE:TASK_PANICKED",
            Self::FrontierIntegrity { .. } => "ENGINE:FRONTIER_INTEGRITY",
        })
    }

    fn is_retryable(&self) -> bool {
        match self {
            Self::Runtime(e) => nebula_error::Classify::is_retryable(e),
            Self::Execution(e) => nebula_error::Classify::is_retryable(e),
            Self::Action(e) => nebula_error::Classify::is_retryable(e),
            _ => self.category().is_default_retryable(),
        }
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;

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
        let node_key = node_key!("test_node");
        let err = EngineError::NodeFailed {
            node_key,
            error: "timeout".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("timeout"));
        assert!(msg.contains("failed"));
    }

    #[test]
    fn frontier_integrity_display_and_classification() {
        use nebula_core::id::ExecutionId;
        use nebula_error::{Classify, ErrorCategory};

        let exec_id = ExecutionId::new();
        let err = EngineError::FrontierIntegrity {
            execution_id: exec_id,
            non_terminal_nodes: vec![
                (node_key!("a"), NodeState::Pending),
                (node_key!("b"), NodeState::Running),
            ],
        };

        let msg = err.to_string();
        assert!(msg.contains("frontier integrity violation"));
        assert!(msg.contains("2 non-terminal"));
        assert!(msg.contains(&exec_id.to_string()));

        assert_eq!(Classify::category(&err), ErrorCategory::Internal);
        assert_eq!(
            Classify::code(&err).as_str(),
            "ENGINE:FRONTIER_INTEGRITY",
            "stable error code for operators / dashboards"
        );
    }
}
