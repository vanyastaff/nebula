//! Large binary object storage.

use std::future::Future;

use crate::{error::StorageError, rows::BlobRow};

/// Storage for binary payloads that exceed inline JSONB limits.
///
/// Blobs are addressed by content-independent IDs (ULID) and referenced
/// from other tables (e.g. `execution_nodes.state_blob_ref`) when the
/// inline limit is exceeded.
pub trait BlobRepo: Send + Sync {
    /// Store a new blob. Returns the generated ID.
    fn put(
        &self,
        content_type: &str,
        data: Vec<u8>,
    ) -> impl Future<Output = Result<Vec<u8>, StorageError>> + Send;

    /// Store a blob with a caller-provided ID (must be unique).
    fn put_with_id(&self, blob: &BlobRow) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Retrieve a blob by ID.
    fn get(&self, id: &[u8]) -> impl Future<Output = Result<Option<BlobRow>, StorageError>> + Send;

    /// Retrieve only the metadata (size, content_type) without the data.
    fn get_metadata(
        &self,
        id: &[u8],
    ) -> impl Future<Output = Result<Option<BlobMetadata>, StorageError>> + Send;

    /// Delete a blob.
    fn delete(&self, id: &[u8]) -> impl Future<Output = Result<(), StorageError>> + Send;
}

/// Blob metadata without payload (for listing/size checks).
#[derive(Debug, Clone)]
pub struct BlobMetadata {
    /// 16-byte BYTEA ID.
    pub id: Vec<u8>,
    /// MIME type.
    pub content_type: String,
    /// Size in bytes.
    pub size: i64,
    /// Creation timestamp.
    pub created_at: chrono::DateTime<chrono::Utc>,
}
