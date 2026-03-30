//! Postgres-backed storage provider using [`nebula-storage`] KV layer.
//!
//! Persists credentials in a key-value table (e.g. `storage_kv`) via
//! `Storage<Key = String, Value = Vec<u8>>`. Key format: `cred:{id}`.
//! Value: JSON-serialized [`CredentialFile`](crate::providers::credential_file::CredentialFile).
//!
//! See `docs/crates/credential/POSTGRES_STORAGE_SPEC.md` for key/value layout and error mapping.

use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use nebula_storage::{Storage, StorageError as KvStorageError};

use crate::core::{
    CredentialContext, CredentialFilter, CredentialId, CredentialMetadata, StorageError,
};
use crate::providers::StorageMetrics;
use crate::providers::credential_file::CredentialFile;
use crate::traits::StorageProvider;
use crate::utils::EncryptedData;

const CRED_KEY_PREFIX: &str = "cred:";
const ROTATION_KEY_PREFIX: &str = "rotation:";

fn credential_key(id: &CredentialId) -> String {
    format!("{}{}", CRED_KEY_PREFIX, id)
}

fn rotation_key(transaction_id: &str) -> String {
    format!("{}{}", ROTATION_KEY_PREFIX, transaction_id)
}

fn map_kv_error(id: &str, err: KvStorageError, is_write: bool) -> StorageError {
    let msg = match &err {
        KvStorageError::Backend(s) => s.clone(),
        KvStorageError::Serialization(e) => e.to_string(),
        KvStorageError::NotFound => "not found".to_string(),
    };
    let source = io::Error::other(msg);
    if is_write {
        StorageError::WriteFailure {
            id: id.to_string(),
            source,
        }
    } else {
        StorageError::ReadFailure {
            id: id.to_string(),
            source,
        }
    }
}

/// Postgres-backed credential storage provider.
///
/// Uses a generic key-value [`Storage`] implementation (typically
/// [`nebula_storage::PostgresStorage`]) to persist encrypted credentials.
/// Scope is not enforced here; the manager layer uses `retrieve_scoped` / `list_scoped`.
#[derive(Clone)]
pub struct PostgresStorageProvider {
    storage: Arc<dyn Storage<Key = String, Value = Vec<u8>> + Send + Sync>,
    metrics: Arc<parking_lot::RwLock<StorageMetrics>>,
}

impl PostgresStorageProvider {
    /// Create a new provider that uses the given KV storage.
    ///
    /// The storage should be a [`nebula_storage::PostgresStorage`] (or any
    /// `Storage<Key = String, Value = Vec<u8>>`). Ensure migrations for the
    /// KV table (e.g. `storage_kv`) have been run.
    pub fn new(storage: Arc<dyn Storage<Key = String, Value = Vec<u8>> + Send + Sync>) -> Self {
        Self {
            storage,
            metrics: Arc::new(parking_lot::RwLock::new(StorageMetrics::new())),
        }
    }

    fn record_operation(&self, op: &str, elapsed: std::time::Duration, success: bool) {
        self.metrics.write().record_operation(op, elapsed, success);
    }
}

#[async_trait]
impl StorageProvider for PostgresStorageProvider {
    async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let start = std::time::Instant::now();
        let key = credential_key(id);

        let cred_file = CredentialFile::new(data, metadata);
        let bytes = serde_json::to_vec(&cred_file).map_err(|e| StorageError::WriteFailure {
            id: id.to_string(),
            source: io::Error::new(io::ErrorKind::InvalidData, e),
        })?;

        self.storage
            .set(&key, &bytes)
            .await
            .map_err(|e| map_kv_error(&id.to_string(), e, true))?;

        self.record_operation("store", start.elapsed(), true);
        Ok(())
    }

    async fn retrieve(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(EncryptedData, CredentialMetadata), StorageError> {
        let start = std::time::Instant::now();
        let key = credential_key(id);

        let bytes = self
            .storage
            .get(&key)
            .await
            .map_err(|e| map_kv_error(&id.to_string(), e, false))?;

        let bytes = bytes.ok_or_else(|| StorageError::NotFound { id: id.to_string() })?;

        let cred_file: CredentialFile =
            serde_json::from_slice(&bytes).map_err(|e| StorageError::ReadFailure {
                id: id.to_string(),
                source: io::Error::new(io::ErrorKind::InvalidData, e),
            })?;

        self.record_operation("retrieve", start.elapsed(), true);
        Ok((cred_file.encrypted_data, cred_file.metadata))
    }

    async fn delete(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let start = std::time::Instant::now();
        let key = credential_key(id);

        self.storage
            .delete(&key)
            .await
            .map_err(|e| map_kv_error(&id.to_string(), e, true))?;

        self.record_operation("delete", start.elapsed(), true);
        Ok(())
    }

    async fn list(
        &self,
        _filter: Option<&CredentialFilter>,
        _context: &CredentialContext,
    ) -> Result<Vec<CredentialId>, StorageError> {
        // KV Storage has no list_prefix yet; return empty until ListableStorage is available.
        // See POSTGRES_STORAGE_SPEC.md.
        Ok(Vec::new())
    }

    async fn exists(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<bool, StorageError> {
        let key = credential_key(id);
        self.storage
            .exists(&key)
            .await
            .map_err(|e| map_kv_error(&id.to_string(), e, false))
    }

    async fn store_rotation_state(
        &self,
        transaction_id: &str,
        state: &serde_json::Value,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let key = rotation_key(transaction_id);
        let bytes = serde_json::to_vec(state).map_err(|e| StorageError::WriteFailure {
            id: transaction_id.to_string(),
            source: io::Error::new(io::ErrorKind::InvalidData, e),
        })?;
        self.storage
            .set(&key, &bytes)
            .await
            .map_err(|e| map_kv_error(transaction_id, e, true))?;
        Ok(())
    }

    async fn get_rotation_state(
        &self,
        transaction_id: &str,
        _context: &CredentialContext,
    ) -> Result<Option<serde_json::Value>, StorageError> {
        let key = rotation_key(transaction_id);
        let bytes = self
            .storage
            .get(&key)
            .await
            .map_err(|e| map_kv_error(transaction_id, e, false))?;
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let value = serde_json::from_slice(&bytes).map_err(|e| StorageError::ReadFailure {
            id: transaction_id.to_string(),
            source: io::Error::new(io::ErrorKind::InvalidData, e),
        })?;
        Ok(Some(value))
    }

    async fn delete_rotation_state(
        &self,
        transaction_id: &str,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let key = rotation_key(transaction_id);
        self.storage
            .delete(&key)
            .await
            .map_err(|e| map_kv_error(transaction_id, e, true))?;
        Ok(())
    }
}
