//! Test-only in-memory credential store.
//!
//! This is a **test shim**. The canonical production `InMemoryStore` lives
//! in `nebula_storage::credential::InMemoryStore` per storage credential layers.
//! This copy lives in `nebula-credential-testutil` (`publish = false`) so the
//! contract crate (`nebula-credential`) does not export `#[cfg(test)]`-style
//! code.
//!
//! Production consumers (composition roots, examples, docs) should import
//! `nebula_storage::credential::InMemoryStore`.
//!
//! Data is lost when the store is dropped.

use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;

use nebula_credential::store::{CredentialStore, PutMode, StoreError, StoredCredential};

/// Test-only in-memory store backed by a `HashMap`. See module docs for why
/// this lives here in addition to `nebula_storage::credential::InMemoryStore`.
///
/// Cloning produces a handle to the **same** underlying data (cheap `Arc` clone).
#[derive(Clone)]
pub struct InMemoryStore {
    /// Test-shim store. `tokio::sync::RwLock` is chosen here for
    /// trait-impl ergonomics — every `async fn` body locks, mutates the
    /// map, and returns without awaiting under the guard, so a
    /// `parking_lot::RwLock` in a sync block would be cheaper. Perf is
    /// irrelevant in this shim; production storage lives in
    /// `nebula-storage` per storage credential layers. Do **NOT** cargo-cult this
    /// `tokio::RwLock<HashMap<...>>` pattern into a production module
    /// where the guard could cross an `.await` — that's the
    /// issue-#587-shaped perf cost the audit flagged.
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
            },
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
            },
            PutMode::CompareAndSwap { expected_version } => {
                let Some(existing) = data.get(&credential.id) else {
                    return Err(StoreError::NotFound {
                        id: credential.id.clone(),
                    });
                };
                if existing.version != expected_version {
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
            },
            // PutMode is #[non_exhaustive]; reject unknown variants so
            // callers learn about them at runtime rather than silently
            // accepting undefined behaviour.
            _ => Err(StoreError::Backend(Box::new(std::io::Error::other(
                "unrecognised PutMode variant in InMemoryStore shim",
            )))),
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
