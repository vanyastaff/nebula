//! Execution result types.

use std::collections::HashMap;
use std::time::Duration;

use nebula_core::id::{ExecutionId, NodeId};
use nebula_execution::ExecutionStatus;

/// The final result of a workflow execution.
#[derive(Debug)]
pub struct ExecutionResult {
    /// Unique execution identifier.
    pub execution_id: ExecutionId,
    /// Final execution status.
    pub status: ExecutionStatus,
    /// Per-node output values (only for successfully completed nodes).
    pub node_outputs: HashMap<NodeId, serde_json::Value>,
    /// Wall-clock duration of the execution.
    pub duration: Duration,
}

impl ExecutionResult {
    /// Whether the execution completed successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Whether the execution failed.
    #[must_use]
    pub fn is_failure(&self) -> bool {
        self.status.is_failure()
    }

    /// Get a specific node's output.
    #[must_use]
    pub fn node_output(&self, node_id: NodeId) -> Option<&serde_json::Value> {
        self.node_outputs.get(&node_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_result() {
        let result = ExecutionResult {
            execution_id: ExecutionId::v4(),
            status: ExecutionStatus::Completed,
            node_outputs: HashMap::new(),
            duration: Duration::from_millis(100),
        };
        assert!(result.is_success());
        assert!(!result.is_failure());
    }

    #[test]
    fn failed_result() {
        let result = ExecutionResult {
            execution_id: ExecutionId::v4(),
            status: ExecutionStatus::Failed,
            node_outputs: HashMap::new(),
            duration: Duration::from_millis(50),
        };
        assert!(result.is_failure());
        assert!(!result.is_success());
    }

    #[test]
    fn node_output_lookup() {
        let node_id = NodeId::v4();
        let mut outputs = HashMap::new();
        outputs.insert(node_id, serde_json::json!(42));

        let result = ExecutionResult {
            execution_id: ExecutionId::v4(),
            status: ExecutionStatus::Completed,
            node_outputs: outputs,
            duration: Duration::from_millis(10),
        };

        assert_eq!(result.node_output(node_id), Some(&serde_json::json!(42)));
        assert!(result.node_output(NodeId::v4()).is_none());
    }
}
