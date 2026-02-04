//! Integration tests for LocalStorageProvider
//!
//! These tests verify real-world scenarios including concurrent access,
//! atomic writes, file permissions, and error recovery.

use nebula_credential::core::{CredentialContext, CredentialId, CredentialMetadata};
use nebula_credential::providers::{LocalStorageConfig, LocalStorageProvider};
use nebula_credential::traits::StorageProvider;
use nebula_credential::utils::{EncryptionKey, encrypt};
use std::collections::HashMap;
use tempfile::TempDir;
use tokio::task::JoinSet;

/// Test atomic writes prevent corruption during concurrent access
#[tokio::test]
async fn test_atomic_writes_no_corruption() {
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig::new(temp_dir.path());
    let provider = LocalStorageProvider::new(config);
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("test_cred").unwrap();

    // Write the same credential 100 times concurrently
    let mut set = JoinSet::new();
    for i in 0..100 {
        let provider = provider.clone();
        let context = context.clone();
        let id = id.clone();
        let key = EncryptionKey::from_bytes([42u8; 32]);

        set.spawn(async move {
            let data = encrypt(&key, format!("secret_{}", i).as_bytes()).unwrap();
            let metadata = CredentialMetadata::default();
            provider.store(&id, data, metadata, &context).await
        });
    }

    // All writes should succeed
    let mut success_count = 0;
    while let Some(result) = set.join_next().await {
        if result.unwrap().is_ok() {
            success_count += 1;
        }
    }

    assert_eq!(success_count, 100);

    // Final credential should be readable and valid
    let (retrieved_data, _) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(retrieved_data.version, 1);

    // Verify no temp files left behind
    let entries = std::fs::read_dir(temp_dir.path()).unwrap();
    for entry in entries {
        let path = entry.unwrap().path();
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(!filename.contains(".tmp."), "Found temp file: {}", filename);
    }
}

/// Test Unix file permissions are set correctly
#[tokio::test]
#[cfg(unix)]
async fn test_unix_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig::new(temp_dir.path());
    let provider = LocalStorageProvider::new(config);
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("test_cred").unwrap();

    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret_value").unwrap();
    let metadata = CredentialMetadata::default();

    provider.store(&id, data, metadata, &context).await.unwrap();

    // Check directory permissions (0700 = rwx------)
    let dir_metadata = std::fs::metadata(temp_dir.path()).unwrap();
    let dir_mode = dir_metadata.permissions().mode();
    assert_eq!(
        dir_mode & 0o777,
        0o700,
        "Directory should have 0700 permissions, got {:o}",
        dir_mode & 0o777
    );

    // Check file permissions (0600 = rw-------)
    let file_path = temp_dir.path().join("test_cred.cred");
    let file_metadata = std::fs::metadata(&file_path).unwrap();
    let file_mode = file_metadata.permissions().mode();
    assert_eq!(
        file_mode & 0o777,
        0o600,
        "File should have 0600 permissions, got {:o}",
        file_mode & 0o777
    );
}

/// Test Windows ACL (placeholder for future implementation)
#[tokio::test]
#[cfg(windows)]
async fn test_windows_acl() {
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig::new(temp_dir.path());
    let provider = LocalStorageProvider::new(config);
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("test_cred").unwrap();

    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret_value").unwrap();
    let metadata = CredentialMetadata::default();

    provider.store(&id, data, metadata, &context).await.unwrap();

    // Verify file exists
    let file_path = temp_dir.path().join("test_cred.cred");
    assert!(file_path.exists());

    // TODO: Add Windows ACL verification when implemented
    // For now, just verify the file was created
}

/// Test concurrent writes to different credentials
#[tokio::test]
async fn test_concurrent_writes_different_credentials() {
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig::new(temp_dir.path());
    let provider = LocalStorageProvider::new(config);
    let context = CredentialContext::new("test_user");

    // Write 50 different credentials concurrently
    let mut set = JoinSet::new();
    for i in 0..50 {
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

    // All writes should succeed
    let mut success_count = 0;
    while let Some(result) = set.join_next().await {
        if result.unwrap().is_ok() {
            success_count += 1;
        }
    }

    assert_eq!(success_count, 50);

    // Verify all credentials are readable
    for i in 0..50 {
        let id = CredentialId::new(&format!("cred_{}", i)).unwrap();
        let (data, _) = provider.retrieve(&id, &context).await.unwrap();
        assert_eq!(data.version, 1);
    }
}

/// Test file corruption recovery (invalid JSON)
#[tokio::test]
async fn test_file_corruption_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig::new(temp_dir.path());
    let provider = LocalStorageProvider::new(config);
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("corrupted_cred").unwrap();

    // Create directory
    std::fs::create_dir_all(temp_dir.path()).unwrap();

    // Write corrupted data directly
    let file_path = temp_dir.path().join("corrupted_cred.cred");
    std::fs::write(&file_path, b"{ invalid json }").unwrap();

    // Attempt to retrieve should fail with ReadFailure
    let result = provider.retrieve(&id, &context).await;
    assert!(result.is_err());
    match result.unwrap_err() {
        nebula_credential::core::StorageError::ReadFailure { id: err_id, .. } => {
            assert_eq!(err_id, "corrupted_cred");
        }
        _ => panic!("Expected ReadFailure error"),
    }

    // Should be able to overwrite corrupted file
    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"new_secret").unwrap();
    let metadata = CredentialMetadata::default();

    provider
        .store(&id, data.clone(), metadata, &context)
        .await
        .unwrap();

    // Now retrieval should work
    let (retrieved_data, _) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(retrieved_data.ciphertext, data.ciphertext);
}

