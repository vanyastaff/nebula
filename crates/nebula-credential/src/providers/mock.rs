//! Mock storage provider for testing
//!
//! Provides in-memory storage with error simulation capabilities for unit testing.

use crate::core::{
    CredentialContext, CredentialFilter, CredentialId, CredentialMetadata, StorageError,
};
use crate::utils::EncryptedData;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Mock storage provider for unit testing
///
/// Stores credentials in memory using HashMap with thread-safe access.
/// Supports error simulation for testing error handling paths.
///
/// # Examples
///
/// ```rust
/// use nebula_credential::providers::MockStorageProvider;
/// use nebula_credential::core::StorageError;
///
/// #[tokio::main]
/// async fn main() {
///     let provider = MockStorageProvider::new();
///
///     // Simulate permission denied error
///     provider.fail_next_with(StorageError::PermissionDenied {
///         resource: "test_credential".into(),
///         required_permission: "write".into(),
///     }).await;
///
///     // Next operation will fail with the configured error
/// }
/// ```
#[derive(Clone, Debug)]
pub struct MockStorageProvider {
    /// In-memory credential storage
    storage: Arc<RwLock<HashMap<CredentialId, (EncryptedData, CredentialMetadata)>>>,

    /// Error to inject on next operation (one-time)
    should_fail: Arc<RwLock<Option<StorageError>>>,
}

impl MockStorageProvider {
    /// Create new mock storage provider
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_credential::providers::MockStorageProvider;
    ///
    /// let provider = MockStorageProvider::new();
    /// ```
    pub fn new() -> Self {
        Self {
            storage: Arc::new(RwLock::new(HashMap::new())),
            should_fail: Arc::new(RwLock::new(None)),
        }
    }

    /// Configure mock to fail next operation with specific error
    ///
    /// Error is consumed after one operation (one-time injection).
    ///
    /// # Arguments
    ///
    /// * `error` - Error to return on next operation
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_credential::providers::MockStorageProvider;
    /// use nebula_credential::core::StorageError;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let provider = MockStorageProvider::new();
    ///
    ///     provider.fail_next_with(StorageError::NotFound {
    ///         id: "test_id".into(),
    ///     }).await;
    /// }
    /// ```
    pub async fn fail_next_with(&self, error: StorageError) {
        *self.should_fail.write().await = Some(error);
    }

    /// Clear all stored credentials
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_credential::providers::MockStorageProvider;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let provider = MockStorageProvider::new();
    ///     provider.clear().await;
    /// }
    /// ```
    pub async fn clear(&self) {
        self.storage.write().await.clear();
    }

    /// Get count of stored credentials
    ///
    /// # Returns
    ///
    /// Number of credentials currently in storage
    ///
    /// # Example
    ///
    /// ```rust
    /// use nebula_credential::providers::MockStorageProvider;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let provider = MockStorageProvider::new();
    ///     let count = provider.count().await;
    ///     assert_eq!(count, 0);
    /// }
    /// ```
    pub async fn count(&self) -> usize {
        self.storage.read().await.len()
    }

    /// Check if mock will fail next operation
    ///
    /// Used internally to inject errors
    async fn check_should_fail(&self) -> Result<(), StorageError> {
        if let Some(error) = self.should_fail.write().await.take() {
            return Err(error);
        }
        Ok(())
    }
}

impl Default for MockStorageProvider {
    fn default() -> Self {
        Self::new()
    }
}

// Implement StorageProvider trait
use crate::traits::StorageProvider;
use async_trait::async_trait;

#[async_trait]
impl StorageProvider for MockStorageProvider {
    async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        // Check if should fail
        self.check_should_fail().await?;

        // Store in HashMap
        let mut storage = self.storage.write().await;
        storage.insert(id.clone(), (data, metadata));

        Ok(())
    }

    async fn retrieve(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(EncryptedData, CredentialMetadata), StorageError> {
        // Check if should fail
        self.check_should_fail().await?;

        // Retrieve from HashMap
        let storage = self.storage.read().await;
        storage
            .get(id)
            .cloned()
            .ok_or_else(|| StorageError::NotFound { id: id.to_string() })
    }

    async fn delete(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        // Check if should fail
        self.check_should_fail().await?;

        // Remove from HashMap (idempotent - succeeds even if not found)
        let mut storage = self.storage.write().await;
        storage.remove(id);

        Ok(())
    }

    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
        _context: &CredentialContext,
    ) -> Result<Vec<CredentialId>, StorageError> {
        // Check if should fail
        self.check_should_fail().await?;

        let storage = self.storage.read().await;
        let mut ids: Vec<CredentialId> = storage.keys().cloned().collect();

        // Apply filter if provided
        if let Some(filter) = filter
            && let Some(filter_tags) = &filter.tags
        {
            // Filter by tags - only include credentials that have ALL specified tag key-value pairs
            ids.retain(|id| {
                if let Some((_, metadata)) = storage.get(id) {
                    filter_tags
                        .iter()
                        .all(|(key, value)| metadata.tags.get(key).is_some_and(|v| v == value))
                } else {
                    false
                }
            });
        }

        Ok(ids)
    }

    async fn exists(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<bool, StorageError> {
        // Check if should fail
        self.check_should_fail().await?;

        let storage = self.storage.read().await;
        Ok(storage.contains_key(id))
    }
}
