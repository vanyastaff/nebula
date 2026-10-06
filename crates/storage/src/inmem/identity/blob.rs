//! Blobs: workspace-scoped payloads with an optional expiry.

use std::collections::HashMap;
use std::sync::Arc;

use nebula_storage_port::StorageError;
use nebula_storage_port::dto::BlobRow;
use nebula_storage_port::store::BlobStore;
use parking_lot::Mutex;

/// Blob key: `(workspace_id, id)` so a cross-workspace `get` misses.
type BlobKey = (String, String);

/// In-memory `blobs` store.
#[derive(Debug, Default, Clone)]
pub struct InMemoryBlobStore {
    inner: Arc<Mutex<HashMap<BlobKey, BlobRow>>>,
}

impl InMemoryBlobStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl BlobStore for InMemoryBlobStore {
    async fn put(&self, row: BlobRow) -> Result<(), StorageError> {
        let key = (row.workspace_id.clone(), row.id.clone());
        self.inner.lock().insert(key, row);
        Ok(())
    }

    async fn get(&self, workspace_id: &str, id: &str) -> Result<Option<BlobRow>, StorageError> {
        Ok(self
            .inner
            .lock()
            .get(&(workspace_id.to_owned(), id.to_owned()))
            .cloned())
    }

    async fn delete(&self, workspace_id: &str, id: &str) -> Result<(), StorageError> {
        self.inner
            .lock()
            .remove(&(workspace_id.to_owned(), id.to_owned()));
        Ok(())
    }

    async fn evict_expired(&self) -> Result<u64, StorageError> {
        // Compare parsed instants, not RFC 3339 strings: a lexical compare is
        // wrong across offsets and fractional-second precision. A blob whose
        // `expires_at` does not parse is treated as expired (fail closed —
        // never retain a row that cannot be proven fresh).
        let now = chrono::Utc::now();
        let mut rows = self.inner.lock();
        let before = rows.len();
        rows.retain(|_, blob| match &blob.expires_at {
            Some(expires_at) => chrono::DateTime::parse_from_rfc3339(expires_at)
                .is_ok_and(|expires_at| expires_at > now),
            None => true,
        });
        Ok(u64::try_from(before - rows.len()).unwrap_or(u64::MAX))
    }
}