/// Test directory autocreate with deeply nested paths
#[tokio::test]
async fn test_directory_autocreate_nested() {
    let temp_dir = TempDir::new().unwrap();
    let nested_path = temp_dir
        .path()
        .join("level1")
        .join("level2")
        .join("level3")
        .join("credentials");

    let config = LocalStorageConfig::new(&nested_path);
    let provider = LocalStorageProvider::new(config);
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("test_cred").unwrap();

    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"secret_value").unwrap();
    let metadata = CredentialMetadata::default();

    // Should create all nested directories
    provider.store(&id, data, metadata, &context).await.unwrap();

    // Verify directory structure was created
    assert!(nested_path.exists());
    assert!(nested_path.join("test_cred.cred").exists());
}

/// Test concurrent reads don't interfere with writes
#[tokio::test]
async fn test_concurrent_reads_and_writes() {
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig::new(temp_dir.path());
    let provider = LocalStorageProvider::new(config);
    let context = CredentialContext::new("test_user");
    let id = CredentialId::new("test_cred").unwrap();

    // Initial write
    let key = EncryptionKey::from_bytes([42u8; 32]);
    let data = encrypt(&key, b"initial_value").unwrap();
    let metadata = CredentialMetadata::default();
    provider.store(&id, data, metadata, &context).await.unwrap();

    let mut set = JoinSet::new();

    // Spawn 25 readers
    for _ in 0..25 {
        let provider = provider.clone();
        let context = context.clone();
        let id = id.clone();

        set.spawn(async move { provider.retrieve(&id, &context).await });
    }

    // Spawn 25 writers
    for i in 0..25 {
        let provider = provider.clone();
        let context = context.clone();
        let id = id.clone();
        let key = EncryptionKey::from_bytes([42u8; 32]);

        set.spawn(async move {
            let data = encrypt(&key, format!("value_{}", i).as_bytes()).unwrap();
            let metadata = CredentialMetadata::default();
            provider
                .store(&id, data, metadata, &context)
                .await
                .map(|_| {
                    (
                        encrypt(&key, b"dummy").unwrap(),
                        CredentialMetadata::default(),
                    )
                })
        });
    }

    // All operations should succeed (reads might get old or new values, both valid)
    let mut success_count = 0;
    while let Some(result) = set.join_next().await {
        if result.unwrap().is_ok() {
            success_count += 1;
        }
    }

    assert_eq!(success_count, 50);
}

/// Test list operation with large number of credentials
#[tokio::test]
async fn test_list_performance_many_credentials() {
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig::new(temp_dir.path());
    let provider = LocalStorageProvider::new(config);
    let context = CredentialContext::new("test_user");
    let key = EncryptionKey::from_bytes([42u8; 32]);

    // Store 100 credentials
    for i in 0..100 {
        let id = CredentialId::new(&format!("cred_{:03}", i)).unwrap();
        let data = encrypt(&key, format!("secret_{}", i).as_bytes()).unwrap();

        let mut tags = HashMap::new();
        tags.insert("batch".to_string(), (i / 10).to_string());
        let mut metadata = CredentialMetadata::new();
        metadata.tags = tags;

        provider.store(&id, data, metadata, &context).await.unwrap();
    }

    // List all
    let start = std::time::Instant::now();
    let all_ids = provider.list(None, &context).await.unwrap();
    let list_duration = start.elapsed();

    assert_eq!(all_ids.len(), 100);
    assert!(
        list_duration.as_millis() < 1000,
        "List operation took {}ms, should be under 1000ms",
        list_duration.as_millis()
    );

    // List with filter
    let mut filter_tags = HashMap::new();
    filter_tags.insert("batch".to_string(), "5".to_string());
    let filter = nebula_credential::core::CredentialFilter {
        tags: Some(filter_tags),
        created_after: None,
        created_before: None,
    };

    let filtered_ids = provider.list(Some(&filter), &context).await.unwrap();
    assert_eq!(filtered_ids.len(), 10); // batch 5 contains cred_050 to cred_059
}

/// Test storage with special characters in credential IDs
#[tokio::test]
async fn test_special_characters_in_ids() {
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig::new(temp_dir.path());
    let provider = LocalStorageProvider::new(config);
    let context = CredentialContext::new("test_user");
    let key = EncryptionKey::from_bytes([42u8; 32]);

    // Valid characters: alphanumeric, hyphens, underscores
    let test_ids = vec![
        "simple-id",
        "id_with_underscores",
        "id-with-hyphens",
        "MixedCase123",
        "id123",
        "a",
        "very_long_credential_id_with_many_characters_1234567890",
    ];

    for id_str in test_ids {
        let id = CredentialId::new(id_str).unwrap();
        let data = encrypt(&key, id_str.as_bytes()).unwrap();
        let metadata = CredentialMetadata::default();

        provider
            .store(&id, data.clone(), metadata, &context)
            .await
            .unwrap();

        let (retrieved_data, _) = provider.retrieve(&id, &context).await.unwrap();
        assert_eq!(retrieved_data.ciphertext, data.ciphertext);
    }
}
