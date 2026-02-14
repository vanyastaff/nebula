use std::time::Duration;

/// Resource budget for an entire workflow execution.
///
/// The engine enforces these limits across all nodes in an execution.
/// Individual actions do not see or enforce these — the engine/executor
/// layer is responsible.
#[derive(Debug, Clone)]
pub struct ExecutionBudget {
    /// Maximum nodes executing concurrently within this execution.
    pub max_concurrent_nodes: usize,
    /// Maximum total retry attempts across all nodes.
    pub max_total_retries: u32,
    /// Maximum wall-clock time for the entire execution.
    pub max_wall_time: Duration,
    /// Maximum total payload size across all node outputs (bytes).
    pub max_payload_bytes: u64,
    /// Data passing policy for node outputs.
    pub data_policy: DataPassingPolicy,
}

impl Default for ExecutionBudget {
    fn default() -> Self {
        Self {
            max_concurrent_nodes: 10,
            max_total_retries: 50,
            max_wall_time: Duration::from_secs(3600), // 1 hour
            max_payload_bytes: 100 * 1024 * 1024,     // 100 MB
            data_policy: DataPassingPolicy::default(),
        }
    }
}

/// Policy controlling how data is passed between workflow nodes.
#[derive(Debug, Clone)]
pub struct DataPassingPolicy {
    /// Maximum output size per node (bytes). Default: 10 MB.
    pub max_node_output_bytes: u64,
    /// Maximum total data across the execution (bytes). Default: 100 MB.
    pub max_total_execution_bytes: u64,
    /// What to do when a node's output exceeds the limit.
    pub large_data_strategy: LargeDataStrategy,
}

impl Default for DataPassingPolicy {
    fn default() -> Self {
        Self {
            max_node_output_bytes: 10 * 1024 * 1024,   // 10 MB
            max_total_execution_bytes: 100 * 1024 * 1024, // 100 MB
            large_data_strategy: LargeDataStrategy::Reject,
        }
    }
}

/// Strategy for handling outputs that exceed the per-node size limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LargeDataStrategy {
    /// Reject the output and return `ActionError::DataLimitExceeded`.
    Reject,
    /// Spill the output to blob storage, pass a `NodeOutputData::BlobRef` instead.
    SpillToBlob,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budget() {
        let budget = ExecutionBudget::default();
        assert_eq!(budget.max_concurrent_nodes, 10);
        assert_eq!(budget.max_total_retries, 50);
        assert_eq!(budget.max_wall_time, Duration::from_secs(3600));
        assert_eq!(budget.max_payload_bytes, 100 * 1024 * 1024);
    }

    #[test]
    fn default_data_policy() {
        let policy = DataPassingPolicy::default();
        assert_eq!(policy.max_node_output_bytes, 10 * 1024 * 1024);
        assert_eq!(policy.max_total_execution_bytes, 100 * 1024 * 1024);
        assert_eq!(policy.large_data_strategy, LargeDataStrategy::Reject);
    }

    #[test]
    fn custom_budget() {
        let budget = ExecutionBudget {
            max_concurrent_nodes: 50,
            max_total_retries: 100,
            max_wall_time: Duration::from_secs(7200),
            max_payload_bytes: 500 * 1024 * 1024,
            data_policy: DataPassingPolicy {
                max_node_output_bytes: 50 * 1024 * 1024,
                max_total_execution_bytes: 500 * 1024 * 1024,
                large_data_strategy: LargeDataStrategy::SpillToBlob,
            },
        };
        assert_eq!(budget.max_concurrent_nodes, 50);
        assert_eq!(
            budget.data_policy.large_data_strategy,
            LargeDataStrategy::SpillToBlob
        );
    }
}
