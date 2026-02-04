//! Storage provider trait tests
//!
//! Tests for StorageProvider trait using MockStorageProvider implementation.

use nebula_credential::core::{
    CredentialContext, CredentialFilter, CredentialId, CredentialMetadata, StorageError,
};
use nebula_credential::traits::StorageProvider;
use nebula_credential::utils::{EncryptedData, EncryptionKey, encrypt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Mock storage provider for testing
struct MockStorageProvider {
    data: Arc<RwLock<HashMap<String, (EncryptedData, CredentialMetadata)>>>,
}

impl MockStorageProvider {
    fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl StorageProvider for MockStorageProvider {
    async fn store(
        &self,
        id: &CredentialId,
        data: EncryptedData,
        metadata: CredentialMetadata,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let mut storage = self.data.write().await;
        storage.insert(id.as_str().to_string(), (data, metadata));
        Ok(())
    }

    async fn retrieve(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(EncryptedData, CredentialMetadata), StorageError> {
        let storage = self.data.read().await;
        storage
            .get(id.as_str())
            .cloned()
            .ok_or_else(|| StorageError::NotFound {
                id: id.as_str().to_string(),
            })
    }

    async fn delete(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        let mut storage = self.data.write().await;
        storage.remove(id.as_str());
        Ok(()) // Idempotent - removing non-existent key succeeds
    }

    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
        _context: &CredentialContext,
    ) -> Result<Vec<CredentialId>, StorageError> {
        let storage = self.data.read().await;
        let mut ids: Vec<_> = storage
            .iter()
            .filter(|(_, (_, metadata))| {
                if let Some(filter) = filter {
                    // Check tags filter
                    if let Some(filter_tags) = &filter.tags {
                        for (key, value) in filter_tags {
                            if metadata.tags.get(key) != Some(value) {
                                return false;
                            }
                        }
                    }
                    // Check date range filters
                    if let Some(after) = filter.created_after {
                        if metadata.created_at < after {
                            return false;
                        }
                    }
                    if let Some(before) = filter.created_before {
                        if metadata.created_at > before {
                            return false;
                        }
                    }
                }
                true
            })
            .map(|(id, _)| CredentialId::new(id.clone()).expect("stored IDs should be valid"))
            .collect();

        // Sort for deterministic output
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(ids)
    }

    async fn exists(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<bool, StorageError> {
        let storage = self.data.read().await;
        Ok(storage.contains_key(id.as_str()))
    }
}

// Helper function to create test data
fn create_test_data() -> (EncryptionKey, EncryptedData, CredentialMetadata) {
    let key = EncryptionKey::from_bytes([1u8; 32]);
    let plaintext = b"test secret";
    let encrypted = encrypt(&key, plaintext).expect("encryption should succeed");
    let metadata = CredentialMetadata::new();
    (key, encrypted, metadata)
}

/// Test: Store credential and retrieve it
#[tokio::test]
async fn test_mock_provider_store_and_retrieve() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("test_cred").unwrap();
    let context = CredentialContext::new("user_123");
    let (_, encrypted, metadata) = create_test_data();

    // Store credential
    provider
        .store(&id, encrypted.clone(), metadata.clone(), &context)
        .await
        .expect("store should succeed");

    // Retrieve credential
    let (retrieved_data, retrieved_meta) = provider
        .retrieve(&id, &context)
        .await
        .expect("retrieve should succeed");

    // Verify data matches
    assert_eq!(retrieved_data.version, encrypted.version);
    assert_eq!(retrieved_data.nonce, encrypted.nonce);
    assert_eq!(retrieved_data.ciphertext, encrypted.ciphertext);
    assert_eq!(retrieved_data.tag, encrypted.tag);

    // Verify metadata
    assert_eq!(retrieved_meta.created_at, metadata.created_at);
}

/// Test: Delete is idempotent
#[tokio::test]
async fn test_mock_provider_delete_idempotent() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("delete_test").unwrap();
    let context = CredentialContext::new("user_123");
    let (_, encrypted, metadata) = create_test_data();

    // Store credential
    provider
        .store(&id, encrypted, metadata, &context)
        .await
        .expect("store should succeed");

    // Delete once
    provider
        .delete(&id, &context)
        .await
        .expect("first delete should succeed");

    // Delete again (idempotent)
    provider
        .delete(&id, &context)
        .await
        .expect("second delete should succeed");

    // Verify credential is gone
    let exists = provider.exists(&id, &context).await.unwrap();
    assert!(!exists);
}

/// Test: List with no credentials returns empty vec
#[tokio::test]
async fn test_mock_provider_list_empty() {
    let provider = MockStorageProvider::new();
    let context = CredentialContext::new("user_123");

    let ids = provider
        .list(None, &context)
        .await
        .expect("list should succeed");

    assert_eq!(ids.len(), 0);
}

/// Test: exists() returns correct values
#[tokio::test]
async fn test_mock_provider_exists() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("exists_test").unwrap();
    let context = CredentialContext::new("user_123");
    let (_, encrypted, metadata) = create_test_data();

    // Before store
    let exists = provider.exists(&id, &context).await.unwrap();
    assert!(!exists, "should not exist before store");

    // After store
    provider
        .store(&id, encrypted, metadata, &context)
        .await
        .unwrap();
    let exists = provider.exists(&id, &context).await.unwrap();
    assert!(exists, "should exist after store");

    // After delete
    provider.delete(&id, &context).await.unwrap();
    let exists = provider.exists(&id, &context).await.unwrap();
    assert!(!exists, "should not exist after delete");
}

