//! Action execution context.

use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
use tokio_util::sync::CancellationToken;

use crate::error::ActionError;

/// Minimal runtime context provided to actions during execution.
///
/// Provides identity information and cancellation support.
/// Actions **must** periodically call [`check_cancelled`](Self::check_cancelled)
/// in long-running loops to support cooperative cancellation.
#[non_exhaustive]
pub struct ActionContext {
    /// Unique execution run identifier.
    pub execution_id: ExecutionId,
    /// Node in the workflow graph being executed.
    pub node_id: NodeId,
    /// Workflow this execution belongs to.
    pub workflow_id: WorkflowId,
    /// Cancellation signal — checked cooperatively by actions.
    pub cancellation: CancellationToken,
}

impl ActionContext {
    /// Create a new context with the given identifiers.
    pub fn new(execution_id: ExecutionId, node_id: NodeId, workflow_id: WorkflowId) -> Self {
        Self {
            execution_id,
            node_id,
            workflow_id,
            cancellation: CancellationToken::new(),
        }
    }

    /// Create a context with a pre-existing cancellation token.
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancellation = token;
        self
    }

    /// Check whether execution has been cancelled.
    ///
    /// Actions **should** call this in loops and before expensive operations
    /// to support cooperative cancellation.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError::Cancelled`] if the token has been triggered.
    pub fn check_cancelled(&self) -> Result<(), ActionError> {
        if self.cancellation.is_cancelled() {
            Err(ActionError::Cancelled)
        } else {
            Ok(())
        }
    }
}

impl std::fmt::Debug for ActionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionContext")
            .field("execution_id", &self.execution_id)
            .field("node_id", &self.node_id)
            .field("workflow_id", &self.workflow_id)
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> ActionContext {
        ActionContext::new(ExecutionId::v4(), NodeId::v4(), WorkflowId::v4())
    }

    #[test]
    fn check_cancelled_ok() {
        let ctx = test_context();
        assert!(ctx.check_cancelled().is_ok());
    }

    #[test]
    fn check_cancelled_after_cancel() {
        let ctx = test_context();
        ctx.cancellation.cancel();
        let err = ctx.check_cancelled().unwrap_err();
        assert!(matches!(err, ActionError::Cancelled));
    }

    #[test]
    fn with_cancellation_token() {
        let token = CancellationToken::new();
        let child = token.child_token();
        let ctx = test_context().with_cancellation(child);
        assert!(ctx.check_cancelled().is_ok());
        token.cancel();
        assert!(ctx.check_cancelled().is_err());
    }

    #[test]
    fn debug_format() {
        let ctx = test_context();
        let debug = format!("{ctx:?}");
        assert!(debug.contains("ActionContext"));
        assert!(debug.contains("execution_id"));
    }
}
