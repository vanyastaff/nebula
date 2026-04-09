//! Execution events emitted by the engine during workflow execution.
//!
//! Subscribe via `WorkflowEngine::with_event_sender` to receive real-time
//! updates about node lifecycle transitions. Used by the CLI TUI for live
//! execution monitoring.

use std::time::Duration;

use nebula_core::id::{ExecutionId, NodeId};

/// Events emitted during workflow execution.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ExecutionEvent {
    /// A node started executing.
    NodeStarted {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that started.
        node_id: NodeId,
        /// Action key being executed.
        action_key: String,
    },

    /// A node completed successfully.
    NodeCompleted {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that completed.
        node_id: NodeId,
        /// How long the node took.
        elapsed: Duration,
    },

    /// A node failed.
    NodeFailed {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that failed.
        node_id: NodeId,
        /// Error message.
        error: String,
    },

    /// A node was skipped (disabled or dependency not met).
    NodeSkipped {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that was skipped.
        node_id: NodeId,
    },

    /// Workflow execution completed.
    ExecutionFinished {
        /// The execution that finished.
        execution_id: ExecutionId,
        /// Whether it succeeded.
        success: bool,
        /// Total elapsed time.
        elapsed: Duration,
    },
}
