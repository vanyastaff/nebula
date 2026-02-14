//! Integration tests for CredentialManager CRUD operations
//!
//! These tests verify the core functionality of storing, retrieving, deleting,
//! and listing credentials through the CredentialManager API.

use nebula_credential::prelude::*;
use std::sync::Arc;
use std::time::Duration;

/// Helper to create a test manager with mock storage
async fn create_test_manager() -> CredentialManager {
    let storage = MockStorageProvider::new();
    CredentialManager::builder()
        .storage(Arc::new(storage))
        .build()
}

/// Helper to create a test manager with caching enabled
async fn create_cached_manager() -> CredentialManager {
    let storage = MockStorageProvider::new();
    let cache_config = CacheConfig {
        enabled: true,
        ttl: Some(Duration::from_secs(300)),
        idle_timeout: None,
        max_capacity: 100,
        eviction_strategy: EvictionStrategy::Lru,
    };

    CredentialManager::builder()
        .storage(Arc::new(storage))
        .cache_config(cache_config)
        .build()
}

/// Helper to create test encrypted data
fn create_test_data(value: &str) -> EncryptedData {
    let key = EncryptionKey::from_bytes([0u8; 32]);
    encrypt(&key, value.as_bytes()).unwrap()
}

/// Helper to create test metadata
fn create_test_metadata() -> CredentialMetadata {
    CredentialMetadata::new()
}

#[tokio::test]
async fn test_store_and_retrieve() {
    // GIVEN: A credential manager and a credential to store
    let manager = create_test_manager().await;
    let id = CredentialId::new("test-credential").unwrap();
    let data = create_test_data("secret-value");
    let metadata = create_test_metadata();
    let context = CredentialContext::new("test-user");

    // WHEN: We store the credential
    let store_result = manager
        .store(&id, data.clone(), metadata.clone(), &context)
        .await;

    // THEN: The store operation succeeds
    assert!(store_result.is_ok(), "Store operation should succeed");

    // WHEN: We retrieve the credential
    let retrieve_result = manager.retrieve(&id, &context).await;

    // THEN: The retrieve operation succeeds and returns the correct data
    assert!(retrieve_result.is_ok(), "Retrieve operation should succeed");
    let retrieved = retrieve_result.unwrap();
    assert!(retrieved.is_some(), "Credential should exist");

    let (retrieved_data, retrieved_metadata) = retrieved.unwrap();
    assert_eq!(
        retrieved_data.ciphertext, data.ciphertext,
        "Ciphertext should match"
    );
    assert_eq!(
        retrieved_metadata.created_at, metadata.created_at,
        "Created timestamp should match"
    );
}

#[tokio::test]
async fn test_retrieve_nonexistent() {
    // GIVEN: A credential manager with no stored credentials
    let manager = create_test_manager().await;
    let id = CredentialId::new("nonexistent").unwrap();
    let context = CredentialContext::new("test-user");

    // WHEN: We try to retrieve a non-existent credential
    let result = manager.retrieve(&id, &context).await;

    // THEN: The operation succeeds but returns None
    assert!(result.is_ok(), "Retrieve operation should succeed");
    assert!(
        result.unwrap().is_none(),
        "Should return None for non-existent credential"
    );
}

#[tokio::test]
async fn test_delete_credential() {
    // GIVEN: A credential manager with a stored credential
    let manager = create_test_manager().await;
    let id = CredentialId::new("to-delete").unwrap();
    let data = create_test_data("secret");
    let metadata = create_test_metadata();
    let context = CredentialContext::new("test-user");

    // Store the credential first
    manager
        .store(&id, data, metadata, &context)
        .await
        .expect("Store should succeed");

    // Verify it exists
    let exists = manager.retrieve(&id, &context).await.unwrap();
    assert!(exists.is_some(), "Credential should exist before deletion");

    // WHEN: We delete the credential
    let delete_result = manager.delete(&id, &context).await;

    // THEN: The delete operation succeeds
    assert!(delete_result.is_ok(), "Delete operation should succeed");

    // AND: The credential no longer exists
    let after_delete = manager.retrieve(&id, &context).await.unwrap();
    assert!(
        after_delete.is_none(),
        "Credential should not exist after deletion"
    );
}

#[tokio::test]
async fn test_list_credentials() {
    // GIVEN: A credential manager with multiple stored credentials
    let manager = create_test_manager().await;
    let context = CredentialContext::new("test-user");

    let id1 = CredentialId::new("cred-1").unwrap();
    let id2 = CredentialId::new("cred-2").unwrap();
    let id3 = CredentialId::new("cred-3").unwrap();

    let data = create_test_data("secret");
    let metadata = create_test_metadata();

    // Store multiple credentials
    manager
        .store(&id1, data.clone(), metadata.clone(), &context)
        .await
        .unwrap();
    manager
        .store(&id2, data.clone(), metadata.clone(), &context)
        .await
        .unwrap();
    manager.store(&id3, data, metadata, &context).await.unwrap();

    // WHEN: We list all credentials
    let result = manager.list(&context).await;

    // THEN: The list operation succeeds and returns all credential IDs
    assert!(result.is_ok(), "List operation should succeed");
    let ids = result.unwrap();
    assert_eq!(ids.len(), 3, "Should return 3 credentials");
    assert!(ids.contains(&id1), "Should contain cred-1");
    assert!(ids.contains(&id2), "Should contain cred-2");
    assert!(ids.contains(&id3), "Should contain cred-3");
}

#[tokio::test]
async fn test_store_duplicate_id() {
    // GIVEN: A credential manager with a stored credential
    let manager = create_test_manager().await;
    let id = CredentialId::new("duplicate").unwrap();
    let context = CredentialContext::new("test-user");

    let data1 = create_test_data("first-value");
    let metadata1 = create_test_metadata();

    // Store the first credential
    manager
        .store(&id, data1, metadata1, &context)
        .await
        .expect("First store should succeed");

    // WHEN: We store a different credential with the same ID
    let data2 = create_test_data("second-value");
    let metadata2 = create_test_metadata();

    let result = manager.store(&id, data2.clone(), metadata2, &context).await;

    // THEN: The store operation succeeds (overwrites the previous credential)
    assert!(result.is_ok(), "Second store should succeed (overwrite)");

    // AND: Retrieving returns the latest value
    let retrieved = manager.retrieve(&id, &context).await.unwrap().unwrap();
    assert_eq!(
        retrieved.0.ciphertext, data2.ciphertext,
        "Should return the second (latest) value"
    );
}

#[tokio::test]
async fn test_cache_hit_on_second_retrieve() {
    // GIVEN: A credential manager with caching enabled
    let manager = create_cached_manager().await;
    let id = CredentialId::new("cached-cred").unwrap();
    let data = create_test_data("cached-secret");
    let metadata = create_test_metadata();
    let context = CredentialContext::new("test-user");

    // Store credential
    manager.store(&id, data, metadata, &context).await.unwrap();

    // WHEN: We retrieve the credential twice
    manager.retrieve(&id, &context).await.unwrap();
    manager.retrieve(&id, &context).await.unwrap();

    // THEN: Cache stats show a hit
    let stats = manager.cache_stats();
    assert!(stats.is_some(), "Cache stats should be available");
    let stats = stats.unwrap();
    assert!(stats.hits > 0, "Should have at least one cache hit");
}
