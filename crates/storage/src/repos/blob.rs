//! Large binary object storage.

use async_trait::async_trait;

use crate::{error::StorageError, rows::BlobRow};

/// Storage for binary payloads that exceed inline JSONB limits.
///
/// Blobs are addressed by content-independent IDs (ULID) and referenced
/// from other tables (e.g. `execution_nodes.state_blob_ref`) when the
/// inline limit is exceeded.
#[async_trait]
pub trait BlobRepo: Send + Sync {
    /// Store a new blob. Returns the generated ID.
    async fn put(&self, content_type: &str, data: Vec<u8>) -> Result<Vec<u8>, StorageError>;

    /// Store a blob with a caller-provided ID (must be unique).
    async fn put_with_id(&self, blob: &BlobRow) -> Result<(), StorageError>;

    /// Retrieve a blob by ID.
    async fn get(&self, id: &[u8]) -> Result<Option<BlobRow>, StorageError>;

    /// Retrieve only the metadata (size, content_type) without the data.
    async fn get_metadata(&self, id: &[u8]) -> Result<Option<BlobMetadata>, StorageError>;

    /// Delete a blob.
    async fn delete(&self, id: &[u8]) -> Result<(), StorageError>;
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
