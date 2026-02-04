//! Integration tests for Kubernetes Secrets provider using testcontainers
//!
//! These tests use testcontainers to spin up a K3s cluster automatically.
//!
//! **Prerequisites**: Docker must be running
//!
//! Run with: `cargo test --test integration_kubernetes --features storage-k8s -- --ignored`

use chrono::Utc;
use nebula_credential::core::{
    CredentialContext, CredentialFilter, CredentialId, CredentialMetadata,
};
use nebula_credential::providers::{KubernetesSecretsConfig, KubernetesSecretsProvider};
use nebula_credential::traits::StorageProvider;
use nebula_credential::utils::EncryptedData;
use std::collections::HashMap;
use std::time::Duration;
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::k3s::K3s;

/// Create a Kubernetes provider connected to the test cluster
async fn create_k8s_provider(
    kubeconfig_content: String,
    host: String,
    port: u16,
) -> KubernetesSecretsProvider {
    // read_kube_config() returns the YAML content with server: https://127.0.0.1:6443
    // We need to rewrite it to use the actual Docker host and port
    let updated_config = kubeconfig_content
        .replace(
            "https://127.0.0.1:6443",
            &format!("https://{}:{}", host, port),
        )
        .replace(
            "https://0.0.0.0:6443",
            &format!("https://{}:{}", host, port),
        );

    let temp_dir = std::env::temp_dir();
    let kubeconfig_path = temp_dir.join(format!("kubeconfig-test-{}.yaml", uuid::Uuid::new_v4()));

    tokio::fs::write(&kubeconfig_path, updated_config)
        .await
        .expect("Failed to write kubeconfig");

    let config = KubernetesSecretsConfig {
        namespace: "default".into(),
        kubeconfig_path: Some(kubeconfig_path),
        secret_prefix: "nebula-test-".into(),
        timeout: Duration::from_secs(30),
        accept_invalid_certs: true, // K3s uses self-signed certs
        ..Default::default()
    };

    // Wait a bit for K3s to be fully ready
    tokio::time::sleep(Duration::from_secs(5)).await;

    KubernetesSecretsProvider::new(config)
        .await
        .expect("Failed to create provider")
}

