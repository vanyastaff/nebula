//! Execution events emitted by the engine during workflow execution.
//!
//! Subscribe via `WorkflowEngine::with_event_sender` to receive real-time
//! updates about node lifecycle transitions. Used by the CLI TUI for live
//! execution monitoring.

use std::time::Duration;

use nebula_core::{NodeKey, id::ExecutionId};

/// Events emitted during workflow execution.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ExecutionEvent {
    /// A node started executing.
    NodeStarted {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that started.
        node_key: NodeKey,
        /// Action key being executed.
        action_key: String,
    },

    /// A node completed successfully.
    NodeCompleted {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that completed.
        node_key: NodeKey,
        /// How long the node took.
        elapsed: Duration,
    },

    /// A node failed.
    NodeFailed {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that failed.
        node_key: NodeKey,
        /// Error message.
        error: String,
    },

    /// A node was skipped (disabled or dependency not met).
    NodeSkipped {
        /// Execution this node belongs to.
        execution_id: ExecutionId,
        /// The node that was skipped.
        node_key: NodeKey,
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
