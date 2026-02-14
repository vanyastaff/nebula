//! Node output wrapper with metadata.

use chrono::{DateTime, Utc};
use nebula_action::NodeOutputData;
use nebula_workflow::NodeState;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A node's output data along with execution metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOutput {
    /// The output data produced by the action.
    pub data: NodeOutputData,
    /// The node state when this output was produced.
    pub status: NodeState,
    /// When this output was produced.
    pub produced_at: DateTime<Utc>,
    /// How long the node took to produce this output.
    #[serde(default, with = "crate::serde_duration_opt")]
    pub duration: Option<Duration>,
    /// Approximate size of the output in bytes.
    pub bytes: u64,
}

impl NodeOutput {
    /// Create an inline output.
    #[must_use]
    pub fn inline(value: serde_json::Value, status: NodeState, bytes: u64) -> Self {
        Self {
            data: NodeOutputData::inline(value),
            status,
            produced_at: Utc::now(),
            duration: None,
            bytes,
        }
    }

    /// Create a blob reference output.
    #[must_use]
    pub fn blob_ref(
        key: impl Into<String>,
        size: u64,
        mime: impl Into<String>,
        status: NodeState,
    ) -> Self {
        Self {
            data: NodeOutputData::blob(key, size, mime),
            status,
            produced_at: Utc::now(),
            duration: None,
            bytes: size,
        }
    }

    /// Returns `true` if the output data is inline.
    #[must_use]
    pub fn is_inline(&self) -> bool {
        self.data.is_inline()
    }

    /// Returns `true` if the output data is a blob reference.
    #[must_use]
    pub fn is_blob_ref(&self) -> bool {
        self.data.is_blob_ref()
    }

    /// Extract the inline value, if present.
    #[must_use]
    pub fn as_value(&self) -> Option<&serde_json::Value> {
        self.data.as_inline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_output() {
        let output =
            NodeOutput::inline(serde_json::json!({"result": 42}), NodeState::Completed, 128);
        assert!(output.is_inline());
        assert!(!output.is_blob_ref());
        assert_eq!(output.as_value(), Some(&serde_json::json!({"result": 42})));
        assert_eq!(output.bytes, 128);
        assert_eq!(output.status, NodeState::Completed);
    }

    #[test]
    fn blob_ref_output() {
        let output = NodeOutput::blob_ref(
            "exec/node/output.bin",
            1_500_000,
            "application/octet-stream",
            NodeState::Completed,
        );
        assert!(output.is_blob_ref());
        assert!(!output.is_inline());
        assert!(output.as_value().is_none());
        assert_eq!(output.bytes, 1_500_000);
    }

    #[test]
    fn produced_at_is_set() {
        let before = Utc::now();
        let output = NodeOutput::inline(serde_json::json!(null), NodeState::Completed, 0);
        let after = Utc::now();
        assert!(output.produced_at >= before);
        assert!(output.produced_at <= after);
    }

    #[test]
    fn duration_default_none() {
        let output = NodeOutput::inline(serde_json::json!(1), NodeState::Completed, 4);
        assert!(output.duration.is_none());
    }

    #[test]
    fn serde_roundtrip_inline() {
        let output = NodeOutput::inline(
            serde_json::json!({"key": "value"}),
            NodeState::Completed,
            64,
        );
        let json = serde_json::to_string(&output).unwrap();
        let back: NodeOutput = serde_json::from_str(&json).unwrap();
        assert!(back.is_inline());
        assert_eq!(back.bytes, 64);
        assert_eq!(back.status, NodeState::Completed);
    }

    #[test]
    fn serde_roundtrip_blob_ref() {
        let output = NodeOutput::blob_ref("key123", 5000, "text/plain", NodeState::Completed);
        let json = serde_json::to_string(&output).unwrap();
        let back: NodeOutput = serde_json::from_str(&json).unwrap();
        assert!(back.is_blob_ref());
        assert_eq!(back.bytes, 5000);
    }
}