/// Test: Retrieve non-existent credential returns NotFound
#[tokio::test]
async fn test_mock_provider_retrieve_nonexistent() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("nonexistent").unwrap();
    let context = CredentialContext::new("user_123");

    let result = provider.retrieve(&id, &context).await;

    assert!(result.is_err());
    match result {
        Err(StorageError::NotFound { id: err_id }) => {
            assert_eq!(err_id, "nonexistent");
        }
        _ => panic!("Expected NotFound error"),
    }
}

/// Test: Concurrent writes (last write wins)
#[tokio::test]
async fn test_mock_provider_concurrent_writes() {
    let provider = Arc::new(MockStorageProvider::new());
    let id = CredentialId::new("concurrent_test").unwrap();
    let context = CredentialContext::new("user_123");

    // Spawn multiple concurrent writes
    let mut handles = vec![];
    for i in 0..10 {
        let provider_clone = Arc::clone(&provider);
        let id_clone = id.clone();
        let context_clone = context.clone();

        let handle = tokio::spawn(async move {
            let key = EncryptionKey::from_bytes([i as u8; 32]);
            let plaintext = format!("secret_{}", i);
            let encrypted = encrypt(&key, plaintext.as_bytes()).unwrap();
            let mut metadata = CredentialMetadata::new();
            metadata
                .tags
                .insert("write_number".to_string(), i.to_string());

            provider_clone
                .store(&id_clone, encrypted, metadata, &context_clone)
                .await
                .unwrap();
        });
        handles.push(handle);
    }

    // Wait for all writes to complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Verify exactly one credential exists (last write won)
    let (_, metadata) = provider.retrieve(&id, &context).await.unwrap();

    // Verify it has a write_number tag (proves it's from one of the writes)
    assert!(metadata.tags.contains_key("write_number"));

    // Verify no corruption (can successfully retrieve)
    let exists = provider.exists(&id, &context).await.unwrap();
    assert!(exists);
}

/// Test: List with tag filter
#[tokio::test]
async fn test_mock_provider_list_with_filter() {
    let provider = MockStorageProvider::new();
    let context = CredentialContext::new("user_123");

    // Store credentials with different tags
    for i in 1..=5 {
        let id = CredentialId::new(format!("cred_{}", i)).unwrap();
        let (_, encrypted, mut metadata) = create_test_data();

        if i % 2 == 0 {
            metadata
                .tags
                .insert("environment".to_string(), "production".to_string());
        } else {
            metadata
                .tags
                .insert("environment".to_string(), "development".to_string());
        }

        provider
            .store(&id, encrypted, metadata, &context)
            .await
            .unwrap();
    }

    // List all
    let all_ids = provider.list(None, &context).await.unwrap();
    assert_eq!(all_ids.len(), 5);

    // List with filter (production only)
    let filter = CredentialFilter::new().with_tag("environment", "production");
    let prod_ids = provider.list(Some(&filter), &context).await.unwrap();
    assert_eq!(prod_ids.len(), 2);
    assert!(prod_ids.iter().all(|id| {
        let id_str = id.as_str();
        id_str == "cred_2" || id_str == "cred_4"
    }));
}
