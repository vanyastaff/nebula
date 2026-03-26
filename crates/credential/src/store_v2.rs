//! v2 credential store trait with layered composition.
//!
//! Provides a CRUD abstraction for credential persistence with optimistic
//! concurrency control via [`PutMode::CompareAndSwap`]. Encryption is handled
//! by the [`EncryptionLayer`](crate::layer::EncryptionLayer) wrapper, not by
//! store implementations themselves.

use std::future::Future;

use serde_json::Value;

/// How to handle conflicts on put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutMode {
    /// Fail if credential already exists.
    CreateOnly,
    /// Overwrite unconditionally.
    Overwrite,
    /// Compare-and-swap: only succeed if current version matches.
    CompareAndSwap {
        /// The version the caller last observed.
        expected_version: u64,
    },
}

/// A stored credential with metadata.
#[derive(Debug, Clone)]
pub struct StoredCredential {
    /// The credential ID.
    pub id: String,
    /// Serialized credential state (encrypted at the `EncryptionLayer` boundary).
    pub data: Vec<u8>,
    /// State type identifier (`CredentialStateV2::KIND`).
    pub state_kind: String,
    /// Schema version (`CredentialStateV2::VERSION`).
    pub state_version: u32,
    /// Monotonic version counter (for CAS).
    pub version: u64,
    /// When this credential was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When this credential was last modified.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Optional expiration time.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Arbitrary metadata.
    pub metadata: serde_json::Map<String, Value>,
}

/// Error from store operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// Credential not found.
    #[error("credential not found: {id}")]
    NotFound {
        /// The ID that was looked up.
        id: String,
    },
    /// Version conflict on CAS put.
    #[error("version conflict for {id}: expected {expected}, got {actual}")]
    VersionConflict {
        /// The credential ID.
        id: String,
        /// The version the caller expected.
        expected: u64,
        /// The version actually in the store.
        actual: u64,
    },
    /// Credential already exists (`CreateOnly` mode).
    #[error("credential already exists: {id}")]
    AlreadyExists {
        /// The credential ID.
        id: String,
    },
    /// Backend error.
    #[error("store backend error: {0}")]
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

/// Core CRUD trait for credential persistence.
///
/// Implementations handle raw bytes — encryption/decryption is done
/// by the [`EncryptionLayer`](crate::layer::EncryptionLayer) wrapper,
/// not by the store itself.
pub trait CredentialStoreV2: Send + Sync {
    /// Retrieve a stored credential by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] if no credential with the given ID exists.
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn get(&self, id: &str) -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;

    /// Store or update a credential.
    ///
    /// The returned [`StoredCredential`] has its `version`, `created_at`,
    /// and `updated_at` fields set by the store.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::AlreadyExists`] when `mode` is
    /// [`PutMode::CreateOnly`] and the ID already exists.
    ///
    /// Returns [`StoreError::VersionConflict`] when `mode` is
    /// [`PutMode::CompareAndSwap`] and the stored version differs.
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn put(
        &self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;

    /// Delete a credential by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::NotFound`] if no credential with the given ID exists.
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn delete(&self, id: &str) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// List credential IDs, optionally filtered by `state_kind`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn list(
        &self,
        state_kind: Option<&str>,
    ) -> impl Future<Output = Result<Vec<String>, StoreError>> + Send;

    /// Check if a credential exists.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Backend`] on underlying storage failures.
    fn exists(&self, id: &str) -> impl Future<Output = Result<bool, StoreError>> + Send;
}
