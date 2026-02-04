# Testing Strategy: Storage Backends

**Feature**: Production-Ready Storage Backends (002-storage-backends)  
**Date**: 2026-02-04  
**Purpose**: Comprehensive testing strategy for all 5 storage providers

---

## Table of Contents

1. [Testing Pyramid](#testing-pyramid)
2. [Unit Tests (Fast, No Docker)](#unit-tests-fast-no-docker)
3. [Integration Tests (Docker)](#integration-tests-docker)
4. [Local Tests (Filesystem)](#local-tests-filesystem)
5. [Docker Setup](#docker-setup)
6. [Test Execution](#test-execution)
7. [CI/CD Integration](#cicd-integration)

---

## Testing Pyramid

```
         ┌─────────────────┐
         │  E2E Tests      │  ← Manual testing with real cloud providers
         │  (Manual)       │     (AWS, Azure - expensive, slow)
         └─────────────────┘
              ▲
              │
         ┌────────────────────────┐
         │  Integration Tests     │  ← Docker containers (Vault, K8s, LocalStack)
         │  (Docker/testcontainers) │    ~10 tests per provider, ~5-30s each
         └────────────────────────┘
                   ▲
                   │
         ┌──────────────────────────────┐
         │  Unit Tests                  │  ← Mock providers, fast feedback
         │  (MockStorageProvider)       │    ~50 tests per provider, <1s each
         └──────────────────────────────┘
```

**Test Distribution**:
- **70% Unit Tests**: MockStorageProvider, logic validation, error handling
- **25% Integration Tests**: Docker containers, real backend behavior
- **5% E2E Tests**: Manual validation with real cloud accounts (expensive)

---

## Unit Tests (Fast, No Docker)

### MockStorageProvider Pattern

**Purpose**: Test business logic without external dependencies (filesystem, network, Docker).

**Location**: `crates/nebula-credential/src/providers/mock.rs`

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use async_trait::async_trait;

/// Mock storage provider for unit testing
///
/// Stores credentials in memory, simulates errors on demand.
#[derive(Clone)]
pub struct MockStorageProvider {
    storage: Arc<RwLock<HashMap<CredentialId, (EncryptedData, CredentialMetadata)>>>,
    should_fail: Arc<RwLock<Option<StorageError>>>,
}

impl MockStorageProvider {
    pub fn new() -> Self {
        Self {
            storage: Arc::new(RwLock::new(HashMap::new())),
            should_fail: Arc::new(RwLock::new(None)),
        }
    }
    
    /// Configure mock to fail next operation with specific error
    pub async fn fail_next_with(&self, error: StorageError) {
        *self.should_fail.write().await = Some(error);
    }
    
    /// Clear all stored credentials
    pub async fn clear(&self) {
        self.storage.write().await.clear();
    }
    
    /// Get count of stored credentials
    pub async fn count(&self) -> usize {
        self.storage.read().await.len()
    }
}

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
        if let Some(error) = self.should_fail.write().await.take() {
            return Err(error);
        }
        
        let mut storage = self.storage.write().await;
        storage.insert(id.clone(), (data, metadata));
        Ok(())
    }
    
    async fn retrieve(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(EncryptedData, CredentialMetadata), StorageError> {
        if let Some(error) = self.should_fail.write().await.take() {
            return Err(error);
        }
        
        let storage = self.storage.read().await;
        storage.get(id)
            .cloned()
            .ok_or(StorageError::NotFound {
                resource_id: id.to_string(),
            })
    }
    
    async fn delete(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<(), StorageError> {
        if let Some(error) = self.should_fail.write().await.take() {
            return Err(error);
        }
        
        let mut storage = self.storage.write().await;
        storage.remove(id);
        Ok(()) // Idempotent - succeeds even if not found
    }
    
    async fn list(
        &self,
        filter: Option<&CredentialFilter>,
        _context: &CredentialContext,
    ) -> Result<Vec<CredentialId>, StorageError> {
        if let Some(error) = self.should_fail.write().await.take() {
            return Err(error);
        }
        
        let storage = self.storage.read().await;
        let mut ids: Vec<CredentialId> = storage.keys().cloned().collect();
        
        // Apply filter if provided
        if let Some(filter) = filter {
            if let Some(tags) = &filter.tags {
                ids.retain(|id| {
                    if let Some((_, metadata)) = storage.get(id) {
                        tags.iter().all(|tag| metadata.tags.contains(tag))
                    } else {
                        false
                    }
                });
            }
        }
        
        Ok(ids)
    }
    
    async fn exists(
        &self,
        id: &CredentialId,
        _context: &CredentialContext,
    ) -> Result<bool, StorageError> {
        if let Some(error) = self.should_fail.write().await.take() {
            return Err(error);
        }
        
        let storage = self.storage.read().await;
        Ok(storage.contains_key(id))
    }
}
```

### Example Unit Tests

**Location**: `crates/nebula-credential/tests/providers/mock_tests.rs`

```rust
use nebula_credential::prelude::*;

#[tokio::test]
async fn test_store_and_retrieve() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("test_cred").unwrap();
    let key = EncryptionKey::generate().unwrap();
    let data = encrypt(&key, b"secret_value").unwrap();
    let metadata = CredentialMetadata {
        created_at: Utc::now(),
        tags: vec!["env:test".into()],
        ..Default::default()
    };
    let context = CredentialContext::new("user_123");
    
    // Store
    provider.store(&id, data.clone(), metadata.clone(), &context).await.unwrap();
    
    // Retrieve
    let (retrieved_data, retrieved_metadata) = provider.retrieve(&id, &context).await.unwrap();
    
    assert_eq!(data.ciphertext, retrieved_data.ciphertext);
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
    let key = EncryptionKey::generate().unwrap();
    let data = encrypt(&key, b"secret").unwrap();
    let metadata = CredentialMetadata::default();
    provider.store(&id, data, metadata, &context).await.unwrap();
    
    // First delete
    provider.delete(&id, &context).await.unwrap();
    
    // Second delete should also succeed (idempotent)
    provider.delete(&id, &context).await.unwrap();
}

#[tokio::test]
async fn test_list_with_filter() {
    let provider = MockStorageProvider::new();
    let context = CredentialContext::new("user_123");
    let key = EncryptionKey::generate().unwrap();
    
    // Store credentials with different tags
    let cred1 = CredentialId::new("cred1").unwrap();
    let metadata1 = CredentialMetadata {
        tags: vec!["env:prod".into(), "type:api".into()],
        ..Default::default()
    };
    provider.store(&cred1, encrypt(&key, b"data1").unwrap(), metadata1, &context).await.unwrap();
    
    let cred2 = CredentialId::new("cred2").unwrap();
    let metadata2 = CredentialMetadata {
        tags: vec!["env:dev".into(), "type:api".into()],
        ..Default::default()
    };
    provider.store(&cred2, encrypt(&key, b"data2").unwrap(), metadata2, &context).await.unwrap();
    
    // List with filter for prod
    let filter = CredentialFilter {
        tags: Some(vec!["env:prod".into()]),
        ..Default::default()
    };
    let ids = provider.list(Some(&filter), &context).await.unwrap();
    
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0].as_str(), "cred1");
}

#[tokio::test]
async fn test_simulated_error() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("test_cred").unwrap();
    let context = CredentialContext::new("user_123");
    
    // Configure mock to fail next operation
    provider.fail_next_with(StorageError::PermissionDenied {
        resource: "test_resource".into(),
        required_permission: "read".into(),
    }).await;
    
    // Next operation should fail
    let result = provider.retrieve(&id, &context).await;
    assert!(matches!(result, Err(StorageError::PermissionDenied { .. })));
    
    // Subsequent operations should succeed (error was one-time)
    let key = EncryptionKey::generate().unwrap();
    let data = encrypt(&key, b"secret").unwrap();
    let metadata = CredentialMetadata::default();
    provider.store(&id, data, metadata, &context).await.unwrap();
}

#[tokio::test]
async fn test_concurrent_operations() {
    use tokio::task::JoinSet;
    
    let provider = MockStorageProvider::new();
    let context = CredentialContext::new("user_123");
    let key = EncryptionKey::generate().unwrap();
    
    let mut set = JoinSet::new();
    
    // Spawn 100 concurrent store operations
    for i in 0..100 {
        let provider = provider.clone();
        let context = context.clone();
        let key = key.clone();
        
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
```

---

## Integration Tests (Docker)

### Docker-Compose Setup

**Location**: `crates/nebula-credential/docker-compose.test.yml`

```yaml
version: '3.8'

services:
  # HashiCorp Vault (dev mode)
  vault:
    image: hashicorp/vault:latest
    environment:
      VAULT_DEV_ROOT_TOKEN_ID: test-root-token
      VAULT_DEV_LISTEN_ADDRESS: 0.0.0.0:8200
    ports:
      - "8200:8200"
    cap_add:
      - IPC_LOCK
    healthcheck:
      test: ["CMD", "vault", "status"]
      interval: 5s
      timeout: 3s
      retries: 5

  # LocalStack (AWS Secrets Manager emulator)
  localstack:
    image: localstack/localstack:latest
    environment:
      SERVICES: secretsmanager
      DEBUG: 1
      DEFAULT_REGION: us-east-1
    ports:
      - "4566:4566"
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:4566/_localstack/health"]
      interval: 5s
      timeout: 3s
      retries: 10

  # Kubernetes (kind - Kubernetes in Docker)
  # Note: Requires separate setup script, not in docker-compose
  # See scripts/setup-kind-for-tests.sh

  # Lowkey Vault (Azure Key Vault emulator)
  lowkey-vault:
    image: nagyesta/lowkey-vault:7.1.0
    ports:
      - "8443:8443"
    environment:
      LOWKEY_ENABLE_AUTH: "true"
      LOWKEY_VAULT_NAMES: "test-vault-1,test-vault-2"
    healthcheck:
      test: ["CMD", "curl", "-f", "-k", "https://localhost:8443/metadata/vaults"]
      interval: 5s
      timeout: 3s
      retries: 5

  # PostgreSQL (for future credential caching layer - Phase 3)
  postgres:
    image: postgres:15-alpine
    environment:
      POSTGRES_USER: test
      POSTGRES_PASSWORD: test
      POSTGRES_DB: nebula_test
    ports:
      - "5432:5432"
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U test"]
      interval: 5s
      timeout: 3s
      retries: 5
```

### Testcontainers Integration Tests

**Location**: `crates/nebula-credential/tests/integration/vault_integration.rs`

```rust
use testcontainers::{clients::Cli, images::generic::GenericImage};
use nebula_credential::prelude::*;

#[tokio::test]
async fn test_vault_provider_crud_operations() {
    // Start Vault container
    let docker = Cli::default();
    let vault_container = docker.run(
        GenericImage::new("hashicorp/vault", "latest")
            .with_env_var("VAULT_DEV_ROOT_TOKEN_ID", "test-token")
            .with_env_var("VAULT_DEV_LISTEN_ADDRESS", "0.0.0.0:8200")
            .with_wait_for(testcontainers::core::WaitFor::message_on_stdout("Vault server started!"))
    );
    
    let port = vault_container.get_host_port_ipv4(8200);
    let vault_url = format!("http://127.0.0.1:{}", port);
    
    // Configure provider
    let config = VaultConfig {
        address: vault_url,
        auth_method: VaultAuthMethod::Token {
            token: SecretString::new("test-token".into()),
        },
        mount_path: "secret".into(),
        path_prefix: "test/credentials".into(),
        tls_verify: false, // Dev mode uses HTTP
        ..Default::default()
    };
    
    let provider = HashiCorpVaultProvider::new(config).await.unwrap();
    
    // Test CRUD operations
    let id = CredentialId::new("integration_test_cred").unwrap();
    let key = EncryptionKey::generate().unwrap();
    let data = encrypt(&key, b"test_secret_value").unwrap();
    let metadata = CredentialMetadata {
        created_at: Utc::now(),
        tags: vec!["integration:test".into()],
        ..Default::default()
    };
    let context = CredentialContext::new("integration_test_user");
    
    // Store
    provider.store(&id, data.clone(), metadata.clone(), &context).await.unwrap();
    
    // Retrieve
    let (retrieved_data, retrieved_metadata) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(data.ciphertext, retrieved_data.ciphertext);
    assert_eq!(metadata.tags, retrieved_metadata.tags);
    
    // Exists
    let exists = provider.exists(&id, &context).await.unwrap();
    assert!(exists);
    
    // List
    let ids = provider.list(None, &context).await.unwrap();
    assert!(ids.contains(&id));
    
    // Delete
    provider.delete(&id, &context).await.unwrap();
    
    // Verify deleted
    let exists = provider.exists(&id, &context).await.unwrap();
    assert!(!exists);
}

#[tokio::test]
async fn test_vault_versioning() {
    let docker = Cli::default();
    let vault_container = docker.run(
        GenericImage::new("hashicorp/vault", "latest")
            .with_env_var("VAULT_DEV_ROOT_TOKEN_ID", "test-token")
    );
    
    let port = vault_container.get_host_port_ipv4(8200);
    let config = VaultConfig {
        address: format!("http://127.0.0.1:{}", port),
        auth_method: VaultAuthMethod::Token {
            token: SecretString::new("test-token".into()),
        },
        ..Default::default()
    };
    
    let provider = HashiCorpVaultProvider::new(config).await.unwrap();
    let id = CredentialId::new("versioned_cred").unwrap();
    let context = CredentialContext::new("test_user");
    let key = EncryptionKey::generate().unwrap();
    
    // Store version 1
    let data_v1 = encrypt(&key, b"secret_v1").unwrap();
    let metadata_v1 = CredentialMetadata {
        tags: vec!["version:1".into()],
        ..Default::default()
    };
    provider.store(&id, data_v1, metadata_v1, &context).await.unwrap();
    
    // Store version 2 (overwrites, creates new version)
    let data_v2 = encrypt(&key, b"secret_v2").unwrap();
    let metadata_v2 = CredentialMetadata {
        tags: vec!["version:2".into()],
        ..Default::default()
    };
    provider.store(&id, data_v2.clone(), metadata_v2.clone(), &context).await.unwrap();
    
    // Retrieve should return latest version (v2)
    let (retrieved_data, retrieved_metadata) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(data_v2.ciphertext, retrieved_data.ciphertext);
    assert_eq!(metadata_v2.tags, retrieved_metadata.tags);
}

#[tokio::test]
async fn test_vault_token_renewal() {
    // This test requires Vault Enterprise or complex setup
    // Marked as #[ignore] for normal test runs
    // Run with: cargo test --test vault_integration -- --ignored
}
```

**Location**: `crates/nebula-credential/tests/integration/localstack_integration.rs`

```rust
use testcontainers::{clients::Cli, images::generic::GenericImage};
use nebula_credential::prelude::*;

#[tokio::test]
async fn test_aws_provider_with_localstack() {
    let docker = Cli::default();
    let localstack = docker.run(
        GenericImage::new("localstack/localstack", "latest")
            .with_env_var("SERVICES", "secretsmanager")
            .with_env_var("DEFAULT_REGION", "us-east-1")
            .with_wait_for(testcontainers::core::WaitFor::message_on_stdout("Ready."))
    );
    
    let port = localstack.get_host_port_ipv4(4566);
    let endpoint = format!("http://127.0.0.1:{}", port);
    
    // Configure AWS SDK to use LocalStack
    std::env::set_var("AWS_ACCESS_KEY_ID", "test");
    std::env::set_var("AWS_SECRET_ACCESS_KEY", "test");
    std::env::set_var("AWS_REGION", "us-east-1");
    std::env::set_var("AWS_ENDPOINT_URL", &endpoint);
    
    let config = AwsSecretsManagerConfig {
        region: Some("us-east-1".into()),
        secret_prefix: "nebula/test/".into(),
        ..Default::default()
    };
    
    let provider = AwsSecretsManagerProvider::new(config).await.unwrap();
    
    // Test operations
    let id = CredentialId::new("aws_test_cred").unwrap();
    let key = EncryptionKey::generate().unwrap();
    let data = encrypt(&key, b"aws_secret").unwrap();
    let metadata = CredentialMetadata {
        tags: vec!["provider:aws".into()],
        ..Default::default()
    };
    let context = CredentialContext::new("aws_test_user");
    
    // Store
    provider.store(&id, data.clone(), metadata.clone(), &context).await.unwrap();
    
    // Retrieve
    let (retrieved_data, _) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(data.ciphertext, retrieved_data.ciphertext);
    
    // Delete
    provider.delete(&id, &context).await.unwrap();
}

#[tokio::test]
async fn test_aws_size_limit_validation() {
    let docker = Cli::default();
    let localstack = docker.run(
        GenericImage::new("localstack/localstack", "latest")
            .with_env_var("SERVICES", "secretsmanager")
    );
    
    let port = localstack.get_host_port_ipv4(4566);
    std::env::set_var("AWS_ENDPOINT_URL", format!("http://127.0.0.1:{}", port));
    
    let config = AwsSecretsManagerConfig::default();
    let provider = AwsSecretsManagerProvider::new(config).await.unwrap();
    
    let id = CredentialId::new("large_cred").unwrap();
    let context = CredentialContext::new("test_user");
    
    // Create credential larger than 64KB (AWS limit)
    let large_data = vec![0u8; 70 * 1024]; // 70KB
    let key = EncryptionKey::generate().unwrap();
    let encrypted = encrypt(&key, &large_data).unwrap();
    let metadata = CredentialMetadata::default();
    
    // Should fail with CredentialTooLarge error
    let result = provider.store(&id, encrypted, metadata, &context).await;
    
    assert!(matches!(result, Err(StorageError::CredentialTooLarge { .. })));
}
```

**Location**: `crates/nebula-credential/tests/integration/azure_lowkey_vault_integration.rs`

```rust
use testcontainers::{clients::Cli, images::generic::GenericImage};
use nebula_credential::prelude::*;

#[tokio::test]
async fn test_azure_provider_with_lowkey_vault() {
    // Start Lowkey Vault container (Azure Key Vault emulator)
    let docker = Cli::default();
    let lowkey_vault = docker.run(
        GenericImage::new("nagyesta/lowkey-vault", "7.1.0")
            .with_env_var("LOWKEY_ENABLE_AUTH", "false") // Disable auth for simpler testing
            .with_env_var("LOWKEY_VAULT_NAMES", "test-vault")
            .with_wait_for(testcontainers::core::WaitFor::message_on_stdout("Started"))
    );
    
    let port = lowkey_vault.get_host_port_ipv4(8443);
    let vault_url = format!("https://localhost:{}", port);
    
    // Configure Azure provider to use Lowkey Vault
    let config = AzureKeyVaultConfig {
        vault_url,
        credential_type: AzureCredentialType::DeveloperTools, // Mock credential
        secret_prefix: "test-".into(),
        ..Default::default()
    };
    
    let provider = AzureKeyVaultProvider::new(config).await.unwrap();
    
    // Test CRUD operations
    let id = CredentialId::new("azure_test_cred").unwrap();
    let key = EncryptionKey::generate().unwrap();
    let data = encrypt(&key, b"azure_secret_value").unwrap();
    let metadata = CredentialMetadata {
        created_at: Utc::now(),
        tags: vec!["provider:azure".into(), "emulator:lowkey-vault".into()],
        ..Default::default()
    };
    let context = CredentialContext::new("azure_test_user");
    
    // Store
    provider.store(&id, data.clone(), metadata.clone(), &context).await.unwrap();
    
    // Retrieve
    let (retrieved_data, retrieved_metadata) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(data.ciphertext, retrieved_data.ciphertext);
    assert_eq!(metadata.tags, retrieved_metadata.tags);
    
    // Exists
    let exists = provider.exists(&id, &context).await.unwrap();
    assert!(exists);
    
    // List
    let ids = provider.list(None, &context).await.unwrap();
    assert!(ids.contains(&id));
    
    // Delete
    provider.delete(&id, &context).await.unwrap();
    
    // Verify deleted
    let exists = provider.exists(&id, &context).await.unwrap();
    assert!(!exists);
}

#[tokio::test]
async fn test_azure_soft_delete_with_lowkey_vault() {
    let docker = Cli::default();
    let lowkey_vault = docker.run(
        GenericImage::new("nagyesta/lowkey-vault", "7.1.0")
            .with_env_var("LOWKEY_ENABLE_AUTH", "false")
    );
    
    let port = lowkey_vault.get_host_port_ipv4(8443);
    let config = AzureKeyVaultConfig {
        vault_url: format!("https://localhost:{}", port),
        credential_type: AzureCredentialType::DeveloperTools,
        ..Default::default()
    };
    
    let provider = AzureKeyVaultProvider::new(config).await.unwrap();
    let id = CredentialId::new("soft_delete_test").unwrap();
    let context = CredentialContext::new("test_user");
    let key = EncryptionKey::generate().unwrap();
    
    // Store credential
    let data = encrypt(&key, b"soft_delete_value").unwrap();
    let metadata = CredentialMetadata::default();
    provider.store(&id, data, metadata, &context).await.unwrap();
    
    // Soft-delete
    provider.delete(&id, &context).await.unwrap();
    
    // Should not be retrievable (soft-deleted)
    let result = provider.retrieve(&id, &context).await;
    assert!(matches!(result, Err(StorageError::NotFound { .. })));
    
    // TODO: Test recovery if Lowkey Vault supports it
    // provider.recover(&id, &context).await.unwrap();
}

#[tokio::test]
async fn test_azure_metadata_tags_with_lowkey_vault() {
    let docker = Cli::default();
    let lowkey_vault = docker.run(
        GenericImage::new("nagyesta/lowkey-vault", "7.1.0")
            .with_env_var("LOWKEY_ENABLE_AUTH", "false")
    );
    
    let port = lowkey_vault.get_host_port_ipv4(8443);
    let config = AzureKeyVaultConfig {
        vault_url: format!("https://localhost:{}", port),
        ..Default::default()
    };
    
    let provider = AzureKeyVaultProvider::new(config).await.unwrap();
    let context = CredentialContext::new("test_user");
    let key = EncryptionKey::generate().unwrap();
    
    // Store credential with multiple tags
    let id = CredentialId::new("tagged_cred").unwrap();
    let metadata = CredentialMetadata {
        tags: vec![
            "environment:production".into(),
            "application:web-api".into(),
            "owner:platform-team".into(),
        ],
        ..Default::default()
    };
    let data = encrypt(&key, b"tagged_secret").unwrap();
    provider.store(&id, data, metadata.clone(), &context).await.unwrap();
    
    // Retrieve and verify tags preserved
    let (_, retrieved_metadata) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(retrieved_metadata.tags.len(), 3);
    assert!(retrieved_metadata.tags.contains(&"environment:production".into()));
    assert!(retrieved_metadata.tags.contains(&"application:web-api".into()));
    assert!(retrieved_metadata.tags.contains(&"owner:platform-team".into()));
}

#[tokio::test]
async fn test_azure_concurrent_operations_with_lowkey_vault() {
    use tokio::task::JoinSet;
    
    let docker = Cli::default();
    let lowkey_vault = docker.run(
        GenericImage::new("nagyesta/lowkey-vault", "7.1.0")
            .with_env_var("LOWKEY_ENABLE_AUTH", "false")
    );
    
    let port = lowkey_vault.get_host_port_ipv4(8443);
    let config = AzureKeyVaultConfig {
        vault_url: format!("https://localhost:{}", port),
        ..Default::default()
    };
    
    let provider = Arc::new(AzureKeyVaultProvider::new(config).await.unwrap());
    let context = CredentialContext::new("test_user");
    let key = Arc::new(EncryptionKey::generate().unwrap());
    
    let mut set = JoinSet::new();
    
    // Spawn 50 concurrent store operations
    for i in 0..50 {
        let provider = Arc::clone(&provider);
        let context = context.clone();
        let key = Arc::clone(&key);
        
        set.spawn(async move {
            let id = CredentialId::new(&format!("concurrent_cred_{}", i)).unwrap();
            let data = encrypt(&key, format!("secret_{}", i).as_bytes()).unwrap();
            let metadata = CredentialMetadata {
                tags: vec![format!("iteration:{}", i)],
                ..Default::default()
            };
            provider.store(&id, data, metadata, &context).await
        });
    }
    
    // All operations should succeed
    let mut success_count = 0;
    while let Some(result) = set.join_next().await {
        result.unwrap().unwrap();
        success_count += 1;
    }
    
    assert_eq!(success_count, 50);
}
```

### Kubernetes Integration Tests

**Location**: `scripts/setup-kind-for-tests.sh`

```bash
#!/bin/bash
# Setup Kubernetes cluster for integration testing using kind

set -e

CLUSTER_NAME="nebula-test"

echo "Creating kind cluster: $CLUSTER_NAME"
kind create cluster --name $CLUSTER_NAME --config - <<EOF
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
- role: control-plane
EOF

echo "Waiting for cluster to be ready..."
kubectl wait --for=condition=Ready nodes --all --timeout=60s

echo "Creating test namespace"
kubectl create namespace nebula-test

echo "Creating ServiceAccount with Secrets permissions"
kubectl apply -f - <<EOF
apiVersion: v1
kind: ServiceAccount
metadata:
  name: nebula-test-sa
  namespace: nebula-test
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: nebula-test-role
  namespace: nebula-test
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: nebula-test-rolebinding
  namespace: nebula-test
subjects:
- kind: ServiceAccount
  name: nebula-test-sa
  namespace: nebula-test
roleRef:
  kind: Role
  name: nebula-test-role
  apiGroup: rbac.authorization.k8s.io
EOF

echo "kind cluster ready for testing!"
echo "Run tests with: cargo test --test kubernetes_integration"
```

**Location**: `crates/nebula-credential/tests/integration/kubernetes_integration.rs`

```rust
use kube::Client;
use nebula_credential::prelude::*;

#[tokio::test]
#[ignore] // Requires kind cluster running
async fn test_k8s_provider_crud_operations() {
    // Assumes kind cluster is running (setup-kind-for-tests.sh)
    let config = KubernetesSecretsConfig {
        namespace: "nebula-test".into(),
        kubeconfig_path: None, // Uses default ~/.kube/config
        secret_prefix: "nebula-test-".into(),
        ..Default::default()
    };
    
    let provider = KubernetesSecretsProvider::new(config).await.unwrap();
    
    let id = CredentialId::new("k8s_test_cred").unwrap();
    let key = EncryptionKey::generate().unwrap();
    let data = encrypt(&key, b"k8s_secret").unwrap();
    let metadata = CredentialMetadata {
        tags: vec!["provider:k8s".into()],
        ..Default::default()
    };
    let context = CredentialContext::new("k8s_test_user");
    
    // Store
    provider.store(&id, data.clone(), metadata.clone(), &context).await.unwrap();
    
    // Retrieve
    let (retrieved_data, retrieved_metadata) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(data.ciphertext, retrieved_data.ciphertext);
    
    // List
    let ids = provider.list(None, &context).await.unwrap();
    assert!(ids.contains(&id));
    
    // Delete
    provider.delete(&id, &context).await.unwrap();
    
    // Verify deleted
    let result = provider.retrieve(&id, &context).await;
    assert!(matches!(result, Err(StorageError::NotFound { .. })));
}

#[tokio::test]
#[ignore]
async fn test_k8s_namespace_isolation() {
    // Test that credentials in different namespaces are isolated
    let config1 = KubernetesSecretsConfig {
        namespace: "nebula-test".into(),
        ..Default::default()
    };
    
    let config2 = KubernetesSecretsConfig {
        namespace: "default".into(),
        ..Default::default()
    };
    
    let provider1 = KubernetesSecretsProvider::new(config1).await.unwrap();
    let provider2 = KubernetesSecretsProvider::new(config2).await.unwrap();
    
    let id = CredentialId::new("isolated_cred").unwrap();
    let key = EncryptionKey::generate().unwrap();
    let data = encrypt(&key, b"isolated").unwrap();
    let metadata = CredentialMetadata::default();
    let context = CredentialContext::new("test_user");
    
    // Store in nebula-test namespace
    provider1.store(&id, data, metadata, &context).await.unwrap();
    
    // Should NOT be visible in default namespace
    let result = provider2.retrieve(&id, &context).await;
    assert!(matches!(result, Err(StorageError::NotFound { .. })));
}
```

---

## Local Tests (Filesystem)

**Location**: `crates/nebula-credential/tests/integration/local_storage_integration.rs`

```rust
use tempfile::TempDir;
use nebula_credential::prelude::*;
use std::fs;

#[tokio::test]
async fn test_local_storage_atomic_writes() {
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig {
        base_path: temp_dir.path().to_path_buf(),
        create_dir: true,
        ..Default::default()
    };
    
    let provider = LocalStorageProvider::new(config).await.unwrap();
    
    let id = CredentialId::new("atomic_test").unwrap();
    let key = EncryptionKey::generate().unwrap();
    let data = encrypt(&key, b"atomic_write_test").unwrap();
    let metadata = CredentialMetadata::default();
    let context = CredentialContext::new("test_user");
    
    // Store should create file atomically
    provider.store(&id, data, metadata, &context).await.unwrap();
    
    // Verify file exists
    let file_path = temp_dir.path().join("atomic_test.enc.json");
    assert!(file_path.exists());
    
    // Verify no temporary files left behind
    let temp_files: Vec<_> = fs::read_dir(temp_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().unwrap().contains(".tmp"))
        .collect();
    
    assert_eq!(temp_files.len(), 0, "Temporary files should be cleaned up");
}

#[cfg(unix)]
#[tokio::test]
async fn test_local_storage_unix_permissions() {
    use std::os::unix::fs::PermissionsExt;
    
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig {
        base_path: temp_dir.path().to_path_buf(),
        create_dir: true,
        ..Default::default()
    };
    
    let provider = LocalStorageProvider::new(config).await.unwrap();
    
    let id = CredentialId::new("perm_test").unwrap();
    let key = EncryptionKey::generate().unwrap();
    let data = encrypt(&key, b"permission_test").unwrap();
    let metadata = CredentialMetadata::default();
    let context = CredentialContext::new("test_user");
    
    provider.store(&id, data, metadata, &context).await.unwrap();
    
    let file_path = temp_dir.path().join("perm_test.enc.json");
    let metadata = fs::metadata(&file_path).unwrap();
    let permissions = metadata.permissions();
    
    // Verify 0600 permissions (owner read/write only)
    assert_eq!(permissions.mode() & 0o777, 0o600);
}

#[cfg(windows)]
#[tokio::test]
async fn test_local_storage_windows_acl() {
    // Test Windows ACL permissions
    // Requires windows-acl crate integration
}

#[tokio::test]
async fn test_local_storage_concurrent_writes() {
    use tokio::task::JoinSet;
    
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig {
        base_path: temp_dir.path().to_path_buf(),
        create_dir: true,
        enable_locking: true,
        ..Default::default()
    };
    
    let provider = Arc::new(LocalStorageProvider::new(config).await.unwrap());
    
    let mut set = JoinSet::new();
    let key = Arc::new(EncryptionKey::generate().unwrap());
    
    // Spawn 50 concurrent writes to same credential
    for i in 0..50 {
        let provider = Arc::clone(&provider);
        let key = Arc::clone(&key);
        
        set.spawn(async move {
            let id = CredentialId::new("concurrent_cred").unwrap();
            let data = encrypt(&key, format!("value_{}", i).as_bytes()).unwrap();
            let metadata = CredentialMetadata {
                tags: vec![format!("iteration:{}", i)],
                ..Default::default()
            };
            let context = CredentialContext::new("test_user");
            
            provider.store(&id, data, metadata, &context).await
        });
    }
    
    // All operations should succeed (file locking prevents corruption)
    let mut success_count = 0;
    while let Some(result) = set.join_next().await {
        result.unwrap().unwrap();
        success_count += 1;
    }
    
    assert_eq!(success_count, 50);
    
    // File should contain last write (exact iteration depends on scheduling)
    let id = CredentialId::new("concurrent_cred").unwrap();
    let context = CredentialContext::new("test_user");
    let (_, metadata) = provider.retrieve(&id, &context).await.unwrap();
    
    // Verify metadata has exactly one tag (not corrupted with multiple writes)
    assert_eq!(metadata.tags.len(), 1);
    assert!(metadata.tags[0].starts_with("iteration:"));
}

#[tokio::test]
async fn test_local_storage_directory_autocreate() {
    let temp_dir = TempDir::new().unwrap();
    let nested_path = temp_dir.path().join("nested").join("directories").join("credentials");
    
    let config = LocalStorageConfig {
        base_path: nested_path.clone(),
        create_dir: true,
        ..Default::default()
    };
    
    // Directory doesn't exist yet
    assert!(!nested_path.exists());
    
    // Provider initialization should create it
    let provider = LocalStorageProvider::new(config).await.unwrap();
    
    // Directory should now exist
    assert!(nested_path.exists());
    assert!(nested_path.is_dir());
}

#[tokio::test]
async fn test_local_storage_file_corruption_recovery() {
    let temp_dir = TempDir::new().unwrap();
    let config = LocalStorageConfig {
        base_path: temp_dir.path().to_path_buf(),
        create_dir: true,
        ..Default::default()
    };
    
    let provider = LocalStorageProvider::new(config).await.unwrap();
    
    let id = CredentialId::new("corruption_test").unwrap();
    let key = EncryptionKey::generate().unwrap();
    let data = encrypt(&key, b"original_data").unwrap();
    let metadata = CredentialMetadata::default();
    let context = CredentialContext::new("test_user");
    
    // Store valid credential
    provider.store(&id, data, metadata, &context).await.unwrap();
    
    // Manually corrupt the file (simulate crash during write)
    let file_path = temp_dir.path().join("corruption_test.enc.json");
    fs::write(&file_path, b"corrupted json data").unwrap();
    
    // Retrieve should fail with ReadFailure (not panic)
    let result = provider.retrieve(&id, &context).await;
    assert!(matches!(result, Err(StorageError::ReadFailure(_))));
    
    // Overwrite with new valid data (recovery)
    let new_data = encrypt(&key, b"recovered_data").unwrap();
    let new_metadata = CredentialMetadata::default();
    provider.store(&id, new_data.clone(), new_metadata, &context).await.unwrap();
    
    // Should now succeed
    let (retrieved, _) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(new_data.ciphertext, retrieved.ciphertext);
}
```

---

## Docker Setup

### Starting Test Infrastructure

```bash
# Start all Docker containers for testing
cd crates/nebula-credential
docker-compose -f docker-compose.test.yml up -d

# Wait for health checks
docker-compose -f docker-compose.test.yml ps

# Setup kind cluster for K8s tests
./scripts/setup-kind-for-tests.sh

# Verify all services ready
curl http://localhost:8200/v1/sys/health  # Vault
curl http://localhost:4566/_localstack/health  # LocalStack
kubectl get nodes  # Kubernetes
```

### Cleanup

```bash
# Stop Docker containers
docker-compose -f docker-compose.test.yml down

# Delete kind cluster
kind delete cluster --name nebula-test

# Clean up test data
rm -rf /tmp/nebula-test-*
```

---

## Test Execution

### Run All Tests

```bash
# Run all unit tests (fast, no Docker)
cargo test --lib

# Run integration tests (requires Docker)
docker-compose -f docker-compose.test.yml up -d
cargo test --test integration

# Run K8s tests (requires kind cluster)
./scripts/setup-kind-for-tests.sh
cargo test --test kubernetes_integration -- --ignored

# Run ALL tests (unit + integration + K8s)
./scripts/run-all-tests.sh
```

### Run Provider-Specific Tests

```bash
# Local storage only
cargo test --test local_storage_integration

# Vault only (requires Docker)
docker-compose -f docker-compose.test.yml up -d vault
cargo test --test vault_integration

# AWS/LocalStack only
docker-compose -f docker-compose.test.yml up -d localstack
cargo test --test localstack_integration

# Mock provider only (very fast)
cargo test mock_tests
```

### Test Script

**Location**: `scripts/run-all-tests.sh`

```bash
#!/bin/bash
# Run complete test suite for nebula-credential

set -e

echo "=== Nebula Credential Test Suite ==="

# 1. Unit tests (no Docker)
echo ""
echo "[1/5] Running unit tests..."
cargo test --lib --package nebula-credential
echo "✓ Unit tests passed"

# 2. Start Docker services
echo ""
echo "[2/5] Starting Docker test services..."
cd crates/nebula-credential
docker-compose -f docker-compose.test.yml up -d
echo "Waiting for services to be ready..."
sleep 10
cd ../..

# 3. Integration tests (Vault, LocalStack)
echo ""
echo "[3/5] Running Docker integration tests..."
cargo test --test vault_integration --package nebula-credential
cargo test --test localstack_integration --package nebula-credential
echo "✓ Docker integration tests passed"

# 4. Local filesystem tests
echo ""
echo "[4/5] Running local storage integration tests..."
cargo test --test local_storage_integration --package nebula-credential
echo "✓ Local storage tests passed"

# 5. Kubernetes tests (optional, requires kind)
if command -v kind &> /dev/null; then
    echo ""
    echo "[5/5] Running Kubernetes integration tests..."
    ./scripts/setup-kind-for-tests.sh
    cargo test --test kubernetes_integration --package nebula-credential -- --ignored
    kind delete cluster --name nebula-test
    echo "✓ Kubernetes tests passed"
else
    echo ""
    echo "[5/5] Skipping Kubernetes tests (kind not installed)"
fi

# Cleanup
echo ""
echo "Cleaning up Docker containers..."
cd crates/nebula-credential
docker-compose -f docker-compose.test.yml down
cd ../..

echo ""
echo "=== All tests passed! ==="
```

---

## CI/CD Integration

### GitHub Actions Workflow

**Location**: `.github/workflows/test-storage-backends.yml`

```yaml
name: Storage Backends Tests

on:
  push:
    branches: [main, 002-storage-backends]
  pull_request:
    branches: [main]

jobs:
  unit-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          toolchain: 1.92
      
      - name: Cache cargo
        uses: actions/cache@v3
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
      
      - name: Run unit tests
        run: cargo test --lib --package nebula-credential
  
  integration-tests:
    runs-on: ubuntu-latest
    services:
      vault:
        image: hashicorp/vault:latest
        env:
          VAULT_DEV_ROOT_TOKEN_ID: test-token
        ports:
          - 8200:8200
        options: >-
          --cap-add=IPC_LOCK
      
      localstack:
        image: localstack/localstack:latest
        env:
          SERVICES: secretsmanager
          DEFAULT_REGION: us-east-1
        ports:
          - 4566:4566
    
    steps:
      - uses: actions/checkout@v3
      
      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
      
      - name: Wait for services
        run: |
          timeout 60 bash -c 'until curl -f http://localhost:8200/v1/sys/health; do sleep 2; done'
          timeout 60 bash -c 'until curl -f http://localhost:4566/_localstack/health; do sleep 2; done'
      
      - name: Run Vault integration tests
        env:
          VAULT_ADDR: http://localhost:8200
          VAULT_TOKEN: test-token
        run: cargo test --test vault_integration --package nebula-credential
      
      - name: Run AWS/LocalStack integration tests
        env:
          AWS_ACCESS_KEY_ID: test
          AWS_SECRET_ACCESS_KEY: test
          AWS_REGION: us-east-1
          AWS_ENDPOINT_URL: http://localhost:4566
        run: cargo test --test localstack_integration --package nebula-credential
  
  local-storage-tests:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    
    steps:
      - uses: actions/checkout@v3
      
      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
      
      - name: Run local storage tests
        run: cargo test --test local_storage_integration --package nebula-credential
  
  kubernetes-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
      
      - name: Setup kind
        uses: helm/kind-action@v1.5.0
        with:
          cluster_name: nebula-test
      
      - name: Setup test namespace and RBAC
        run: |
          kubectl create namespace nebula-test
          kubectl apply -f crates/nebula-credential/tests/fixtures/k8s-test-rbac.yaml
      
      - name: Run Kubernetes tests
        run: cargo test --test kubernetes_integration --package nebula-credential -- --ignored
```

---

## Test Coverage Goals

| Component | Unit Coverage | Integration Coverage |
|-----------|---------------|---------------------|
| LocalStorageProvider | 90%+ | 85%+ (filesystem ops) |
| AwsSecretsManagerProvider | 80%+ | 70%+ (LocalStack) |
| AzureKeyVaultProvider | 80%+ | 0% (no emulator) |
| HashiCorpVaultProvider | 85%+ | 80%+ (Docker) |
| KubernetesSecretsProvider | 85%+ | 75%+ (kind) |
| RetryPolicy | 95%+ | N/A |
| StorageMetrics | 90%+ | N/A |

```bash
# Generate coverage report
cargo tarpaulin --out Html --output-dir coverage --package nebula-credential
```

---

## Summary

✅ **Unit Tests**: MockStorageProvider for fast feedback (~50 tests, <1s total)  
✅ **Integration Tests**: Docker containers for Vault, LocalStack, kind (~15 tests, ~30s total)  
✅ **Local Tests**: Filesystem operations on temp directories (~10 tests, ~5s total)  
✅ **CI/CD**: GitHub Actions workflow with matrix testing (Linux, Windows, macOS)  

**Total Estimated Tests**: ~75 tests across all providers  
**Total Execution Time**: ~45 seconds (unit + integration)

**Next Steps**: Implement tests following TDD (write tests first, then implementation)
