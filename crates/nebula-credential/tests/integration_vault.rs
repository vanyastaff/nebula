//! Integration tests for HashiCorp Vault provider with testcontainers
//!
//! These tests automatically start and manage a Vault container.
//! No external docker-compose required.

use nebula_credential::core::{CredentialContext, CredentialId, CredentialMetadata};
use nebula_credential::providers::{HashiCorpVaultProvider, VaultAuthMethod, VaultConfig};
use nebula_credential::traits::StorageProvider;
use nebula_credential::utils::EncryptedData;
use std::collections::HashMap;
use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::hashicorp_vault::HashicorpVault;

/// Helper to create test encrypted data
fn test_encrypted_data(seed: u8) -> EncryptedData {
    EncryptedData {
        version: 1,
        nonce: [seed; 12],
        ciphertext: vec![seed; 32],
        tag: [seed; 16],
    }
}

/// Helper to create test metadata
fn test_metadata(tags: HashMap<String, String>) -> CredentialMetadata {
    CredentialMetadata {
        created_at: chrono::Utc::now(),
        last_accessed: None,
        last_modified: chrono::Utc::now(),
        rotation_policy: None,
        tags,
    }
}

/// Helper to create Vault provider with testcontainers
async fn create_vault_provider(port: u16, token: &str) -> HashiCorpVaultProvider {
    let config = VaultConfig {
        address: format!("http://127.0.0.1:{}", port),
        auth_method: VaultAuthMethod::Token {
            token: token.to_string(),
        },
        mount_path: "secret".into(),
        path_prefix: "nebula-test".into(), // No trailing slash
        namespace: None,
        timeout: Duration::from_secs(10),
        tls_verify: false, // Disable TLS verification for local testing
        ..Default::default()
    };

    HashiCorpVaultProvider::new(config)
        .await
        .expect("Failed to create Vault provider")
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_store_and_retrieve() {
    let container = HashicorpVault::default()
        .start()
        .await
        .expect("Failed to start Vault");
    let port = container
        .get_host_port_ipv4(8200)
        .await
        .expect("Failed to get port");
    let token = "myroot";

    let provider = create_vault_provider(port, token).await;
    let id = CredentialId::new("test_store_retrieve").unwrap();
    let context = CredentialContext::new("user_123");

    let data = test_encrypted_data(42);
    let mut tags = HashMap::new();
    tags.insert("environment".into(), "test".into());
    let metadata = test_metadata(tags.clone());

    // Store credential
    provider
        .store(&id, data.clone(), metadata.clone(), &context)
        .await
        .expect("Failed to store credential");

    // Retrieve credential
    let (retrieved_data, retrieved_metadata) = provider
        .retrieve(&id, &context)
        .await
        .expect("Failed to retrieve credential");

    // Verify data
    assert_eq!(retrieved_data.version, data.version);
    assert_eq!(retrieved_data.nonce, data.nonce);
    assert_eq!(retrieved_data.ciphertext, data.ciphertext);
    assert_eq!(retrieved_data.tag, data.tag);

    // Verify metadata tags
    assert_eq!(
        retrieved_metadata.tags.get("environment"),
        Some(&"test".into())
    );

    // Cleanup
    provider
        .delete(&id, &context)
        .await
        .expect("Failed to delete credential");
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_update_existing() {
    let container = HashicorpVault::default()
        .start()
        .await
        .expect("Failed to start Vault");
    let port = container
        .get_host_port_ipv4(8200)
        .await
        .expect("Failed to get port");
    let token = "myroot";

    let provider = create_vault_provider(port, token).await;
    let id = CredentialId::new("test_update").unwrap();
    let context = CredentialContext::new("user_123");

    // Cleanup any existing secret from previous runs
    let _ = provider.delete(&id, &context).await;

    // Store initial credential
    let data1 = test_encrypted_data(10);
    let metadata1 = test_metadata(HashMap::new());

    provider
        .store(&id, data1, metadata1, &context)
        .await
        .expect("Failed to store initial credential");

    // Update with new data
    let data2 = test_encrypted_data(20);
    let metadata2 = test_metadata(HashMap::new());

    provider
        .store(&id, data2.clone(), metadata2, &context)
        .await
        .expect("Failed to update credential");

    // Verify updated data
    let (retrieved_data, _) = provider
        .retrieve(&id, &context)
        .await
        .expect("Failed to retrieve updated credential");

    assert_eq!(retrieved_data.nonce[0], 20);
    assert_eq!(retrieved_data.ciphertext[0], 20);

    // Cleanup
    provider
        .delete(&id, &context)
        .await
        .expect("Failed to delete credential");
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_delete() {
    let container = HashicorpVault::default()
        .start()
        .await
        .expect("Failed to start Vault");
    let port = container
        .get_host_port_ipv4(8200)
        .await
        .expect("Failed to get port");
    let token = "myroot";

    let provider = create_vault_provider(port, token).await;
    let id = CredentialId::new("test_delete").unwrap();
    let context = CredentialContext::new("user_123");

    // Store credential
    let data = test_encrypted_data(30);
    let metadata = test_metadata(HashMap::new());

    provider
        .store(&id, data, metadata, &context)
        .await
        .expect("Failed to store credential");

    // Verify exists
    let exists = provider
        .exists(&id, &context)
        .await
        .expect("Failed to check existence");
    assert!(exists);

    // Delete credential
    provider
        .delete(&id, &context)
        .await
        .expect("Failed to delete credential");

    // Verify doesn't exist
    let exists_after = provider
        .exists(&id, &context)
        .await
        .expect("Failed to check existence after delete");
    assert!(!exists_after);

    // Delete again (idempotent)
    provider
        .delete(&id, &context)
        .await
        .expect("Failed to delete credential again (idempotent)");
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_exists() {
    let container = HashicorpVault::default()
        .start()
        .await
        .expect("Failed to start Vault");
    let port = container
        .get_host_port_ipv4(8200)
        .await
        .expect("Failed to get port");
    let token = "myroot";

    let provider = create_vault_provider(port, token).await;
    let id = CredentialId::new("test_exists").unwrap();
    let context = CredentialContext::new("user_123");

    // Check non-existent
    let exists = provider
        .exists(&id, &context)
        .await
        .expect("Failed to check existence");
    assert!(!exists);

    // Store credential
    let data = test_encrypted_data(40);
    let metadata = test_metadata(HashMap::new());

    provider
        .store(&id, data, metadata, &context)
        .await
        .expect("Failed to store credential");

    // Check exists
    let exists_after = provider
        .exists(&id, &context)
        .await
        .expect("Failed to check existence after store");
    assert!(exists_after);

    // Cleanup
    provider
        .delete(&id, &context)
        .await
        .expect("Failed to delete credential");
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_list() {
    let container = HashicorpVault::default()
        .start()
        .await
        .expect("Failed to start Vault");
    let port = container
        .get_host_port_ipv4(8200)
        .await
        .expect("Failed to get port");
    let token = "myroot";

    let provider = create_vault_provider(port, token).await;
    let context = CredentialContext::new("user_123");

    // Create multiple credentials
    let ids = vec![
        CredentialId::new("test_list_1").unwrap(),
        CredentialId::new("test_list_2").unwrap(),
        CredentialId::new("test_list_3").unwrap(),
    ];

    for id in &ids {
        let data = test_encrypted_data(50);
        let metadata = test_metadata(HashMap::new());
        provider
            .store(id, data, metadata, &context)
            .await
            .expect("Failed to store credential");
    }

    // List credentials
    let listed_ids = provider
        .list(None, &context)
        .await
        .expect("Failed to list credentials");

    // Verify all IDs are present
    for id in &ids {
        assert!(
            listed_ids.contains(id),
            "Expected {} to be in list",
            id.as_str()
        );
    }

    // Cleanup
    for id in &ids {
        provider
            .delete(id, &context)
            .await
            .expect("Failed to delete credential");
    }
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_retrieve_nonexistent() {
    let container = HashicorpVault::default()
        .start()
        .await
        .expect("Failed to start Vault");
    let port = container
        .get_host_port_ipv4(8200)
        .await
        .expect("Failed to get port");
    let token = "myroot";

    let provider = create_vault_provider(port, token).await;
    let id = CredentialId::new("test_nonexistent").unwrap();
    let context = CredentialContext::new("user_123");

    // Try to retrieve non-existent credential
    let result = provider.retrieve(&id, &context).await;

    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        nebula_credential::core::StorageError::NotFound { .. }
    ));
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_metadata_tags() {
    let container = HashicorpVault::default()
        .start()
        .await
        .expect("Failed to start Vault");
    let port = container
        .get_host_port_ipv4(8200)
        .await
        .expect("Failed to get port");
    let token = "myroot";

    let provider = create_vault_provider(port, token).await;
    let id = CredentialId::new("test_tags").unwrap();
    let context = CredentialContext::new("user_123");

    let data = test_encrypted_data(60);
    let mut tags = HashMap::new();
    tags.insert("project".into(), "nebula".into());
    tags.insert("environment".into(), "staging".into());
    tags.insert("owner".into(), "team_api".into());
    let metadata = test_metadata(tags.clone());

    // Store credential
    provider
        .store(&id, data, metadata, &context)
        .await
        .expect("Failed to store credential");

    // Retrieve and verify tags
    let (_, retrieved_metadata) = provider
        .retrieve(&id, &context)
        .await
        .expect("Failed to retrieve credential");

    assert_eq!(
        retrieved_metadata.tags.get("project"),
        Some(&"nebula".into())
    );
    assert_eq!(
        retrieved_metadata.tags.get("environment"),
        Some(&"staging".into())
    );
    assert_eq!(
        retrieved_metadata.tags.get("owner"),
        Some(&"team_api".into())
    );

    // Cleanup
    provider
        .delete(&id, &context)
        .await
        .expect("Failed to delete credential");
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_vault_versioning() {
    let container = HashicorpVault::default()
        .start()
        .await
        .expect("Failed to start Vault");
    let port = container
        .get_host_port_ipv4(8200)
        .await
        .expect("Failed to get port");
    let token = "myroot";

    let provider = create_vault_provider(port, token).await;
    let id = CredentialId::new("test_versioning").unwrap();
    let context = CredentialContext::new("user_123");

    // Store version 1
    let data1 = test_encrypted_data(70);
    let metadata1 = test_metadata(HashMap::new());
    provider
        .store(&id, data1, metadata1, &context)
        .await
        .expect("Failed to store version 1");

    // Store version 2
    let data2 = test_encrypted_data(80);
    let metadata2 = test_metadata(HashMap::new());
    provider
        .store(&id, data2.clone(), metadata2, &context)
        .await
        .expect("Failed to store version 2");

    // Retrieve should get latest version (v2)
    let (retrieved_data, _) = provider
        .retrieve(&id, &context)
        .await
        .expect("Failed to retrieve credential");

    assert_eq!(retrieved_data.nonce[0], 80);
    assert_eq!(retrieved_data.ciphertext[0], 80);

    // Cleanup
    provider
        .delete(&id, &context)
        .await
        .expect("Failed to delete credential");
}
