---
title: StorageBackends
tags: [nebula, nebula-credential, docs]
status: published
created: 2025-08-24
---

# Storage Backends Reference

Complete reference for credential storage backends.

## Overview

Storage backends provide persistent storage for credentials with different trade-offs for security, performance, and scalability.

|Backend|Security|Performance|Scalability|Use Case|
|---|---|---|---|---|
|[Memory](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#memory)|Low|Very High|Low|Development, Testing|
|[File](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#file)|Medium|High|Low|Single instance, Small scale|
|[Database](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#database)|High|High|High|Production, Multi-instance|
|[Vault](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#hashicorp-vault)|Very High|Medium|High|Enterprise, High security|
|[AWS Secrets Manager](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#aws-secrets-manager)|Very High|Medium|High|AWS deployments|
|[Azure Key Vault](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#azure-key-vault)|Very High|Medium|High|Azure deployments|
|[Kubernetes Secrets](https://claude.ai/chat/da11ccfb-b59d-468b-aed9-a2cb812d14bf#kubernetes-secrets)|High|High|High|Kubernetes deployments|

## Storage Trait

All storage backends implement the `CredentialStorage` trait:

```rust
#[async_trait]
pub trait CredentialStorage: Send + Sync {
    /// Save credential
    async fn save(
        &self,
        id: &CredentialId,
        credential_type: &str,
        input: &Value,
        state: &Value,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError>;
    
    /// Load credential
    async fn load(
        &self,
        id: &CredentialId,
    ) -> Result<StoredCredential, StorageError>;
    
    /// Update credential state
    async fn update_state(
        &self,
        id: &CredentialId,
        state: &Value,
    ) -> Result<(), StorageError>;
    
    /// Delete credential
    async fn delete(
        &self,
        id: &CredentialId,
    ) -> Result<(), StorageError>;
    
    /// List credentials
    async fn list(
        &self,
        filter: Option<ListFilter>,
    ) -> Result<Vec<CredentialSummary>, StorageError>;
    
    /// Check if credential exists
    async fn exists(
        &self,
        id: &CredentialId,
    ) -> Result<bool, StorageError>;
    
    /// Get storage health
    async fn health_check(&self) -> Result<HealthStatus, StorageError>;
}
```

## Memory

In-memory storage for development and testing.

### Configuration

```rust
pub struct MemoryStorageConfig {
    /// Maximum number of credentials to store
    pub max_credentials: usize,
    
    /// Enable persistence to temp file on shutdown
    pub persist_on_shutdown: bool,
    
    /// Encryption for in-memory data
    pub encryption: Option<EncryptionConfig>,
}
```

### Usage

```rust
let storage = DatabaseStorage::new(DatabaseStorageConfig {
    url: "postgresql://user:pass@localhost/nebula".to_string(),
    pool: PoolConfig {
        min_connections: 2,
        max_connections: 10,
        connection_timeout: Duration::from_secs(30),
        idle_timeout: Duration::from_secs(600),
        max_lifetime: Duration::from_secs(1800),
    },
    encryption: Some(EncryptionConfig::aes256()),
    table_prefix: "cred_".to_string(),
    migration: MigrationMode::Auto,
});

let manager = CredentialManager::builder()
    .storage_backend(StorageBackend::Database {
        url: "postgresql://localhost/nebula".to_string(),
    })
    .build()
    .await?;
```

### Database Schema

```sql
-- Credentials table
CREATE TABLE cred_credentials (
    id UUID PRIMARY KEY,
    credential_type VARCHAR(100) NOT NULL,
    input_data JSONB NOT NULL,
    state_data JSONB NOT NULL,
    metadata JSONB,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL,
    expires_at TIMESTAMP WITH TIME ZONE,
    rotation_status VARCHAR(50),
    encryption_key_id VARCHAR(100),
    version INTEGER NOT NULL DEFAULT 1
);

-- Indices
CREATE INDEX idx_cred_type ON cred_credentials(credential_type);
CREATE INDEX idx_cred_expires ON cred_credentials(expires_at);
CREATE INDEX idx_cred_created ON cred_credentials(created_at);

-- Audit log table
CREATE TABLE cred_audit_log (
    id UUID PRIMARY KEY,
    credential_id UUID REFERENCES cred_credentials(id),
    action VARCHAR(50) NOT NULL,
    user_id VARCHAR(255),
    ip_address INET,
    timestamp TIMESTAMP WITH TIME ZONE NOT NULL,
    details JSONB
);
```

### Supported Databases

- PostgreSQL 12+
- MySQL 8.0+
- MariaDB 10.5+
- SQLite 3.35+
- CockroachDB 21+

### Characteristics

- ✅ **Pros:**
    
    - Full ACID compliance
    - Rich query capabilities
    - Multi-instance support
    - Backup and replication
    - Audit trail support
- ❌ **Cons:**
    
    - Requires database setup
    - Higher latency than memory/file
    - Database maintenance needed
    - Connection pool management

## HashiCorp Vault

Enterprise-grade secret management with HashiCorp Vault.

### Configuration

```rust
pub struct VaultStorageConfig {
    /// Vault server URL
    pub url: String,
    
    /// Authentication method
    pub auth: VaultAuth,
    
    /// Secret engine path
    pub mount_path: String,
    
    /// KV version (1 or 2)
    pub kv_version: u8,
    
    /// Namespace (Enterprise only)
    pub namespace: Option<String>,
    
    /// TLS configuration
    pub tls: Option<TlsConfig>,
    
    /// Retry configuration
    pub retry: RetryConfig,
}

pub enum VaultAuth {
    Token(String),
    AppRole {
        role_id: String,
        secret_id: String,
    },
    Kubernetes {
        role: String,
        jwt_path: String,
    },
    AwsIam {
        role: String,
        region: String,
    },
}
```

### Usage

```rust
let storage = VaultStorage::new(VaultStorageConfig {
    url: "https://vault.example.com:8200".to_string(),
    auth: VaultAuth::Token(std::env::var("VAULT_TOKEN")?),
    mount_path: "secret".to_string(),
    kv_version: 2,
    namespace: Some("nebula".to_string()),
    tls: Some(TlsConfig {
        ca_cert: Some(read_file("/etc/ssl/vault-ca.pem")?),
        client_cert: Some(read_file("/etc/ssl/client.pem")?),
        client_key: Some(read_file("/etc/ssl/client-key.pem")?),
        insecure: false,
    }),
    retry: RetryConfig::default(),
});

let manager = CredentialManager::builder()
    .storage_backend(StorageBackend::Vault {
        url: "https://vault.example.com:8200".to_string(),
        token: std::env::var("VAULT_TOKEN")?,
    })
    .build()
    .await?;
```

### Vault Policies

```hcl
# Nebula credential policy
path "secret/data/nebula/credentials/*" {
  capabilities = ["create", "read", "update", "delete", "list"]
}

path "secret/metadata/nebula/credentials/*" {
  capabilities = ["read", "list", "delete"]
}

path "sys/leases/renew" {
  capabilities = ["update"]
}

path "auth/token/renew-self" {
  capabilities = ["update"]
}
```

### Dynamic Secrets

```rust
// Configure dynamic secret generation
let dynamic_config = DynamicSecretConfig {
    backend: DynamicBackend::Database,
    role: "readonly".to_string(),
    ttl: Duration::from_hours(1),
    max_ttl: Duration::from_hours(24),
};

storage.configure_dynamic_secrets(dynamic_config).await?;

// Get dynamic credential
let dynamic_cred = storage.get_dynamic_credential("database").await?;
```

### Characteristics

- ✅ **Pros:**
    
    - Enterprise-grade security
    - Dynamic secret generation
    - Audit logging built-in
    - Encryption in transit and at rest
    - Access control policies
    - Secret rotation support
- ❌ **Cons:**
    
    - Requires Vault infrastructure
    - Network latency
    - Complex setup
    - Licensing costs (Enterprise)

## AWS Secrets Manager

AWS-native secret management service.

### Configuration

```rust
pub struct AwsSecretsManagerConfig {
    /// AWS region
    pub region: String,
    
    /// AWS credentials
    pub credentials: AwsCredentials,
    
    /// KMS key for encryption
    pub kms_key_id: Option<String>,
    
    /// Secret name prefix
    pub prefix: String,
    
    /// Automatic rotation
    pub rotation: Option<RotationConfig>,
    
    /// Tags for secrets
    pub tags: HashMap<String, String>,
}

pub enum AwsCredentials {
    Default,
    Static {
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
    },
    AssumeRole {
        role_arn: String,
        external_id: Option<String>,
    },
    InstanceProfile,
}
```

### Usage

```rust
let storage = AwsSecretsManagerStorage::new(AwsSecretsManagerConfig {
    region: "us-west-2".to_string(),
    credentials: AwsCredentials::Default,
    kms_key_id: Some("alias/nebula-credentials".to_string()),
    prefix: "nebula/".to_string(),
    rotation: Some(RotationConfig {
        enabled: true,
        schedule: "rate(30 days)".to_string(),
        lambda_arn: "arn:aws:lambda:us-west-2:123456789012:function:rotate".to_string(),
    }),
    tags: hashmap! {
        "Environment".to_string() => "production".to_string(),
        "Application".to_string() => "nebula".to_string(),
    },
});

let manager = CredentialManager::builder()
    .storage_backend(StorageBackend::AwsSecretsManager {
        region: "us-west-2".to_string(),
    })
    .build()
    .await?;
```

### IAM Policy

```json
{
    "Version": "2012-10-17",
    "Statement": [
        {
            "Effect": "Allow",
            "Action": [
                "secretsmanager:CreateSecret",
                "secretsmanager:GetSecretValue",
                "secretsmanager:UpdateSecret",
                "secretsmanager:DeleteSecret",
                "secretsmanager:DescribeSecret",
                "secretsmanager:ListSecrets",
                "secretsmanager:TagResource"
            ],
            "Resource": "arn:aws:secretsmanager:*:*:secret:nebula/*"
        },
        {
            "Effect": "Allow",
            "Action": [
                "kms:Decrypt",
                "kms:Encrypt",
                "kms:GenerateDataKey"
            ],
            "Resource": "arn:aws:kms:*:*:key/*"
        }
    ]
}
```

### Characteristics

- ✅ **Pros:**
    
    - Native AWS integration
    - Automatic rotation support
    - Cross-region replication
    - Fine-grained IAM policies
    - CloudTrail audit logging
    - High availability
- ❌ **Cons:**
    
    - AWS-only
    - Cost per secret
    - API rate limits
    - Regional service

## Azure Key Vault

Azure-native key and secret management.

### Configuration

```rust
pub struct AzureKeyVaultConfig {
    /// Key Vault URL
    pub vault_url: String,
    
    /// Authentication
    pub auth: AzureAuth,
    
    /// API version
    pub api_version: String,
    
    /// Retry configuration
    pub retry: RetryConfig,
}

pub enum AzureAuth {
    ServicePrincipal {
        tenant_id: String,
        client_id: String,
        client_secret: String,
    },
    ManagedIdentity {
        client_id: Option<String>,
    },
    Certificate {
        tenant_id: String,
        client_id: String,
        certificate: Vec<u8>,
    },
}
```

### Usage

```rust
let storage = AzureKeyVaultStorage::new(AzureKeyVaultConfig {
    vault_url: "https://nebula-vault.vault.azure.net".to_string(),
    auth: AzureAuth::ManagedIdentity {
        client_id: None,
    },
    api_version: "7.3".to_string(),
    retry: RetryConfig::default(),
});

let manager = CredentialManager::builder()
    .storage_backend(StorageBackend::AzureKeyVault {
        url: "https://nebula-vault.vault.azure.net".to_string(),
    })
    .build()
    .await?;
```

### Access Policy

```json
{
    "tenantId": "tenant-id",
    "objectId": "object-id",
    "permissions": {
        "secrets": [
            "get",
            "list",
            "set",
            "delete",
            "backup",
            "restore",
            "recover",
            "purge"
        ]
    }
}
```

## Kubernetes Secrets

Kubernetes-native secret storage.

### Configuration

```rust
pub struct KubernetesSecretsConfig {
    /// Namespace for secrets
    pub namespace: String,
    
    /// Label selector
    pub label_selector: Option<String>,
    
    /// Secret name prefix
    pub prefix: String,
    
    /// Encryption configuration
    pub encryption: Option<EncryptionConfig>,
    
    /// Service account
    pub service_account: Option<String>,
}
```

### Usage

```rust
let storage = KubernetesSecretsStorage::new(KubernetesSecretsConfig {
    namespace: "nebula-system".to_string(),
    label_selector: Some("app=nebula,component=credentials".to_string()),
    prefix: "nebula-cred-".to_string(),
    encryption: Some(EncryptionConfig::aes256()),
    service_account: Some("nebula-credentials".to_string()),
});

let manager = CredentialManager::builder()
    .storage_backend(StorageBackend::KubernetesSecrets {
        namespace: "nebula-system".to_string(),
    })
    .build()
    .await?;
```

### RBAC Configuration

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: nebula-credentials
  namespace: nebula-system
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["create", "get", "list", "update", "delete", "watch"]

---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: nebula-credentials
  namespace: nebula-system
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: nebula-credentials
subjects:
- kind: ServiceAccount
  name: nebula-credentials
  namespace: nebula-system
```

## Custom Storage Backend

Implement custom storage backend for specific requirements.

### Implementation

```rust
use nebula_credential::storage::*;

pub struct CustomStorage {
    // Your implementation
}

#[async_trait]
impl CredentialStorage for CustomStorage {
    async fn save(
        &self,
        id: &CredentialId,
        credential_type: &str,
        input: &Value,
        state: &Value,
        metadata: &CredentialMetadata,
    ) -> Result<(), StorageError> {
        // Implementation
        Ok(())
    }
    
    async fn load(
        &self,
        id: &CredentialId,
    ) -> Result<StoredCredential, StorageError> {
        // Implementation
        todo!()
    }
    
    // ... implement other methods
}

// Use custom storage
let storage = CustomStorage::new();
let manager = CredentialManager::builder()
    .storage_backend(StorageBackend::Custom(Box::new(storage)))
    .build()
    .await?;
```

## Storage Migration

### Migrating Between Backends

```rust
use nebula_credential::migration::*;

let migrator = StorageMigrator::new(
    source_storage,
    target_storage,
);

// Migrate all credentials
let report = migrator.migrate_all().await?;
println!("Migrated {} credentials", report.success_count);
println!("Failed: {}", report.failure_count);

// Migrate with transformation
let report = migrator
    .with_transformer(|cred| {
        // Transform credential during migration
        cred.metadata.insert("migrated_at", Utc::now().to_string());
        cred
    })
    .migrate_all()
    .await?;
```

## Performance Comparison

|Backend|Write Latency|Read Latency|Query Performance|Throughput|
|---|---|---|---|---|
|Memory|< 1ms|< 1ms|Excellent|Very High|
|File|1-5ms|1-3ms|Poor|High|
|Database|5-20ms|2-10ms|Excellent|High|
|Vault|20-50ms|15-30ms|Good|Medium|
|AWS SM|30-100ms|20-50ms|Good|Medium|
|Azure KV|30-100ms|20-50ms|Good|Medium|
|K8s Secrets|10-30ms|5-15ms|Fair|High|

## Choosing a Storage Backend

### Decision Tree

```mermaid
graph TD
    A[Choose Storage] --> B{Environment?}
    B -->|Development| C[Memory]
    B -->|Testing| D[File or Memory]
    B -->|Production| E{Scale?}
    E -->|Single Instance| F[File with Encryption]
    E -->|Multi Instance| G{Platform?}
    G -->|Self-Hosted| H[Database or Vault]
    G -->|AWS| I[AWS Secrets Manager]
    G -->|Azure| J[Azure Key Vault]
    G -->|Kubernetes| K[K8s Secrets or Vault]
```

## Related

- [Credential Manager](https://claude.ai/chat/CredentialManager.md)
- [Security Features](https://claude.ai/chat/SecurityFeatures.md)
- [Configuration](https://claude.ai/chat/Configuration.md) let storage = MemoryStorage::new(MemoryStorageConfig { max_credentials: 1000, persist_on_shutdown: false, encryption: None, });

let manager = CredentialManager::builder() .storage_backend(StorageBackend::Memory) .build() .await?;

````

### Characteristics

- ✅ **Pros:**
  - Zero latency
  - No external dependencies
  - Perfect for testing
  - Simple setup

- ❌ **Cons:**
  - Data lost on restart
  - Not suitable for production
  - Limited by memory
  - Single instance only

## File

File-based storage with encryption.

### Configuration

```rust
pub struct FileStorageConfig {
    /// Directory for credential files
    pub path: PathBuf,
    
    /// Encryption configuration
    pub encryption: EncryptionConfig,
    
    /// File permissions (Unix)
    pub permissions: u32,
    
    /// Backup configuration
    pub backup: Option<BackupConfig>,
    
    /// File format
    pub format: FileFormat,
}

pub enum FileFormat {
    Json,
    Yaml,
    Binary,
    Encrypted,
}
````

### Usage

```rust
let storage = FileStorage::new(FileStorageConfig {
    path: PathBuf::from("/var/lib/nebula/credentials"),
    encryption: EncryptionConfig::aes256(),
    permissions: 0o600,
    backup: Some(BackupConfig {
        enabled: true,
        retention: Duration::days(30),
        location: PathBuf::from("/backup/credentials"),
    }),
    format: FileFormat::Encrypted,
});

let manager = CredentialManager::builder()
    .storage_backend(StorageBackend::File {
        path: "/var/lib/nebula/credentials".into(),
    })
    .build()
    .await?;
```

### File Structure

```
/var/lib/nebula/credentials/
├── metadata.json           # Storage metadata
├── credentials/
│   ├── cred_abc123.enc    # Encrypted credential
│   ├── cred_def456.enc
│   └── ...
├── indices/
│   ├── by_type.idx        # Type index
│   └── by_created.idx     # Creation time index
└── backups/
    └── 2024-08-24/
        └── ...
```

### Characteristics

- ✅ **Pros:**
    
    - Simple deployment
    - Built-in encryption
    - Low latency
    - Backup support
- ❌ **Cons:**
    
    - Single instance only
    - Limited query capabilities
    - Manual scaling
    - File system dependent

## Database

Relational database storage with full query support.

### Configuration

```rust
pub struct DatabaseStorageConfig {
    /// Database connection URL
    pub url: String,
    
    /// Connection pool configuration
    pub pool: PoolConfig,
    
    /// Encryption at rest
    pub encryption: Option<EncryptionConfig>,
    
    /// Table prefix
    pub table_prefix: String,
    
    /// Migration mode
    pub migration: MigrationMode,
}

pub struct PoolConfig {
    pub min_connections: u32,
    pub max_connections: u32,
    pub connection_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_lifetime: Duration,
}

pub enum MigrationMode {
    Auto,
    Manual,
    Validate,
}
```

### Usage