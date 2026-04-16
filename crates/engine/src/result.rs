//! Execution result types.

use std::{collections::HashMap, time::Duration};

use nebula_core::{NodeKey, id::ExecutionId};
use nebula_execution::ExecutionStatus;

/// The final result of a workflow execution.
#[derive(Debug)]
pub struct ExecutionResult {
    /// Unique execution identifier.
    pub execution_id: ExecutionId,
    /// Final execution status.
    pub status: ExecutionStatus,
    /// Per-node output values (only for successfully completed nodes).
    pub node_outputs: HashMap<NodeKey, serde_json::Value>,
    /// Per-node error messages (only for failed nodes).
    pub node_errors: HashMap<NodeKey, String>,
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
    pub fn node_output(&self, node_key: NodeKey) -> Option<&serde_json::Value> {
        self.node_outputs.get(&node_key)
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::node_key;

    use super::*;

    #[test]
    fn success_result() {
        let result = ExecutionResult {
            execution_id: ExecutionId::new(),
            status: ExecutionStatus::Completed,
            node_outputs: HashMap::new(),
            node_errors: HashMap::new(),
            duration: Duration::from_millis(100),
        };
        assert!(result.is_success());
        assert!(!result.is_failure());
    }

    #[test]
    fn failed_result() {
        let result = ExecutionResult {
            execution_id: ExecutionId::new(),
            status: ExecutionStatus::Failed,
            node_outputs: HashMap::new(),
            node_errors: HashMap::new(),
            duration: Duration::from_millis(50),
        };
        assert!(result.is_failure());
        assert!(!result.is_success());
    }

    #[test]
    fn node_output_lookup() {
        let node_key = node_key!("test_node");
        let mut outputs = HashMap::new();
        outputs.insert(node_key.clone(), serde_json::json!(42));

        let result = ExecutionResult {
            execution_id: ExecutionId::new(),
            status: ExecutionStatus::Completed,
            node_outputs: outputs,
            node_errors: HashMap::new(),
            duration: Duration::from_millis(10),
        };

        assert_eq!(result.node_output(node_key), Some(&serde_json::json!(42)));
        assert!(result.node_output(node_key!("test")).is_none());
    }
}
