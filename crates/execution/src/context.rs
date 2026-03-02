//! Execution context for workflow runs.

use nebula_core::ExecutionId;

/// Temporary placeholder for ExecutionBudget until restored
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ExecutionBudget {
    /// Maximum number of nodes that can run concurrently
    pub max_concurrent_nodes: usize,
}

impl Default for ExecutionBudget {
    /// Default budget with reasonable concurrency (10 nodes).
    fn default() -> Self {
        Self {
            max_concurrent_nodes: 10,
        }
    }
}

/// Lightweight execution context.
///
/// This is a minimal placeholder until execution context is properly designed.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// Unique identifier for this execution.
    pub execution_id: ExecutionId,
    /// Resource budget for this execution.
    pub budget: ExecutionBudget,
}

impl ExecutionContext {
    /// Create a new execution context.
    pub fn new(execution_id: ExecutionId, budget: ExecutionBudget) -> Self {
        Self {
            execution_id,
            budget,
        }
    }
}
