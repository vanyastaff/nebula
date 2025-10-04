//! In-memory state storage for testing

use crate::core::CredentialError;
use crate::traits::{StateStore, StateVersion};
use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;

/// In-memory implementation of StateStore
pub struct MemoryStateStore {
    states: Arc<DashMap<String, (Value, StateVersion)>>,
}

impl MemoryStateStore {
    /// Create new in-memory store
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            states: Arc::new(DashMap::new()),
        })
    }

    /// Clear all states
    pub fn clear(&self) {
        self.states.clear();
    }

    /// Get number of stored states
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

impl Default for MemoryStateStore {
    fn default() -> Self {
        Self {
            states: Arc::new(DashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl StateStore for MemoryStateStore {
    async fn load(&self, id: &str) -> Result<(Value, StateVersion), CredentialError> {
        self.states
            .get(id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| CredentialError::not_found(id))
    }

    async fn save(
        &self,
        id: &str,
        version: StateVersion,
        state: &Value,
    ) -> Result<StateVersion, CredentialError> {
        match self.states.entry(id.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(mut entry) => {
                let (_, current_version) = entry.get();
                if *current_version != version {
                    return Err(CredentialError::CasConflict);
                }
                let new_version = StateVersion(version.0 + 1);
                entry.insert((state.clone(), new_version));
                Ok(new_version)
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                let new_version = StateVersion(1);
                entry.insert((state.clone(), new_version));
                Ok(new_version)
            }
        }
    }

    async fn delete(&self, id: &str) -> Result<(), CredentialError> {
        self.states
            .remove(id)
            .ok_or_else(|| CredentialError::not_found(id))?;
        Ok(())
    }

    async fn exists(&self, id: &str) -> Result<bool, CredentialError> {
        Ok(self.states.contains_key(id))
    }

    async fn list(&self) -> Result<Vec<String>, CredentialError> {
        Ok(self
            .states
            .iter()
            .map(|entry| entry.key().clone())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_memory_store_basic() {
        let store = MemoryStateStore::new();
        assert!(store.is_empty());

        let state = json!({"key": "value"});
        let v1 = store.save("test", StateVersion(0), &state).await.unwrap();
        assert_eq!(v1, StateVersion(1));
        assert!(!store.is_empty());

        let (loaded, version) = store.load("test").await.unwrap();
        assert_eq!(loaded, state);
        assert_eq!(version, StateVersion(1));

        assert!(store.exists("test").await.unwrap());

        let ids = store.list().await.unwrap();
        assert_eq!(ids.len(), 1);

        store.delete("test").await.unwrap();
        assert!(store.is_empty());
    }

    #[tokio::test]
    async fn test_memory_store_cas() {
        let store = MemoryStateStore::new();

        let state1 = json!({"v": 1});
        let state2 = json!({"v": 2});

        let v1 = store.save("test", StateVersion(0), &state1).await.unwrap();
        assert_eq!(v1, StateVersion(1));

        let v2 = store.save("test", v1, &state2).await.unwrap();
        assert_eq!(v2, StateVersion(2));

        // CAS conflict with old version
        let result = store.save("test", v1, &state1).await;
        assert!(result.is_err());

        let (state, version) = store.load("test").await.unwrap();
        assert_eq!(state, state2);
        assert_eq!(version, StateVersion(2));
    }

    #[tokio::test]
    async fn test_memory_store_not_found() {
        let store = MemoryStateStore::new();

        let result = store.load("nonexistent").await;
        assert!(result.is_err());

        let result = store.delete("nonexistent").await;
        assert!(result.is_err());

        assert!(!store.exists("nonexistent").await.unwrap());
    }
}
