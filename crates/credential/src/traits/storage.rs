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

    // ========================================================================
    // Credential Rotation State Tracking (Phase 4)
    // ========================================================================

    /// Store rotation transaction state
    ///
    /// Persists the state of an in-progress rotation transaction.
    /// This is a stub - implementations will be added in user stories.
    ///
    /// # Arguments
    ///
    /// * `transaction_id` - Unique rotation transaction identifier
    /// * `state` - Serialized transaction state
    /// * `context` - Request context
    ///
    /// # Default Implementation
    ///
    /// Returns `StorageError::NotSupported` - implementations should override
    /// this method to support rotation tracking.
    async fn store_rotation_state(
        &self,
        _transaction_id: &str,
        _state: &Value,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        Err(StorageError::NotSupported {
            operation: "store_rotation_state".to_string(),
            reason: "Rotation state tracking not implemented for this storage provider".to_string(),
        })
    }

    /// Retrieve rotation transaction state
    ///
    /// Loads the state of a rotation transaction.
    /// This is a stub - implementations will be added in user stories.
    ///
    /// # Arguments
    ///
    /// * `transaction_id` - Unique rotation transaction identifier
    /// * `context` - Request context
    ///
    /// # Returns
    ///
    /// * `Ok(Some(state))` - Transaction state found
    /// * `Ok(None)` - Transaction not found
    /// * `Err(StorageError)` - Retrieval failed
    ///
    /// # Default Implementation
    ///
    /// Returns `StorageError::NotSupported` - implementations should override
    /// this method to support rotation tracking.
    async fn get_rotation_state(
        &self,
        _transaction_id: &str,
        _context: &CredentialContext,
    ) -> Result<Option<Value>, StorageError> {
        Err(StorageError::NotSupported {
            operation: "get_rotation_state".to_string(),
            reason: "Rotation state tracking not implemented for this storage provider".to_string(),
        })
    }

    /// Delete rotation transaction state
    ///
    /// Removes completed or cancelled rotation transaction state.
    /// This is a stub - implementations will be added in user stories.
    ///
    /// # Arguments
    ///
    /// * `transaction_id` - Unique rotation transaction identifier
    /// * `context` - Request context
    ///
    /// # Default Implementation
    ///
    /// Returns `StorageError::NotSupported` - implementations should override
    /// this method to support rotation tracking.
    async fn delete_rotation_state(
        &self,
        _transaction_id: &str,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        Err(StorageError::NotSupported {
            operation: "delete_rotation_state".to_string(),
            reason: "Rotation state tracking not implemented for this storage provider".to_string(),
        })
    }

    /// List all in-progress rotation transactions for a credential
    ///
    /// Returns transaction IDs for active rotations.
    /// This is a stub - implementations will be added in user stories.
    ///
    /// # Arguments
    ///
    /// * `credential_id` - Credential to check for rotations
    /// * `context` - Request context
    ///
    /// # Returns
    ///
    /// * `Ok(vec![transaction_ids])` - List of active rotation transaction IDs
    /// * `Err(StorageError)` - List operation failed
    ///
    /// # Default Implementation
    ///
    /// Returns empty vector - implementations should override this method
    /// to support rotation tracking.
    async fn list_rotation_transactions(
        &self,
        _credential_id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<Vec<String>, StorageError> {
        Ok(Vec::new())
    }

    /// Store usage metrics for a credential during grace period
    ///
    /// # T070: Usage Metric Persistence
    ///
    /// Persists usage metrics for credentials during grace period to track
    /// migration progress and determine when old credentials can be safely revoked.
    ///
    /// # Arguments
    ///
    /// * `credential_id` - Credential being tracked
    /// * `metrics` - Usage metrics (request count, last used, etc.)
    /// * `context` - Request context
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Metrics stored successfully
    /// * `Err(StorageError)` - Storage operation failed
    ///
    /// # Default Implementation
    ///
    /// Returns error - implementations should override this method
    /// to support usage metric persistence.
    async fn store_usage_metrics(
        &self,
        _credential_id: &CredentialId,
        _metrics: &crate::rotation::UsageMetrics,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        Err(StorageError::NotSupported {
            operation: "store_usage_metrics".to_string(),
            reason: "Usage metric persistence not implemented for this storage provider"
                .to_string(),
        })
    }

    /// Retrieve usage metrics for a credential
    ///
    /// # Arguments
    ///
    /// * `credential_id` - Credential to get metrics for
    /// * `context` - Request context
    ///
    /// # Returns
    ///
    /// * `Ok(Some(metrics))` - Metrics found and retrieved
    /// * `Ok(None)` - No metrics stored for this credential
    /// * `Err(StorageError)` - Retrieval operation failed
    ///
    /// # Default Implementation
    ///
    /// Returns None - implementations should override this method
    /// to support usage metric retrieval.
    async fn retrieve_usage_metrics(
        &self,
        _credential_id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<Option<crate::rotation::UsageMetrics>, StorageError> {
        Ok(None)
    }

    /// Delete usage metrics for a credential
    ///
    /// Called when grace period ends and metrics are no longer needed.
    ///
    /// # Arguments
    ///
    /// * `credential_id` - Credential to delete metrics for
    /// * `context` - Request context
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Metrics deleted or didn't exist
    /// * `Err(StorageError)` - Delete operation failed
    ///
    /// # Default Implementation
    ///
    /// Returns Ok(()) - implementations should override this method
    /// to support usage metric cleanup.
    async fn delete_usage_metrics(
        &self,
        _credential_id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        Ok(())
    }
}
