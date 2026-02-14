//! Runtime execution context (non-serializable).

use std::collections::HashMap;
use std::sync::Arc;

use nebula_action::ExecutionBudget;
use nebula_core::{ExecutionId, NodeId};
use nebula_workflow::WorkflowDefinition;
use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;

use crate::error::ExecutionError;
use crate::output::NodeOutput;

/// Runtime context for an executing workflow.
///
/// This type is NOT serializable — it holds runtime resources like
/// Arc-wrapped shared state and cancellation tokens. Persistent state
/// is tracked by [`ExecutionState`](crate::state::ExecutionState).
#[derive(Debug)]
pub struct ExecutionContext {
    /// Unique identifier for this execution.
    pub execution_id: ExecutionId,
    /// The workflow definition being executed.
    pub workflow: Arc<WorkflowDefinition>,
    /// Per-node outputs, populated as nodes complete.
    pub node_outputs: Arc<RwLock<HashMap<NodeId, NodeOutput>>>,
    /// Token for cooperative cancellation.
    pub cancellation: CancellationToken,
    /// Execution-level variables shared across nodes.
    pub variables: Arc<RwLock<serde_json::Map<String, serde_json::Value>>>,
    /// Resource budget for this execution.
    pub budget: ExecutionBudget,
}

impl ExecutionContext {
    /// Create a new execution context.
    #[must_use]
    pub fn new(
        execution_id: ExecutionId,
        workflow: Arc<WorkflowDefinition>,
        budget: ExecutionBudget,
    ) -> Self {
        Self {
            execution_id,
            workflow,
            node_outputs: Arc::new(RwLock::new(HashMap::new())),
            cancellation: CancellationToken::new(),
            variables: Arc::new(RwLock::new(serde_json::Map::new())),
            budget,
        }
    }

    /// Replace the cancellation token.
    #[must_use]
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancellation = token;
        self
    }

    /// Set initial variables.
    #[must_use]
    pub fn with_variables(self, vars: serde_json::Map<String, serde_json::Value>) -> Self {
        *self.variables.write() = vars;
        self
    }

    /// Store a node's output.
    pub fn set_node_output(&self, node_id: NodeId, output: NodeOutput) {
        self.node_outputs.write().insert(node_id, output);
    }

    /// Retrieve a node's output.
    #[must_use]
    pub fn get_node_output(&self, node_id: NodeId) -> Option<NodeOutput> {
        self.node_outputs.read().get(&node_id).cloned()
    }

    /// Set an execution variable.
    pub fn set_variable(&self, key: impl Into<String>, value: serde_json::Value) {
        self.variables.write().insert(key.into(), value);
    }

    /// Get an execution variable.
    #[must_use]
    pub fn get_variable(&self, key: &str) -> Option<serde_json::Value> {
        self.variables.read().get(key).cloned()
    }

    /// Check if cancellation has been requested.
    pub fn check_cancelled(&self) -> Result<(), ExecutionError> {
        if self.cancellation.is_cancelled() {
            Err(ExecutionError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Total output bytes across all completed nodes.
    #[must_use]
    pub fn total_output_bytes(&self) -> u64 {
        self.node_outputs.read().values().map(|o| o.bytes).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use nebula_core::{Version, WorkflowId};
    use nebula_workflow::{NodeState, WorkflowConfig};

    fn test_workflow() -> Arc<WorkflowDefinition> {
        let now = Utc::now();
        Arc::new(WorkflowDefinition {
            id: WorkflowId::v4(),
            name: "test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: vec![],
            connections: vec![],
            variables: std::collections::HashMap::new(),
            config: WorkflowConfig::default(),
            tags: vec![],
            created_at: now,
            updated_at: now,
        })
    }

    fn test_context() -> ExecutionContext {
        ExecutionContext::new(
            ExecutionId::v4(),
            test_workflow(),
            ExecutionBudget::default(),
        )
    }

    #[test]
    fn new_context() {
        let ctx = test_context();
        assert!(ctx.node_outputs.read().is_empty());
        assert!(ctx.variables.read().is_empty());
        assert!(!ctx.cancellation.is_cancelled());
    }

    #[test]
    fn set_and_get_node_output() {
        let ctx = test_context();
        let nid = NodeId::v4();
        let output = NodeOutput::inline(serde_json::json!(42), NodeState::Completed, 8);
        ctx.set_node_output(nid, output);

        let retrieved = ctx.get_node_output(nid).unwrap();
        assert!(retrieved.is_inline());
        assert_eq!(retrieved.bytes, 8);
    }

    #[test]
    fn get_missing_node_output() {
        let ctx = test_context();
        assert!(ctx.get_node_output(NodeId::v4()).is_none());
    }

    #[test]
    fn set_and_get_variable() {
        let ctx = test_context();
        ctx.set_variable("key", serde_json::json!("value"));
        assert_eq!(ctx.get_variable("key"), Some(serde_json::json!("value")));
    }

    #[test]
    fn get_missing_variable() {
        let ctx = test_context();
        assert!(ctx.get_variable("missing").is_none());
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
        assert!(err.to_string().contains("cancelled"));
    }

    #[test]
    fn total_output_bytes() {
        let ctx = test_context();
        let n1 = NodeId::v4();
        let n2 = NodeId::v4();
        ctx.set_node_output(
            n1,
            NodeOutput::inline(serde_json::json!(1), NodeState::Completed, 100),
        );
        ctx.set_node_output(
            n2,
            NodeOutput::inline(serde_json::json!(2), NodeState::Completed, 200),
        );
        assert_eq!(ctx.total_output_bytes(), 300);
    }

    #[test]
    fn with_cancellation() {
        let token = CancellationToken::new();
        let child = token.clone();
        let ctx = test_context().with_cancellation(child);
        assert!(!ctx.cancellation.is_cancelled());
        token.cancel();
        assert!(ctx.cancellation.is_cancelled());
    }
}
