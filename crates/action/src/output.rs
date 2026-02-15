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

// ── ActionOutput<T> ──────────────────────────────────────────────────────────

/// First-class output type for actions, supporting structured values,
/// binary data, external references, and stream handles.
///
/// Wraps the action's output `T` (typically `serde_json::Value`) with
/// additional variants for non-value data. The engine and runtime
/// dispatch on this enum to decide how to pass data between nodes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[non_exhaustive]
pub enum ActionOutput<T> {
    /// A structured value produced by the action.
    Value(T),
    /// Binary data (files, images, etc.).
    Binary(BinaryData),
    /// A reference to data stored externally.
    Reference(DataReference),
    /// A handle to a data stream (consumed asynchronously).
    Stream(StreamReference),
    /// No output produced.
    Empty,
}

impl<T> ActionOutput<T> {
    /// Transform the inner value, preserving non-value variants unchanged.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> ActionOutput<U> {
        match self {
            Self::Value(v) => ActionOutput::Value(f(v)),
            Self::Binary(b) => ActionOutput::Binary(b),
            Self::Reference(r) => ActionOutput::Reference(r),
            Self::Stream(s) => ActionOutput::Stream(s),
            Self::Empty => ActionOutput::Empty,
        }
    }

    /// Fallible transform of the inner value.
    pub fn try_map<U, E>(self, f: impl FnOnce(T) -> Result<U, E>) -> Result<ActionOutput<U>, E> {
        match self {
            Self::Value(v) => Ok(ActionOutput::Value(f(v)?)),
            Self::Binary(b) => Ok(ActionOutput::Binary(b)),
            Self::Reference(r) => Ok(ActionOutput::Reference(r)),
            Self::Stream(s) => Ok(ActionOutput::Stream(s)),
            Self::Empty => Ok(ActionOutput::Empty),
        }
    }

    /// Extract the inner value, returning `None` for non-value variants.
    pub fn into_value(self) -> Option<T> {
        match self {
            Self::Value(v) => Some(v),
            _ => None,
        }
    }

    /// Borrow the inner value, returning `None` for non-value variants.
    pub fn as_value(&self) -> Option<&T> {
        match self {
            Self::Value(v) => Some(v),
            _ => None,
        }
    }

    /// Returns `true` if this is a `Value` variant.
    pub fn is_value(&self) -> bool {
        matches!(self, Self::Value(_))
    }

    /// Returns `true` if this is a `Binary` variant.
    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Binary(_))
    }

    /// Returns `true` if this is a `Reference` variant.
    pub fn is_reference(&self) -> bool {
        matches!(self, Self::Reference(_))
    }

    /// Returns `true` if this is a `Stream` variant.
    pub fn is_stream(&self) -> bool {
        matches!(self, Self::Stream(_))
    }

    /// Returns `true` if this is an `Empty` variant.
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }
}

/// Binary data carried inline or stored externally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryData {
    /// MIME content type (e.g. `"image/png"`, `"application/pdf"`).
    pub content_type: String,
    /// Where the bytes live.
    pub data: BinaryStorage,
    /// Total size in bytes.
    pub size: u64,
    /// Optional metadata (e.g. filename, dimensions).
    pub metadata: Option<serde_json::Value>,
}

/// Storage location for binary data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum BinaryStorage {
    /// Bytes carried inline (small payloads).
    Inline(Vec<u8>),
    /// Bytes stored externally.
    Stored {
        /// Backend identifier (e.g. `"s3"`, `"local"`).
        storage_type: String,
        /// Path or key within the storage backend.
        path: String,
        /// Optional integrity checksum (e.g. SHA-256 hex).
        checksum: Option<String>,
    },
}

/// A reference to data stored externally (not fetched yet).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataReference {
    /// Backend identifier (e.g. `"s3"`, `"local"`, `"database"`).
    pub storage_type: String,
    /// Path or key within the storage backend.
    pub path: String,
    /// Size in bytes (if known).
    pub size: Option<u64>,
    /// MIME content type (if known).
    pub content_type: Option<String>,
}

/// A handle to an async data stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamReference {
    /// Unique identifier for this stream.
    pub stream_id: String,
    /// MIME content type of the stream items (if known).
    pub content_type: Option<String>,
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

    // ── ActionOutput tests ──────────────────────────────────────────

    #[test]
    fn action_output_value() {
        let out = ActionOutput::Value(42);
        assert!(out.is_value());
        assert!(!out.is_binary());
        assert!(!out.is_reference());
        assert!(!out.is_stream());
        assert!(!out.is_empty());
        assert_eq!(out.as_value(), Some(&42));
    }

    #[test]
    fn action_output_binary() {
        let out: ActionOutput<i32> = ActionOutput::Binary(BinaryData {
            content_type: "image/png".into(),
            data: BinaryStorage::Inline(vec![0x89, 0x50, 0x4E, 0x47]),
            size: 4,
            metadata: None,
        });
        assert!(out.is_binary());
        assert!(!out.is_value());
        assert_eq!(out.as_value(), None);
    }

    #[test]
    fn action_output_reference() {
        let out: ActionOutput<i32> = ActionOutput::Reference(DataReference {
            storage_type: "s3".into(),
            path: "bucket/key".into(),
            size: Some(1024),
            content_type: Some("application/json".into()),
        });
        assert!(out.is_reference());
    }

    #[test]
    fn action_output_stream() {
        let out: ActionOutput<i32> = ActionOutput::Stream(StreamReference {
            stream_id: "stream-1".into(),
            content_type: None,
        });
        assert!(out.is_stream());
    }

    #[test]
    fn action_output_empty() {
        let out: ActionOutput<i32> = ActionOutput::Empty;
        assert!(out.is_empty());
        assert_eq!(out.into_value(), None);
    }

    #[test]
    fn action_output_map() {
        let out = ActionOutput::Value(5);
        let mapped = out.map(|n| n * 2);
        assert_eq!(mapped.into_value(), Some(10));
    }

    #[test]
    fn action_output_map_preserves_binary() {
        let out: ActionOutput<i32> = ActionOutput::Binary(BinaryData {
            content_type: "text/plain".into(),
            data: BinaryStorage::Inline(vec![]),
            size: 0,
            metadata: None,
        });
        let mapped: ActionOutput<String> = out.map(|n| n.to_string());
        assert!(mapped.is_binary());
    }

    #[test]
    fn action_output_try_map_ok() {
        let out = ActionOutput::Value(5);
        let mapped = out.try_map(|n| Ok::<_, String>(n * 2));
        assert_eq!(mapped.unwrap().into_value(), Some(10));
    }

    #[test]
    fn action_output_try_map_err() {
        let out = ActionOutput::Value(5);
        let mapped = out.try_map(|_| Err::<i32, _>("fail"));
        assert_eq!(mapped.unwrap_err(), "fail");
    }

    #[test]
    fn action_output_try_map_non_value() {
        let out: ActionOutput<i32> = ActionOutput::Empty;
        let mapped = out.try_map(|_| Err::<i32, _>("should not be called"));
        assert!(mapped.unwrap().is_empty());
    }

    #[test]
    fn action_output_into_value() {
        assert_eq!(ActionOutput::Value(42).into_value(), Some(42));
        assert_eq!(ActionOutput::<i32>::Empty.into_value(), None);
    }
}
