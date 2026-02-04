//! Storage provider abstraction for credential persistence
//!
//! Provides the [`StorageProvider`] trait that abstracts storage operations,
//! enabling pluggable backends (local filesystem, cloud providers, secret vaults).

use crate::core::{
    CredentialContext, CredentialError, CredentialFilter, CredentialId, CredentialMetadata,
    StorageError,
};
use crate::utils::EncryptedData;
use async_trait::async_trait;
use serde_json::Value;

/// Version for CAS operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateVersion(pub u64);

/// Trait for persistent state storage
#[async_trait]
pub trait StateStore: Send + Sync {
    /// Load state by ID
    async fn load(&self, id: &str) -> Result<(Value, StateVersion), CredentialError>;

    /// Save state with CAS
    async fn save(
        &self,
        id: &str,
        version: StateVersion,
        state: &Value,
    ) -> Result<StateVersion, CredentialError>;

    /// Delete state
    async fn delete(&self, id: &str) -> Result<(), CredentialError>;

    /// Check if state exists
    async fn exists(&self, id: &str) -> Result<bool, CredentialError>;

    /// List all credential IDs
    async fn list(&self) -> Result<Vec<String>, CredentialError>;
}

/// Storage provider trait for credential persistence
///
/// Abstracts storage operations (store, retrieve, delete, list) allowing
/// implementations to target different backends (local filesystem, AWS, Azure,
/// Vault, K8s) without changing application code.
///
/// # Thread Safety
///
/// All implementations must be `Send + Sync` to allow sharing across threads.
///
/// # Examples
///
/// ```no_run
/// use nebula_credential::{
///     StorageProvider, CredentialId, EncryptedData,
///     CredentialMetadata, CredentialContext,
/// };
///
/// async fn example(provider: &dyn StorageProvider) -> Result<(), Box<dyn std::error::Error>> {
///     let id = CredentialId::new("github_token")?;
///     let context = CredentialContext::new("user_123");
///
///     // Check existence
///     let exists = provider.exists(&id, &context).await?;
///
///     Ok(())
/// }
/// ```
#[async_trait]
pub trait StorageProvider: Send + Sync {
    /// Store encrypted credential with metadata
    ///
    /// # Arguments
    ///
    /// * `id` - Unique credential identifier
    /// * `data` - Encrypted credential data (ciphertext + nonce + tag)
    /// * `metadata` - Non-sensitive metadata (timestamps, tags)
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Credential stored successfully
    /// * `Err(StorageError)` - Storage operation failed
    ///
    /// # Errors
    ///
    /// * `StorageError::WriteFailure` - I/O error during write
    /// * `StorageError::PermissionDenied` - Insufficient permissions
    /// * `StorageError::Timeout` - Operation exceeded time limit
    ///
    /// # Idempotency
    ///
    /// Not idempotent - overwrites existing credential if ID matches.
    async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
        context: &CredentialContext,
    ) -> Result<(), StorageError>;

    /// Retrieve encrypted credential by ID
    ///
    /// # Arguments
    ///
    /// * `id` - Unique credential identifier
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    ///
    /// * `Ok((data, metadata))` - Encrypted data and metadata
    /// * `Err(StorageError)` - Retrieval failed
    ///
    /// # Errors
    ///
    /// * `StorageError::NotFound` - Credential does not exist
    /// * `StorageError::ReadFailure` - I/O error during read
    /// * `StorageError::PermissionDenied` - Insufficient permissions
    /// * `StorageError::Timeout` - Operation exceeded time limit
    async fn retrieve(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<(EncryptedData, CredentialMetadata), StorageError>;

    /// Delete credential by ID
    ///
    /// # Arguments
    ///
    /// * `id` - Unique credential identifier
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Credential deleted successfully
    ///
    /// # Errors
    ///
    /// * `StorageError::WriteFailure` - I/O error during delete
    /// * `StorageError::PermissionDenied` - Insufficient permissions
    ///
    /// # Idempotency
    ///
    /// Idempotent - deleting non-existent credential succeeds (returns `Ok(())`).
    async fn delete(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<(), StorageError>;

    /// List all credential IDs (optionally filtered)
    ///
    /// # Arguments
    ///
    /// * `filter` - Optional filter criteria (tags, date ranges)
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<CredentialId>)` - List of credential IDs matching filter
    /// * `Err(StorageError)` - List operation failed
    ///
    /// # Errors
    ///
    /// * `StorageError::ReadFailure` - I/O error during list
    /// * `StorageError::PermissionDenied` - Insufficient permissions
    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
        context: &CredentialContext,
    ) -> Result<Vec<CredentialId>, StorageError>;

    /// Check if credential exists
    ///
    /// # Arguments
    ///
    /// * `id` - Unique credential identifier
    /// * `context` - Request context (owner, scope, trace)
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - Credential exists
    /// * `Ok(false)` - Credential does not exist
    /// * `Err(StorageError)` - Check operation failed
    ///
    /// # Errors
    ///
    /// * `StorageError::ReadFailure` - I/O error during check
    /// * `StorageError::PermissionDenied` - Insufficient permissions
    async fn exists(
        &self,
        id: &CredentialId,
        context: &CredentialContext,
    ) -> Result<bool, StorageError>;
}
