//! Execution output types.
//!
//! [`ExecutionOutput`] is the materialized, persistence-ready form of action
//! output data. By the time data reaches this type, all `Deferred`/`Streaming`
//! variants have been resolved by the engine.
//!
//! [`NodeOutput`] wraps `ExecutionOutput` with execution metadata (status,
//! timing, size).

use chrono::{DateTime, Utc};
use nebula_workflow::NodeState;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Materialized output data for persistence and inter-node transport.
///
/// Small data is stored inline as JSON. Large data (exceeding the configured
/// size limit) is spilled to blob storage, and only a reference is kept.
///
/// This type only exists after the engine has resolved any `Deferred`,
/// `Streaming`, or `Collection` outputs from `ActionOutput`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionOutput {
    /// Small data — stored inline as JSON value.
    Inline(serde_json::Value),

    /// Large data — stored in blob storage, referenced by key.
    BlobRef {
        /// Storage key for retrieving the blob.
        key: String,
        /// Size of the blob in bytes.
        size: u64,
        /// MIME type of the blob content.
        mime: String,
    },
}

impl ExecutionOutput {
    /// Create an inline output from a JSON value.
    pub fn inline(value: serde_json::Value) -> Self {
        Self::Inline(value)
    }

    /// Create a blob reference.
    pub fn blob(key: impl Into<String>, size: u64, mime: impl Into<String>) -> Self {
        Self::BlobRef {
            key: key.into(),
            size,
            mime: mime.into(),
        }
    }

    /// Returns `true` if this is an inline value.
    pub fn is_inline(&self) -> bool {
        matches!(self, Self::Inline(_))
    }

    /// Returns `true` if this is a blob reference.
    pub fn is_blob_ref(&self) -> bool {
        matches!(self, Self::BlobRef { .. })
    }

    /// Extract the inline value, if present.
    pub fn as_inline(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Inline(v) => Some(v),
            Self::BlobRef { .. } => None,
        }
    }
}

/// A node's output data along with execution metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOutput {
    /// The output data produced by the action.
    pub data: ExecutionOutput,
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
            data: ExecutionOutput::inline(value),
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
            data: ExecutionOutput::blob(key, size, mime),
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

    // ── ExecutionOutput tests ───────────────────────────────────────

    #[test]
    fn execution_output_inline() {
        let data = ExecutionOutput::inline(serde_json::json!({"result": 42}));
        assert!(data.is_inline());
        assert!(!data.is_blob_ref());
        assert_eq!(data.as_inline(), Some(&serde_json::json!({"result": 42})));
    }

    #[test]
    fn execution_output_blob_ref() {
        let data = ExecutionOutput::blob(
            "exec-123/node-456/output.json",
            1_500_000,
            "application/json",
        );
        assert!(data.is_blob_ref());
        assert!(!data.is_inline());
        assert!(data.as_inline().is_none());

        match &data {
            ExecutionOutput::BlobRef { key, size, mime } => {
                assert_eq!(key, "exec-123/node-456/output.json");
                assert_eq!(*size, 1_500_000);
                assert_eq!(mime, "application/json");
            }
            _ => panic!("expected BlobRef"),
        }
    }

    // ── NodeOutput tests ────────────────────────────────────────────

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
