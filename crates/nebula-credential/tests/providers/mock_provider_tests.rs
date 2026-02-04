//! Unit tests for MockStorageProvider

use nebula_credential::core::{
    CredentialContext, CredentialFilter, CredentialId, CredentialMetadata, StorageError,
};
use nebula_credential::providers::MockStorageProvider;
use nebula_credential::traits::StorageProvider;
use nebula_credential::utils::{EncryptionKey, encrypt};
use std::collections::HashMap;

#[tokio::test]
async fn test_store_and_retrieve() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("test_cred").unwrap();
    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret_value").unwrap();

    let mut tags = HashMap::new();
    tags.insert("env".to_string(), "test".to_string());

    let metadata = CredentialMetadata {
        tags,
        ..Default::default()
    };
    let context = CredentialContext::new("user_123");

    // Store
    provider
        .store(&id, data.clone(), metadata.clone(), &context)
        .await
        .unwrap();

    // Retrieve
    let (retrieved_data, retrieved_metadata) = provider.retrieve(&id, &context).await.unwrap();

    assert_eq!(data.ciphertext, retrieved_data.ciphertext);
    assert_eq!(data.nonce, retrieved_data.nonce);
    assert_eq!(metadata.tags, retrieved_metadata.tags);
}

#[tokio::test]
async fn test_retrieve_not_found() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("nonexistent").unwrap();
    let context = CredentialContext::new("user_123");

    let result = provider.retrieve(&id, &context).await;

    assert!(matches!(result, Err(StorageError::NotFound { .. })));
}

#[tokio::test]
async fn test_delete_idempotent() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("test_cred").unwrap();
    let context = CredentialContext::new("user_123");

    // Delete non-existent credential should succeed
    provider.delete(&id, &context).await.unwrap();

    // Store then delete
    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret").unwrap();
    let metadata = CredentialMetadata::default();
    provider.store(&id, data, metadata, &context).await.unwrap();

    // First delete
    provider.delete(&id, &context).await.unwrap();

    // Verify deleted
    assert_eq!(provider.count().await, 0);

    // Second delete should also succeed (idempotent)
    provider.delete(&id, &context).await.unwrap();
}

#[tokio::test]
async fn test_list_with_filter() {
    let provider = MockStorageProvider::new();
    let context = CredentialContext::new("user_123");
    let key = EncryptionKey::from_bytes([42u8; 32]);

    // Store credentials with different tags
    let cred1 = CredentialId::new("cred1").unwrap();
    let mut tags1 = HashMap::new();
    tags1.insert("env".to_string(), "prod".to_string());
    tags1.insert("type".to_string(), "api".to_string());
    let metadata1 = CredentialMetadata {
        tags: tags1,
        ..Default::default()
    };
    provider
        .store(
            &cred1,
            encrypt(&key, b"data1").unwrap(),
            metadata1,
            &context,
        )
        .await
        .unwrap();

    let cred2 = CredentialId::new("cred2").unwrap();
    let mut tags2 = HashMap::new();
    tags2.insert("env".to_string(), "dev".to_string());
    tags2.insert("type".to_string(), "api".to_string());
    let metadata2 = CredentialMetadata {
        tags: tags2,
        ..Default::default()
    };
    provider
        .store(
            &cred2,
            encrypt(&key, b"data2").unwrap(),
            metadata2,
            &context,
        )
        .await
        .unwrap();

    // List with filter for prod
    let mut filter_tags = HashMap::new();
    filter_tags.insert("env".to_string(), "prod".to_string());
    let filter = CredentialFilter {
        tags: Some(filter_tags),
        ..Default::default()
    };
    let ids = provider.list(Some(&filter), &context).await.unwrap();

    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].as_str(), "cred1");
}

#[tokio::test]
async fn test_list_without_filter() {
    let provider = MockStorageProvider::new();
    let context = CredentialContext::new("user_123");
    let key = EncryptionKey::from_bytes([42u8; 32]);

    // Store multiple credentials
    for i in 0..5 {
        let id = CredentialId::new(&format!("cred{}", i)).unwrap();
        let data = encrypt(&key, format!("secret{}", i).as_bytes()).unwrap();
        let metadata = CredentialMetadata::default();
        provider.store(&id, data, metadata, &context).await.unwrap();
    }

    // List all
    let ids = provider.list(None, &context).await.unwrap();

    assert_eq!(ids.len(), 5);
}

