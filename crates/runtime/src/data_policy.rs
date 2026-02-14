//! Data passing policy for controlling output sizes.
//!
//! Prevents OOM by limiting how much data an action can output.

use serde::{Deserialize, Serialize};

/// Controls how data is passed between nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPassingPolicy {
    /// Maximum size of a single node's output in bytes (default: 10 MB).
    pub max_node_output_bytes: u64,
    /// Maximum total data size across all nodes in an execution (default: 100 MB).
    pub max_total_execution_bytes: u64,
    /// What to do when data exceeds limits.
    pub large_data_strategy: LargeDataStrategy,
}

impl Default for DataPassingPolicy {
    fn default() -> Self {
        Self {
            max_node_output_bytes: 10 * 1024 * 1024,      // 10 MB
            max_total_execution_bytes: 100 * 1024 * 1024, // 100 MB
            large_data_strategy: LargeDataStrategy::Reject,
        }
    }
}

impl DataPassingPolicy {
    /// Check if a serialized output exceeds the per-node limit.
    ///
    /// Returns `Ok(size)` if within limits, or `Err((limit, actual))` if exceeded.
    pub fn check_output_size(&self, output: &serde_json::Value) -> Result<u64, (u64, u64)> {
        let size = serde_json::to_vec(output)
            .map(|v| v.len() as u64)
            .unwrap_or(0);
        if size > self.max_node_output_bytes {
            Err((self.max_node_output_bytes, size))
        } else {
            Ok(size)
        }
    }
}

/// Strategy for handling oversized data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LargeDataStrategy {
    /// Reject the output with `ActionError::DataLimitExceeded`.
    Reject,
    /// Spill to blob storage and pass a reference (Phase 2).
    SpillToBlob,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits() {
        let policy = DataPassingPolicy::default();
        assert_eq!(policy.max_node_output_bytes, 10 * 1024 * 1024);
        assert_eq!(policy.max_total_execution_bytes, 100 * 1024 * 1024);
        assert_eq!(policy.large_data_strategy, LargeDataStrategy::Reject);
    }

    #[test]
    fn check_output_within_limits() {
        let policy = DataPassingPolicy {
            max_node_output_bytes: 1024,
            ..Default::default()
        };
        let small = serde_json::json!({"key": "value"});
        assert!(policy.check_output_size(&small).is_ok());
    }

    #[test]
    fn check_output_exceeds_limits() {
        let policy = DataPassingPolicy {
            max_node_output_bytes: 10, // very small
            ..Default::default()
        };
        let data = serde_json::json!({"a_longer_key": "a_longer_value_that_exceeds"});
        let result = policy.check_output_size(&data);
        assert!(result.is_err());
        let (limit, actual) = result.unwrap_err();
        assert_eq!(limit, 10);
        assert!(actual > 10);
    }

    #[test]
    fn serialization_roundtrip() {
        let policy = DataPassingPolicy::default();
        let json = serde_json::to_string(&policy).unwrap();
        let back: DataPassingPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back.max_node_output_bytes, policy.max_node_output_bytes);
        assert_eq!(back.large_data_strategy, policy.large_data_strategy);
    }
}