fn create_test_data() -> (EncryptedData, CredentialMetadata) {
    let data = EncryptedData {
        version: 1,
        nonce: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        ciphertext: b"encrypted_secret_data".to_vec(),
        tag: [0; 16],
    };

    let mut tags = HashMap::new();
    tags.insert("env".to_string(), "test".to_string());
    tags.insert("service".to_string(), "api".to_string());

    let metadata = CredentialMetadata {
        created_at: Utc::now(),
        last_accessed: None,
        last_modified: Utc::now(),
        scope: None,
        rotation_policy: None,
        tags,
    };

    (data, metadata)
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_k8s_store_and_retrieve() {
    let temp_dir = std::env::temp_dir();

    let k3s_instance = K3s::default()
        .with_conf_mount(&temp_dir)
        .with_privileged(true)
        .with_userns_mode("host")
        .start()
        .await
        .expect("Failed to start K3s");

    let host = k3s_instance.get_host().await.expect("Failed to get host");
    let port = k3s_instance
        .get_host_port_ipv4(6443)
        .await
        .expect("Failed to get port");

    let kubeconfig = k3s_instance
        .image()
        .read_kube_config()
        .expect("Failed to read kubeconfig");

    let provider = create_k8s_provider(kubeconfig, host.to_string(), port).await;
    let id = CredentialId::new("test_credential").unwrap();
    let context = CredentialContext::new("test_user");
    let (data, metadata) = create_test_data();

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

    // Verify metadata
    assert_eq!(retrieved_metadata.tags, metadata.tags);

    // Cleanup
    provider.delete(&id, &context).await.ok();
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_k8s_delete() {
    let temp_dir = std::env::temp_dir();

    let k3s_instance = K3s::default()
        .with_conf_mount(&temp_dir)
        .with_privileged(true)
        .with_userns_mode("host")
        .start()
        .await
        .expect("Failed to start K3s");

    let host = k3s_instance.get_host().await.expect("Failed to get host");
    let port = k3s_instance
        .get_host_port_ipv4(6443)
        .await
        .expect("Failed to get port");

    let kubeconfig = k3s_instance
        .image()
        .read_kube_config()
        .expect("Failed to read kubeconfig");

    let provider = create_k8s_provider(kubeconfig, host.to_string(), port).await;
    let id = CredentialId::new("test_delete").unwrap();
    let context = CredentialContext::new("test_user");
    let (data, metadata) = create_test_data();

    // Store credential
    provider
        .store(&id, data, metadata, &context)
        .await
        .expect("Failed to store credential");

    // Verify it exists
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

    // Verify it's gone
    let exists_after = provider
        .exists(&id, &context)
        .await
        .expect("Failed to check existence");
    assert!(!exists_after);

    // Idempotent delete should succeed
    provider
        .delete(&id, &context)
        .await
        .expect("Failed to delete non-existent credential");
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_k8s_exists() {
    let temp_dir = std::env::temp_dir();

    let k3s_instance = K3s::default()
        .with_conf_mount(&temp_dir)
        .with_privileged(true)
        .with_userns_mode("host")
        .start()
        .await
        .expect("Failed to start K3s");

    let host = k3s_instance.get_host().await.expect("Failed to get host");
    let port = k3s_instance
        .get_host_port_ipv4(6443)
        .await
        .expect("Failed to get port");

    let kubeconfig = k3s_instance
        .image()
        .read_kube_config()
        .expect("Failed to read kubeconfig");

    let provider = create_k8s_provider(kubeconfig, host.to_string(), port).await;
    let id = CredentialId::new("test_exists").unwrap();
    let context = CredentialContext::new("test_user");

    // Check non-existent credential
    let exists = provider
        .exists(&id, &context)
        .await
        .expect("Failed to check existence");
    assert!(!exists);

    // Store credential
    let (data, metadata) = create_test_data();
    provider
        .store(&id, data, metadata, &context)
        .await
        .expect("Failed to store credential");

    // Check existing credential
    let exists_after = provider
        .exists(&id, &context)
        .await
        .expect("Failed to check existence");
    assert!(exists_after);

    // Cleanup
    provider.delete(&id, &context).await.ok();
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_k8s_list() {
    let temp_dir = std::env::temp_dir();

    let k3s_instance = K3s::default()
        .with_conf_mount(&temp_dir)
        .with_privileged(true)
        .with_userns_mode("host")
        .start()
        .await
        .expect("Failed to start K3s");

    let host = k3s_instance.get_host().await.expect("Failed to get host");
    let port = k3s_instance
        .get_host_port_ipv4(6443)
        .await
        .expect("Failed to get port");

    let kubeconfig = k3s_instance
        .image()
        .read_kube_config()
        .expect("Failed to read kubeconfig");

    let provider = create_k8s_provider(kubeconfig, host.to_string(), port).await;
    let context = CredentialContext::new("test_user");

    // Store multiple credentials with different tags
    let test_data = vec![
        ("cred1", "prod", "api"),
        ("cred2", "prod", "db"),
        ("cred3", "dev", "api"),
    ];

    for (name, env, service) in &test_data {
        let id = CredentialId::new(*name).unwrap();
        let (data, mut metadata) = create_test_data();
        metadata.tags.insert("env".to_string(), env.to_string());
        metadata
            .tags
            .insert("service".to_string(), service.to_string());

        provider
            .store(&id, data, metadata, &context)
            .await
            .expect("Failed to store credential");
    }

    // List all credentials
    let all_ids = provider
        .list(None, &context)
        .await
        .expect("Failed to list credentials");
    assert_eq!(all_ids.len(), 3);

    // List with tag filter
    let mut filter_tags = HashMap::new();
    filter_tags.insert("env".to_string(), "prod".to_string());
    let filter = CredentialFilter {
        tags: Some(filter_tags),
        created_after: None,
        created_before: None,
    };

    let filtered_ids = provider
        .list(Some(&filter), &context)
        .await
        .expect("Failed to list filtered credentials");
    assert_eq!(filtered_ids.len(), 2);

    // Cleanup
    for (name, _, _) in &test_data {
        let id = CredentialId::new(*name).unwrap();
        provider.delete(&id, &context).await.ok();
    }
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_k8s_retrieve_nonexistent() {
    let temp_dir = std::env::temp_dir();

    let k3s_instance = K3s::default()
        .with_conf_mount(&temp_dir)
        .with_privileged(true)
        .with_userns_mode("host")
        .start()
        .await
        .expect("Failed to start K3s");

    let host = k3s_instance.get_host().await.expect("Failed to get host");
    let port = k3s_instance
        .get_host_port_ipv4(6443)
        .await
        .expect("Failed to get port");

    let kubeconfig = k3s_instance
        .image()
        .read_kube_config()
        .expect("Failed to read kubeconfig");

    let provider = create_k8s_provider(kubeconfig, host.to_string(), port).await;
    let id = CredentialId::new("nonexistent").unwrap();
    let context = CredentialContext::new("test_user");

    // Try to retrieve non-existent credential
    let result = provider.retrieve(&id, &context).await;

    assert!(result.is_err());
    match result {
        Err(nebula_credential::core::StorageError::NotFound { .. }) => {
            // Expected error
        }
        _ => panic!("Expected NotFound error"),
    }
}

#[tokio::test]
#[ignore] // Requires Docker
async fn test_k8s_update_credential() {
    let temp_dir = std::env::temp_dir();

    let k3s_instance = K3s::default()
        .with_conf_mount(&temp_dir)
        .with_privileged(true)
        .with_userns_mode("host")
        .start()
        .await
        .expect("Failed to start K3s");

    let host = k3s_instance.get_host().await.expect("Failed to get host");
    let port = k3s_instance
        .get_host_port_ipv4(6443)
        .await
        .expect("Failed to get port");

    let kubeconfig = k3s_instance
        .image()
        .read_kube_config()
        .expect("Failed to read kubeconfig");

    let provider = create_k8s_provider(kubeconfig, host.to_string(), port).await;
    let id = CredentialId::new("test_update").unwrap();
    let context = CredentialContext::new("test_user");

    // Store initial credential
    let (data1, metadata1) = create_test_data();
    provider
        .store(&id, data1, metadata1, &context)
        .await
        .expect("Failed to store credential");

    // Update with new data
    let data2 = EncryptedData {
        version: 1,
        nonce: [10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21],
        ciphertext: b"updated_encrypted_data".to_vec(),
        tag: [1; 16],
    };

    let mut tags2 = HashMap::new();
    tags2.insert("env".to_string(), "production".to_string());
    tags2.insert("updated".to_string(), "true".to_string());

    let metadata2 = CredentialMetadata {
        created_at: Utc::now(),
        last_accessed: None,
        last_modified: Utc::now(),
        scope: None,
        rotation_policy: None,
        tags: tags2.clone(),
    };

    provider
        .store(&id, data2.clone(), metadata2, &context)
        .await
        .expect("Failed to update credential");

    // Retrieve and verify updated data
    let (retrieved_data, retrieved_metadata) = provider
        .retrieve(&id, &context)
        .await
        .expect("Failed to retrieve updated credential");

    assert_eq!(retrieved_data.ciphertext, data2.ciphertext);
    assert_eq!(retrieved_metadata.tags, tags2);

    // Cleanup
    provider.delete(&id, &context).await.ok();
}
