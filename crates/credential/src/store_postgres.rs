//! Postgres-backed v2 credential store via [`nebula_storage`] KV layer.
//!
//! Persists [`StoredCredential`] values as JSON in a key-value table
//! (e.g. `storage_kv`) through `Storage<Key = String, Value = Vec<u8>>`.
//! Key format: `cred:{id}`.
//!
//! # Concurrency
//!
//! The underlying `Storage` trait has no compare-and-swap primitive, so
//! [`PutMode::CompareAndSwap`] is implemented as optimistic read-then-write.
//! This is **not** truly atomic — concurrent writers may race. For strong
//! CAS guarantees, use a store backed by a database with native CAS
//! (e.g. Vault KV v2).
//!
//! # Feature gate
//!
//! Requires `storage-postgres`.

use std::sync::Arc;

use nebula_storage::{Storage, StorageError as KvStorageError};
use serde::{Deserialize, Serialize};

use crate::store::{CredentialStore, PutMode, StoreError, StoredCredential};

/// Key prefix used for credential entries in the KV store.
const CRED_KEY_PREFIX: &str = "cred:";

/// Build the KV key for a credential ID.
fn credential_key(id: &str) -> String {
    format!("{CRED_KEY_PREFIX}{id}")
}

// ── Serde wrapper ────────────────────────────────────────────────────────────

/// JSON-serializable representation of a [`StoredCredential`] in the KV store.
///
/// Binary `data` is base64-encoded so it survives JSON round-trips.
#[derive(Serialize, Deserialize)]
struct StoredEntry {
    id: String,
    #[serde(with = "crate::utils::serde_base64")]
    data: Vec<u8>,
    state_kind: String,
    state_version: u32,
    version: u64,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    metadata: serde_json::Map<String, serde_json::Value>,
}

impl From<StoredCredential> for StoredEntry {
    fn from(c: StoredCredential) -> Self {
        Self {
            id: c.id,
            data: c.data,
            state_kind: c.state_kind,
            state_version: c.state_version,
            version: c.version,
            created_at: c.created_at,
            updated_at: c.updated_at,
            expires_at: c.expires_at,
            metadata: c.metadata,
        }
    }
}

impl From<StoredEntry> for StoredCredential {
    fn from(e: StoredEntry) -> Self {
        Self {
            id: e.id,
            data: e.data,
            state_kind: e.state_kind,
            state_version: e.state_version,
            version: e.version,
            created_at: e.created_at,
            updated_at: e.updated_at,
            expires_at: e.expires_at,
            metadata: e.metadata,
        }
    }
}

// ── Error mapping ────────────────────────────────────────────────────────────

/// Map a [`KvStorageError`] to a [`StoreError`].
fn map_kv_error(err: KvStorageError) -> StoreError {
    StoreError::Backend(Box::new(err))
}

// ── PostgresStore ────────────────────────────────────────────────────────────

/// Postgres-backed credential store using the `nebula-storage` KV abstraction.
///
/// Stores each [`StoredCredential`] as a JSON blob keyed by `cred:{id}`.
/// Suitable for server deployments that already run Postgres for workflow
/// storage.
///
/// # Limitations
///
/// - [`PutMode::CompareAndSwap`] uses optimistic read-then-write because the
///   underlying KV trait has no atomic CAS. Concurrent writers may race.
/// - `list` returns an empty list — the `Storage` trait does not expose
///   key-prefix iteration. A future `ListableStorage` extension will fix this.
///
/// # Examples
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use nebula_credential::store_postgres::PostgresStore;
///
/// let kv: Arc<dyn nebula_storage::Storage<Key = String, Value = Vec<u8>> + Send + Sync> = /* ... */;
/// let store = PostgresStore::new(kv);
/// let cred = store.get("my-api-key").await?;
/// ```
pub struct PostgresStore {
    storage: Arc<dyn Storage<Key = String, Value = Vec<u8>> + Send + Sync>,
}

impl PostgresStore {
    /// Create a new Postgres-backed store wrapping the given KV storage.
    ///
    /// The storage is typically a [`nebula_storage::PostgresStorage`].
    /// Ensure the underlying KV table (e.g. `storage_kv`) has been migrated.
    pub fn new(storage: Arc<dyn Storage<Key = String, Value = Vec<u8>> + Send + Sync>) -> Self {
        Self { storage }
    }

    /// Deserialize a raw KV value into a [`StoredCredential`].
    fn deserialize(bytes: &[u8]) -> Result<StoredCredential, StoreError> {
        let entry: StoredEntry =
            serde_json::from_slice(bytes).map_err(|e| StoreError::Backend(Box::new(e)))?;
        Ok(entry.into())
    }

    /// Serialize a [`StoredCredential`] into bytes for KV storage.
    fn serialize(credential: &StoredCredential) -> Result<Vec<u8>, StoreError> {
        let entry: StoredEntry = credential.clone().into();
        serde_json::to_vec(&entry).map_err(|e| StoreError::Backend(Box::new(e)))
    }

    /// Read a credential from the KV store, returning `None` if absent.
    async fn read(&self, id: &str) -> Result<Option<StoredCredential>, StoreError> {
        let key = credential_key(id);
        let bytes = self.storage.get(&key).await.map_err(map_kv_error)?;
        match bytes {
            Some(b) => Ok(Some(Self::deserialize(&b)?)),
            None => Ok(None),
        }
    }
}

