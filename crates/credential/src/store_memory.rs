//! In-memory credential store for testing.
//!
//! Data is lost when the store is dropped. Use this in tests rather than
//! mocking [`CredentialStore`] directly.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::store::{CredentialStore, PutMode, StoreError, StoredCredential};

/// In-memory store backed by a `HashMap`. Test-only — data lost on drop.
///
/// Cloning produces a handle to the **same** underlying data (cheap `Arc` clone).
#[derive(Clone)]
pub struct InMemoryStore {
    data: Arc<RwLock<HashMap<String, StoredCredential>>>,
}

impl InMemoryStore {
    /// Create a new, empty in-memory store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CredentialStore for InMemoryStore {
    async fn get(&self, id: &str) -> Result<StoredCredential, StoreError> {
        self.data
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound { id: id.to_string() })
    }

    async fn put(
        &self,
        mut credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, StoreError> {
        let mut data = self.data.write().await;
        match mode {
            PutMode::CreateOnly => {
                if data.contains_key(&credential.id) {
                    return Err(StoreError::AlreadyExists {
                        id: credential.id.clone(),
                    });
                }
                credential.version = 1;
                credential.created_at = chrono::Utc::now();
                credential.updated_at = credential.created_at;
                data.insert(credential.id.clone(), credential.clone());
                Ok(credential)
            }
            PutMode::Overwrite => {
                let version = data
                    .get(&credential.id)
                    .map_or(1, |existing| existing.version + 1);
                credential.version = version;
                credential.updated_at = chrono::Utc::now();
                if version == 1 {
                    credential.created_at = credential.updated_at;
                }
                data.insert(credential.id.clone(), credential.clone());
                Ok(credential)
            }
            PutMode::CompareAndSwap { expected_version } => {
                if let Some(existing) = data.get(&credential.id)
                    && existing.version != expected_version
                {
                    return Err(StoreError::VersionConflict {
                        id: credential.id.clone(),
                        expected: expected_version,
                        actual: existing.version,
                    });
                }
                credential.version = expected_version + 1;
                credential.updated_at = chrono::Utc::now();
                data.insert(credential.id.clone(), credential.clone());
                Ok(credential)
            }
        }
    }

    async fn delete(&self, id: &str) -> Result<(), StoreError> {
        self.data
            .write()
            .await
            .remove(id)
            .map(|_| ())
            .ok_or_else(|| StoreError::NotFound { id: id.to_string() })
    }

    async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
        let data = self.data.read().await;
        let ids: Vec<String> = data
            .values()
            .filter(|c| state_kind.is_none() || state_kind == Some(c.state_kind.as_str()))
            .map(|c| c.id.clone())
            .collect();
        Ok(ids)
    }

    async fn exists(&self, id: &str) -> Result<bool, StoreError> {
        Ok(self.data.read().await.contains_key(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::store::test_helpers::make_credential;

    #[tokio::test]
    async fn crud_operations() {
        let store = InMemoryStore::new();

        let cred = make_credential("test-1", b"secret-data");

        // Create
        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();
        assert_eq!(stored.version, 1);

        // Read
        let fetched = store.get("test-1").await.unwrap();
        assert_eq!(fetched.data, b"secret-data");

        // Exists
        assert!(store.exists("test-1").await.unwrap());
        assert!(!store.exists("nonexistent").await.unwrap());

        // List
        let ids = store.list(None).await.unwrap();
        assert_eq!(ids, vec!["test-1"]);

        // Delete
        store.delete("test-1").await.unwrap();
        assert!(!store.exists("test-1").await.unwrap());
    }

    #[tokio::test]
    async fn get_returns_not_found() {
        let store = InMemoryStore::new();
        let err = store.get("missing").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn delete_returns_not_found() {
        let store = InMemoryStore::new();
        let err = store.delete("missing").await.unwrap_err();
        assert!(matches!(err, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn create_only_rejects_duplicate() {
        let store = InMemoryStore::new();
        let cred = make_credential("dup", b"");
        store.put(cred.clone(), PutMode::CreateOnly).await.unwrap();
        let err = store.put(cred, PutMode::CreateOnly).await.unwrap_err();
        assert!(matches!(err, StoreError::AlreadyExists { .. }));
    }

    #[tokio::test]
    async fn overwrite_increments_version() {
        let store = InMemoryStore::new();
        let cred = make_credential("ow", b"v1");
        let stored = store.put(cred, PutMode::Overwrite).await.unwrap();
        assert_eq!(stored.version, 1);

        let update = make_credential("ow", b"v2");
        let updated = store.put(update, PutMode::Overwrite).await.unwrap();
        assert_eq!(updated.version, 2);
        assert_eq!(updated.data, b"v2");
    }

    #[tokio::test]
    async fn compare_and_swap_succeeds_with_correct_version() {
        let store = InMemoryStore::new();
        let cred = make_credential("cas", b"v1");
        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();
        assert_eq!(stored.version, 1);

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
    }

    #[tokio::test]
    async fn compare_and_swap_rejects_stale_version() {
        let store = InMemoryStore::new();
        let cred = make_credential("cas", b"v1");
        let stored = store.put(cred, PutMode::CreateOnly).await.unwrap();

        // Update to version 2
        let mut update = stored.clone();
        update.data = b"v2".to_vec();
        store
            .put(
                update,
                PutMode::CompareAndSwap {
                    expected_version: 1,
                },
            )
            .await
            .unwrap();

        // Stale CAS with version 1 should fail
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
    async fn list_filters_by_state_kind() {
        let store = InMemoryStore::new();

        let mut bearer = make_credential("c1", b"");
        bearer.state_kind = "bearer".into();
        store.put(bearer, PutMode::CreateOnly).await.unwrap();

        let mut api_key = make_credential("c2", b"");
        api_key.state_kind = "api_key".into();
        store.put(api_key, PutMode::CreateOnly).await.unwrap();

        let all = store.list(None).await.unwrap();
        assert_eq!(all.len(), 2);

        let bearers = store.list(Some("bearer")).await.unwrap();
        assert_eq!(bearers, vec!["c1"]);

        let api_keys = store.list(Some("api_key")).await.unwrap();
        assert_eq!(api_keys, vec!["c2"]);

        let empty = store.list(Some("nonexistent")).await.unwrap();
        assert!(empty.is_empty());
    }
}