#[tokio::test]
async fn test_exists() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("test_cred").unwrap();
    let context = CredentialContext::new("user_123");

    // Should not exist initially
    assert!(!provider.exists(&id, &context).await.unwrap());

    // Store credential
    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret").unwrap();
    let metadata = CredentialMetadata::default();
    provider.store(&id, data, metadata, &context).await.unwrap();

    // Should exist now
    assert!(provider.exists(&id, &context).await.unwrap());

    // Delete it
    provider.delete(&id, &context).await.unwrap();

    // Should not exist again
    assert!(!provider.exists(&id, &context).await.unwrap());
}

#[tokio::test]
async fn test_simulated_error() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("test_cred").unwrap();
    let context = CredentialContext::new("user_123");

    // Configure mock to fail next operation
    provider
        .fail_next_with(StorageError::PermissionDenied {
            id: "test_credential".into(),
        })
        .await;

    // Next operation should fail
    let result = provider.retrieve(&id, &context).await;
    assert!(matches!(result, Err(StorageError::PermissionDenied { .. })));

    // Subsequent operations should succeed (error was one-time)
    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret").unwrap();
    let metadata = CredentialMetadata::default();
    provider.store(&id, data, metadata, &context).await.unwrap();
}

#[tokio::test]
async fn test_concurrent_operations() {
    use tokio::task::JoinSet;

    let provider = MockStorageProvider::new();
    let context = CredentialContext::new("user_123");

    let mut set = JoinSet::new();

    // Spawn 100 concurrent store operations
    for i in 0..100 {
        let provider = provider.clone();
        let context = context.clone();
        let key = EncryptionKey::from_bytes([42u8; 32]);

        set.spawn(async move {
            let id = CredentialId::new(&format!("cred_{}", i)).unwrap();
            let data = encrypt(&key, format!("secret_{}", i).as_bytes()).unwrap();
            let metadata = CredentialMetadata::default();
            provider.store(&id, data, metadata, &context).await
        });
    }

    // All operations should succeed
    let mut count = 0;
    while let Some(result) = set.join_next().await {
        result.unwrap().unwrap();
        count += 1;
    }

    assert_eq!(count, 100);
    assert_eq!(provider.count().await, 100);
}

#[tokio::test]
async fn test_clear() {
    let provider = MockStorageProvider::new();
    let context = CredentialContext::new("user_123");
    let key = EncryptionKey::from_bytes([42u8; 32]);

    // Store some credentials
    for i in 0..5 {
        let id = CredentialId::new(&format!("cred{}", i)).unwrap();
        let data = encrypt(&key, format!("secret{}", i).as_bytes()).unwrap();
        let metadata = CredentialMetadata::default();
        provider.store(&id, data, metadata, &context).await.unwrap();
    }

    assert_eq!(provider.count().await, 5);

    // Clear all
    provider.clear().await;

    assert_eq!(provider.count().await, 0);

    // List should return empty
    let ids = provider.list(None, &context).await.unwrap();
    assert_eq!(ids.len(), 0);
}

#[tokio::test]
async fn test_overwrite_existing_credential() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("test_cred").unwrap();
    let context = CredentialContext::new("user_123");
    let key = EncryptionKey::from_bytes([42u8; 32]);

    // Store initial credential
    let data1 = encrypt(&key, b"secret1").unwrap();
    let mut tags1 = HashMap::new();
    tags1.insert("version".to_string(), "1".to_string());
    let metadata1 = CredentialMetadata {
        tags: tags1,
        ..Default::default()
    };
    provider
        .store(&id, data1, metadata1, &context)
        .await
        .unwrap();

    // Overwrite with new credential
    let data2 = encrypt(&key, b"secret2").unwrap();
    let mut tags2 = HashMap::new();
    tags2.insert("version".to_string(), "2".to_string());
    let metadata2 = CredentialMetadata {
        tags: tags2.clone(),
        ..Default::default()
    };
    provider
        .store(&id, data2.clone(), metadata2.clone(), &context)
        .await
        .unwrap();

    // Should still have only one credential
    assert_eq!(provider.count().await, 1);

    // Retrieve should return the latest version
    let (retrieved_data, retrieved_metadata) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(data2.ciphertext, retrieved_data.ciphertext);
    assert_eq!(tags2, retrieved_metadata.tags);
}