impl CredentialStore for PostgresStore {
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        self.read(id)
            .await?
            .ok_or_else(|| StoreError::NotFound { id: id.to_string() })
    }

    async fn put(
        &self,
        mut credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        let existing = self.read(&credential.id).await?;

        match mode {
            PutMode::CreateOnly => {
                if existing.is_some() {
                    return Err(StoreError::AlreadyExists {
                        id: credential.id.clone(),
                    });
                }
                credential.version = 1;
                credential.created_at = chrono::Utc::now();
                credential.updated_at = credential.created_at;
            }
            PutMode::Overwrite => {
                let version = existing.as_ref().map_or(1, |e| e.version + 1);
                credential.version = version;
                credential.updated_at = chrono::Utc::now();
                if version == 1 {
                    credential.created_at = credential.updated_at;
                }
            }
            PutMode::CompareAndSwap { expected_version } => {
                // Optimistic concurrency: read current version, compare, then
                // write. NOT truly atomic — see module-level docs.
                if let Some(ref ex) = existing
                    && ex.version != expected_version
                {
                    return Err(StoreError::VersionConflict {
                        id: credential.id.clone(),
                        expected: expected_version,
                        actual: ex.version,
                    });
                }
                credential.version = expected_version + 1;
                credential.updated_at = chrono::Utc::now();
            }
        }

        let key = credential_key(&credential.id);
        let bytes = Self::serialize(&credential)?;
        self.storage.set(&key, &bytes).await.map_err(map_kv_error)?;

        Ok(credential)
    }

    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        // Check existence first so we can return NotFound.
        if !self.exists(id).await? {
            return Err(StoreError::NotFound { id: id.to_string() });
        }
        let key = credential_key(id);
        self.storage.delete(&key).await.map_err(map_kv_error)
    }

    async fn list(&self, _state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        // The `Storage` trait has no list/prefix-scan method.
        // Return empty until `ListableStorage` is available.
        // See POSTGRES_STORAGE_SPEC.md for the planned extension.
        Ok(Vec::new())
    }

    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        let key = credential_key(id);
        self.storage.exists(&key).await.map_err(map_kv_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nebula_storage::StorageError as KvStorageError;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory KV storage for tests.
    struct MockKv {
        data: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl MockKv {
        fn new() -> Self {
            Self {
                data: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl Storage for MockKv {
        type Key = String;
        type Value = Vec<u8>;

        async fn get(&self, key: &String) -> Result<Option<Vec<u8>>, KvStorageError> {
            Ok(self.data.lock().unwrap().get(key).cloned())
        }

        async fn set(&self, key: &String, value: &Vec<u8>) -> Result<(), KvStorageError> {
            self.data.lock().unwrap().insert(key.clone(), value.clone());
            Ok(())
        }

        async fn delete(&self, key: &String) -> Result<(), KvStorageError> {
            self.data.lock().unwrap().remove(key);
            Ok(())
        }

        async fn exists(&self, key: &String) -> Result<bool, KvStorageError> {
            Ok(self.data.lock().unwrap().contains_key(key))
        }
    }

    fn make_store() -> PostgresStore {
        PostgresStore::new(Arc::new(MockKv::new()))
    }

    use crate::store::test_helpers::make_credential;

    #[tokio::test]
    async fn crud_operations() {
        let store = make_store();
        let cred = make_credential("pg-1", b"secret");

        // Create
        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();
        assert_eq!(stored.version, 1);

        // Read
        let fetched = store.get("pg-1").await.unwrap();
        assert_eq!(fetched.data, b"secret");
        assert_eq!(fetched.version, 1);

        // Exists
        assert!(store.exists("pg-1").await.unwrap());
        assert!(!store.exists("nope").await.unwrap());

        // Overwrite
        let update = make_credential("pg-1", b"updated");
        let updated = store.put(update, PutMode::Overwrite).await.unwrap();
        assert_eq!(updated.version, 2);
        assert_eq!(updated.data, b"updated");

        // Delete
        store.delete("pg-1").await.unwrap();
        assert!(!store.exists("pg-1").await.unwrap());
    }

    #[tokio::test]
    async fn create_only_rejects_duplicate() {
        let store = make_store();
        store
            .put(make_credential("dup", b""), PutMode::CreateOnly)
            .await
            .unwrap();

        let err = store
            .put(make_credential("dup", b""), PutMode::CreateOnly)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::AlreadyExists { .. }));
    }

    #[tokio::test]
    async fn compare_and_swap() {
        let store = make_store();
        let stored = store
            .put(make_credential("cas", b"v1"), PutMode::CreateOnly)
            .await
            .unwrap();
        assert_eq!(stored.version, 1);

        // Successful CAS
        let mut update = stored.clone();
        update.data = b"v2".to_vec();
        let updated = store
            .put(
                update,
                PutMode::CompareAndSwap {
                    expected_version: 1,
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.version, 2);

        // Stale CAS
        let mut stale = stored;
        stale.data = b"v3".to_vec();
        let err = store
            .put(
                stale,
                PutMode::CompareAndSwap {
                    expected_version: 1,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::VersionConflict { .. }));
    }

    #[tokio::test]
    async fn get_not_found() {
        let store = make_store();
        let err = store.get("missing").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn delete_not_found() {
        let store = make_store();
        let err = store.delete("missing").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn list_returns_empty() {
        let store = make_store();
        // Storage trait has no list; always returns empty for now.
        let ids = store.list(None).await.unwrap();
        assert!(ids.is_empty());
    }
}
