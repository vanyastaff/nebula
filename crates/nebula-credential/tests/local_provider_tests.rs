//! Unit tests for LocalStorageProvider

use nebula_credential::core::{
    CredentialContext, CredentialFilter, CredentialId, CredentialMetadata,
};
use nebula_credential::providers::{LocalStorageConfig, LocalStorageProvider};
use nebula_credential::traits::StorageProvider;
use nebula_credential::utils::{EncryptionKey, encrypt};
use std::collections::HashMap;
use tempfile::TempDir;
use tokio;

/// Helper to create a test provider with temporary directory
fn create_test_provider() -> (LocalStorageProvider, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig::new(temp_dir.path());
    let provider = LocalStorageProvider::new(config);
    (provider, temp_dir)
}

#[tokio::test]
async fn test_store_and_retrieve() {
    let (provider, _temp_dir) = create_test_provider();
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("test_cred").unwrap();

    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret_value").unwrap();

    let mut tags = HashMap::new();
    tags.insert("env".to_string(), "test".to_string());
    let mut metadata = CredentialMetadata::new();
    metadata.tags = tags;

    // Store
    provider
        .store(&id, data.clone(), metadata.clone(), &context)
        .await
        .unwrap();

    // Retrieve
    let (retrieved_data, retrieved_metadata) = provider.retrieve(&id, &context).await.unwrap();

    assert_eq!(retrieved_data.version, data.version);
    assert_eq!(retrieved_data.nonce, data.nonce);
    assert_eq!(retrieved_data.ciphertext, data.ciphertext);
    assert_eq!(retrieved_data.tag, data.tag);
    assert_eq!(retrieved_metadata.tags, metadata.tags);
}

#[tokio::test]
async fn test_retrieve_not_found() {
    let (provider, _temp_dir) = create_test_provider();
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("nonexistent").unwrap();

    let result = provider.retrieve(&id, &context).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        nebula_credential::core::StorageError::NotFound { id: err_id } => {
            assert_eq!(err_id, "nonexistent");
        }
        _ => panic!("Expected NotFound error"),
    }
}

#[tokio::test]
async fn test_delete_idempotent() {
    let (provider, _temp_dir) = create_test_provider();
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("test_cred").unwrap();

    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret_value").unwrap();
    let metadata = CredentialMetadata::default();

    // Store
    provider.store(&id, data, metadata, &context).await.unwrap();

    // Delete once
    provider.delete(&id, &context).await.unwrap();

    // Delete again (should succeed)
    provider.delete(&id, &context).await.unwrap();

    // Verify deleted
    assert!(!provider.exists(&id, &context).await.unwrap());
}

#[tokio::test]
async fn test_list_with_filter() {
    let (provider, _temp_dir) = create_test_provider();
    let context = CredentialContext::new("test_user");
    let key = EncryptionKey::from_bytes([42u8; 32]);

    // Store credentials with different tags
    for i in 0..5 {
        let id = CredentialId::new(&format!("cred_{}", i)).unwrap();
        let data = encrypt(&key, format!("secret_{}", i).as_bytes()).unwrap();

        let mut tags = HashMap::new();
        tags.insert(
            "env".to_string(),
            if i < 3 {
                "prod".to_string()
            } else {
                "dev".to_string()
            },
        );
        let mut metadata = CredentialMetadata::new();
        metadata.tags = tags;

        provider.store(&id, data, metadata, &context).await.unwrap();
    }

    // Filter by env=prod
    let mut filter_tags = HashMap::new();
    filter_tags.insert("env".to_string(), "prod".to_string());
    let filter = CredentialFilter {
        tags: Some(filter_tags),
        created_after: None,
        created_before: None,
    };

    let ids = provider.list(Some(&filter), &context).await.unwrap();

    assert_eq!(ids.len(), 3);
    for id in &ids {
        assert!(
            id.as_str().starts_with("cred_0")
                || id.as_str().starts_with("cred_1")
                || id.as_str().starts_with("cred_2")
        );
    }
}

