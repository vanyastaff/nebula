//! In-memory backend для разработки и тестов.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::RwLock;

use crate::StorageError;
use crate::storage::Storage;

/// In-memory key-value хранилище (для разработки и тестов).
#[derive(Debug, Default)]
pub struct MemoryStorage {
    inner: RwLock<HashMap<String, Vec<u8>>>,
}

impl MemoryStorage {
    /// Создать пустое in-memory хранилище.
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }
}

/// Тип-обёртка для использования `MemoryStorage` как `Storage<Key = String, Value = T>`
/// для сериализуемых типов.
#[derive(Debug)]
pub struct MemoryStorageTyped<T> {
    inner: MemoryStorage,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T> MemoryStorageTyped<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    /// Создать in-memory хранилище для типа `T`.
    pub fn new() -> Self {
        Self {
            inner: MemoryStorage::new(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T> Default for MemoryStorageTyped<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Storage for MemoryStorage {
    type Key = String;
    type Value = Vec<u8>;

    async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, StorageError> {
        let guard = self.inner.read().await;
        Ok(guard.get(key).cloned())
    }

    async fn set(&self, key: &Self::Key, value: &Self::Value) -> Result<(), StorageError> {
        let mut guard = self.inner.write().await;
        guard.insert(key.clone(), value.clone());
        Ok(())
    }

    async fn delete(&self, key: &Self::Key) -> Result<(), StorageError> {
        let mut guard = self.inner.write().await;
        guard.remove(key);
        Ok(())
    }

    async fn exists(&self, key: &Self::Key) -> Result<bool, StorageError> {
        let guard = self.inner.read().await;
        Ok(guard.contains_key(key))
    }
}

#[async_trait]
impl<T> Storage for MemoryStorageTyped<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    type Key = String;
    type Value = T;

    async fn get(&self, key: &Self::Key) -> Result<Option<Self::Value>, StorageError> {
        let raw = self.inner.get(key).await?;
        match raw {
            None => Ok(None),
            Some(bytes) => {
                let value = serde_json::from_slice(&bytes)?;
                Ok(Some(value))
            }
        }
    }

    async fn set(&self, key: &Self::Key, value: &Self::Value) -> Result<(), StorageError> {
        let bytes = serde_json::to_vec(value)?;
        self.inner.set(key, &bytes).await
    }

    async fn delete(&self, key: &Self::Key) -> Result<(), StorageError> {
        self.inner.delete(key).await
    }

    async fn exists(&self, key: &Self::Key) -> Result<bool, StorageError> {
        self.inner.exists(key).await
    }
}
