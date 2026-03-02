//! Execution context types and traits.
//!
//! [`Context`] is the base trait for action execution. [`ActionContext`] is the
//! stable context for StatelessAction/StatefulAction/ResourceAction;
//! [`TriggerContext`] is used by TriggerAction. [`NodeContext`] is deprecated
//! in favor of ActionContext.

use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
use tokio_util::sync::CancellationToken;

/// Base trait for action execution contexts.
///
/// Implemented by [`ActionContext`] and (for backward compatibility) [`NodeContext`].
/// Engine/runtime/sandbox provide concrete implementations; actions receive `&impl Context`.
pub trait Context: Send + Sync {
    /// Execution identity.
    fn execution_id(&self) -> ExecutionId;
    /// Node identity within the workflow.
    fn node_id(&self) -> NodeId;
    /// Workflow identity.
    fn workflow_id(&self) -> WorkflowId;
    /// Cancellation token; action may check before or during work.
    fn cancellation(&self) -> &CancellationToken;
}

/// Stable execution context for StatelessAction, StatefulAction, and ResourceAction.
///
/// Composes execution identity and cancellation. Capability modules (resources,
/// credentials, logger) can be added as fields by the runtime/sandbox without
/// changing this crate's API.
#[derive(Debug, Clone)]
pub struct ActionContext {
    /// Execution identity.
    pub execution_id: ExecutionId,
    /// Node identity within the workflow.
    pub node_id: NodeId,
    /// Workflow identity.
    pub workflow_id: WorkflowId,
    /// Cancellation token.
    pub cancellation: CancellationToken,
}

impl Context for ActionContext {
    fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }
    fn node_id(&self) -> NodeId {
        self.node_id
    }
    fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }
    fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

impl ActionContext {
    /// Create a new action context.
    #[must_use]
    pub fn new(
        execution_id: ExecutionId,
        node_id: NodeId,
        workflow_id: WorkflowId,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            execution_id,
            node_id,
            workflow_id,
            cancellation,
        }
    }
}

/// Context for TriggerAction (workflow starters).
///
/// Triggers live outside a specific execution; they start new executions.
/// Composes workflow/trigger identity and cancellation; scheduler and emitter
/// are provided by runtime.
#[derive(Debug, Clone)]
pub struct TriggerContext {
    /// Workflow this trigger belongs to.
    pub workflow_id: WorkflowId,
    /// Trigger (node) identity.
    pub trigger_id: NodeId,
    /// Cancellation token.
    pub cancellation: CancellationToken,
}

impl TriggerContext {
    /// Create a new trigger context.
    #[must_use]
    pub fn new(
        workflow_id: WorkflowId,
        trigger_id: NodeId,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            workflow_id,
            trigger_id,
            cancellation,
        }
    }
}

/// Temporary node context (bridge); use [`ActionContext`] instead.
///
/// Preserved for backward compatibility during migration.
#[deprecated(since = "0.1.0", note = "Use ActionContext instead")]
pub struct NodeContext {
    /// Execution identity.
    pub execution_id: ExecutionId,
    /// Node identity.
    pub node_id: NodeId,
    /// Workflow identity.
    pub workflow_id: WorkflowId,
    /// Cancellation token.
    pub cancellation: CancellationToken,
}

impl Context for NodeContext {
    fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }
    fn node_id(&self) -> NodeId {
        self.node_id
    }
    fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }
    fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

impl NodeContext {
    /// Create a new node context (deprecated: use [`ActionContext::new`]).
    #[must_use]
    #[deprecated(since = "0.1.0", note = "Use ActionContext::new instead")]
    pub fn new(
        execution_id: ExecutionId,
        node_id: NodeId,
        workflow_id: WorkflowId,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            execution_id,
            node_id,
            workflow_id,
            cancellation,
        }
    }
}

impl From<NodeContext> for ActionContext {
    fn from(n: NodeContext) -> Self {
        Self::new(n.execution_id, n.node_id, n.workflow_id, n.cancellation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockContext {
        token: CancellationToken,
    }
    impl Default for MockContext {
        fn default() -> Self {
            Self {
                token: CancellationToken::new(),
            }
        }
    }
    impl Context for MockContext {
        fn execution_id(&self) -> ExecutionId {
            ExecutionId::nil()
        }
        fn node_id(&self) -> NodeId {
            NodeId::nil()
        }
        fn workflow_id(&self) -> WorkflowId {
            WorkflowId::nil()
        }
        fn cancellation(&self) -> &CancellationToken {
            &self.token
        }
    }

    #[test]
    fn context_trait_object_safety() {
        let ctx = MockContext::default();
        let _: &dyn Context = &ctx;
    }

    #[test]
    #[allow(deprecated)]
    fn action_context_from_node_context() {
        let token = CancellationToken::new();
        let exec_id = ExecutionId::new();
        let node_id = NodeId::new();
        let wf_id = WorkflowId::new();
        let node = NodeContext::new(exec_id, node_id, wf_id, token);
        let action_ctx = ActionContext::from(node);
        assert_eq!(action_ctx.execution_id(), exec_id);
        assert_eq!(action_ctx.node_id(), node_id);
        assert_eq!(action_ctx.workflow_id(), wf_id);
    }
}