#[tokio::test]
async fn test_list_without_filter() {
    let (provider, _temp_dir) = create_test_provider();
    let context = CredentialContext::new("test_user");
    let key = EncryptionKey::from_bytes([42u8; 32]);

    // Store 10 credentials
    for i in 0..10 {
        let id = CredentialId::new(&format!("cred_{}", i)).unwrap();
        let data = encrypt(&key, format!("secret_{}", i).as_bytes()).unwrap();
        let metadata = CredentialMetadata::default();

        provider.store(&id, data, metadata, &context).await.unwrap();
    }

    // List all
    let ids = provider.list(None, &context).await.unwrap();

    assert_eq!(ids.len(), 10);
}

#[tokio::test]
async fn test_exists() {
    let (provider, _temp_dir) = create_test_provider();
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("test_cred").unwrap();

    // Initially doesn't exist
    assert!(!provider.exists(&id, &context).await.unwrap());

    // Store
    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret_value").unwrap();
    let metadata = CredentialMetadata::default();
    provider.store(&id, data, metadata, &context).await.unwrap();

    // Now exists
    assert!(provider.exists(&id, &context).await.unwrap());

    // Delete
    provider.delete(&id, &context).await.unwrap();

    // No longer exists
    assert!(!provider.exists(&id, &context).await.unwrap());
}

#[tokio::test]
async fn test_directory_autocreate() {
    let temp_dir = TempDir::new().unwrap();
    let nested_path = temp_dir.path().join("nested").join("dir");

    let config = LocalStorageConfig::new(&nested_path);
    let provider = LocalStorageProvider::new(config);
    let context = CredentialContext::new("test_user");

    // Directory should be created on first store
    let id = CredentialId::new("test_cred").unwrap();
    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret_value").unwrap();
    let metadata = CredentialMetadata::default();

    provider.store(&id, data, metadata, &context).await.unwrap();

    // Verify directory exists
    assert!(nested_path.exists());
}

#[tokio::test]
async fn test_overwrite_existing_credential() {
    let (provider, _temp_dir) = create_test_provider();
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("test_cred").unwrap();
    let key = EncryptionKey::from_bytes([42u8; 32]);

    // Store first value
    let data1 = encrypt(&key, b"secret_value_1").unwrap();
    let metadata1 = CredentialMetadata::default();
    provider
        .store(&id, data1, metadata1, &context)
        .await
        .unwrap();

    // Overwrite with second value
    let data2 = encrypt(&key, b"secret_value_2").unwrap();
    let mut tags = HashMap::new();
    tags.insert("version".to_string(), "2".to_string());
    let mut metadata2 = CredentialMetadata::new();
    metadata2.tags = tags.clone();
    provider
        .store(&id, data2.clone(), metadata2, &context)
        .await
        .unwrap();

    // Retrieve should get second value
    let (retrieved_data, retrieved_metadata) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(retrieved_data.ciphertext, data2.ciphertext);
    assert_eq!(retrieved_metadata.tags, tags);
}

#[tokio::test]
#[cfg(unix)]
async fn test_file_permissions_unix() {
    use std::os::unix::fs::PermissionsExt;

    let (provider, temp_dir) = create_test_provider();
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("test_cred").unwrap();

    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret_value").unwrap();
    let metadata = CredentialMetadata::default();

    provider.store(&id, data, metadata, &context).await.unwrap();

    // Check directory permissions (0700)
    let dir_metadata = std::fs::metadata(temp_dir.path()).unwrap();
    let dir_mode = dir_metadata.permissions().mode();
    assert_eq!(
        dir_mode & 0o777,
        0o700,
        "Directory should have 0700 permissions"
    );

    // Check file permissions (0600)
    let file_path = temp_dir.path().join("test_cred.cred");
    let file_metadata = std::fs::metadata(&file_path).unwrap();
    let file_mode = file_metadata.permissions().mode();
    assert_eq!(
        file_mode & 0o777,
        0o600,
        "File should have 0600 permissions"
    );
}
