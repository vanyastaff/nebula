//! Execution context traits.
//!
//! [`Context`] is the base trait. Concrete implementations will be added
//! when the design is refined.

use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
use tokio_util::sync::CancellationToken;

/// Base trait for execution contexts.
///
/// Implemented by context types to allow polymorphism and testability
/// (e.g. mock contexts in tests).
pub trait Context: Send + Sync {}

/// Temporary node context (bridge until design is refined).
#[doc(hidden)]
pub struct NodeContext {
    pub execution_id: ExecutionId,
    pub node_id: NodeId,
    pub workflow_id: WorkflowId,
    pub cancellation: CancellationToken,
}

impl Context for NodeContext {}

impl NodeContext {
    #[doc(hidden)]
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

#[cfg(test)]
mod tests {
    use super::*;

    struct MockContext;
    impl Context for MockContext {}

    #[test]
    fn context_trait_object_safety() {
        let ctx = MockContext;
        let _: &dyn Context = &ctx;
    }
}
