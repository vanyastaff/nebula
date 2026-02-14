use serde::{Deserialize, Serialize};

/// How action output data is passed between workflow nodes.
///
/// Small data is stored inline as JSON. Large data (exceeding the
/// configured `DataPassingPolicy` limit) is spilled to blob storage,
/// and only a reference is passed through the workflow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NodeOutputData {
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

impl NodeOutputData {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_output() {
        let data = NodeOutputData::inline(serde_json::json!({"result": 42}));
        assert!(data.is_inline());
        assert!(!data.is_blob_ref());
        assert_eq!(data.as_inline(), Some(&serde_json::json!({"result": 42})));
    }

    #[test]
    fn blob_ref_output() {
        let data = NodeOutputData::blob(
            "exec-123/node-456/output.json",
            1_500_000,
            "application/json",
        );
        assert!(data.is_blob_ref());
        assert!(!data.is_inline());
        assert!(data.as_inline().is_none());

        match &data {
            NodeOutputData::BlobRef { key, size, mime } => {
                assert_eq!(key, "exec-123/node-456/output.json");
                assert_eq!(*size, 1_500_000);
                assert_eq!(mime, "application/json");
            }
            _ => panic!("expected BlobRef"),
        }
    }
}
