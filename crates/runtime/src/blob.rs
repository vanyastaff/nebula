//! Blob storage for oversized action outputs.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::RuntimeError;

/// Reference to stored blob data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRef {
    /// Blob URI (e.g., `"s3://bucket/key"`).
    pub uri: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// MIME type.
    pub content_type: String,
}

/// Trait for external blob storage backends.
///
/// Implementations could target local filesystem, S3, GCS, etc.
/// The runtime uses this to spill oversized node outputs when
/// [`LargeDataStrategy::SpillToBlob`](crate::LargeDataStrategy::SpillToBlob)
/// is configured.
///
/// # Errors
///
/// Both methods return [`RuntimeError`] on I/O or serialization failures.
#[async_trait]
pub trait BlobStorage: Send + Sync {
    /// Write data to blob storage, returning a reference.
    async fn write(&self, data: &[u8], content_type: &str) -> Result<BlobRef, RuntimeError>;

    /// Read data back from blob storage.
    async fn read(&self, blob_ref: &BlobRef) -> Result<Vec<u8>, RuntimeError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_ref_serialization_round_trip() {
        let blob = BlobRef {
            uri: "s3://bucket/key".into(),
            size_bytes: 1024,
            content_type: "application/json".into(),
        };
        let json = serde_json::to_string(&blob).unwrap();
        let parsed: BlobRef = serde_json::from_str(&json).unwrap();
        assert_eq!(blob, parsed);
    }

    #[test]
    fn blob_ref_debug_output() {
        let blob = BlobRef {
            uri: "file:///tmp/test".into(),
            size_bytes: 42,
            content_type: "text/plain".into(),
        };
        let debug = format!("{blob:?}");
        assert!(debug.contains("file:///tmp/test"));
        assert!(debug.contains("42"));
    }
}
