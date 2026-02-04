# Data Model: Production-Ready Storage Backends

**Feature**: 002-storage-backends  
**Date**: 2026-02-03  
**Dependencies**: Phase 1 Core Credential Abstractions (001-credential-core-abstractions)

This document defines the data model for all storage provider implementations in Phase 2.

---

## Table of Contents

1. [Entity Overview](#entity-overview)
2. [Provider Configurations](#provider-configurations)
3. [Shared Utilities](#shared-utilities)
4. [Provider Implementations](#provider-implementations)
5. [Validation Rules](#validation-rules)
6. [State Transitions](#state-transitions)

---

## Entity Overview

### Entity Hierarchy

```
StorageProvider (trait from Phase 1)
    ├── LocalStorageProvider
    ├── AwsSecretsManagerProvider
    ├── AzureKeyVaultProvider
    ├── HashiCorpVaultProvider
    └── KubernetesSecretsProvider

ProviderConfig (new trait)
    ├── LocalStorageConfig
    ├── AwsSecretsManagerConfig
    ├── AzureKeyVaultConfig
    ├── VaultConfig
    └── KubernetesSecretsConfig

RetryPolicy (new struct)
StorageMetrics (new struct, foundation for Phase 8)
CredentialFile (new struct, local storage only)
```

### Relationship Diagram

```
┌─────────────────────────┐
│   StorageProvider       │ (Phase 1 trait)
│   ──────────────────    │
│   + store()             │
│   + retrieve()          │
│   + delete()            │
│   + list()              │
│   + exists()            │
└─────────────────────────┘
            △
            │ implements
            │
   ┌────────┴────────┬────────┬────────┬────────┐
   │                 │        │        │        │
┌──▼───────┐  ┌────▼─────┐  ┌▼───────┐┌▼──────┐┌▼──────┐
│ Local    │  │ AWS      │  │ Azure  ││ Vault ││ K8s   │
│ Storage  │  │ Secrets  │  │ Key    ││       ││Secret │
│ Provider │  │ Manager  │  │ Vault  ││       ││       │
└──────────┘  └──────────┘  └────────┘└───────┘└───────┘
     │             │             │         │        │
     │ has         │ has         │ has     │ has    │ has
     ▼             ▼             ▼         ▼        ▼
┌──────────┐  ┌─────────┐  ┌────────┐┌────────┐┌────────┐
│ Local    │  │ Aws     │  │ Azure  ││ Vault  ││ K8s    │
│ Storage  │  │ Secrets │  │ Key    ││ Config ││ Secrets│
│ Config   │  │ Manager │  │ Vault  ││        ││ Config │
│          │  │ Config  │  │ Config ││        ││        │
└──────────┘  └─────────┘  └────────┘└────────┘└────────┘
```

---

## Provider Configurations

### Base Configuration Trait

```rust
/// Trait for storage provider configuration
///
/// All provider configs must implement this trait to ensure
/// validation before initialization.
pub trait ProviderConfig: Send + Sync + Clone {
    /// Validate configuration parameters
    ///
    /// # Returns
    /// * `Ok(())` - Configuration is valid
    /// * `Err(ConfigError)` - Configuration has errors with details
    fn validate(&self) -> Result<(), ConfigError>;
    
    /// Get provider name for logging and metrics
    fn provider_name(&self) -> &'static str;
}
```

### LocalStorageConfig

**Purpose**: Configuration for file-based local encrypted storage.

**Fields**:
```rust
#[derive(Debug, Clone)]
pub struct LocalStorageConfig {
    /// Base directory for credential storage
    /// Example: ~/.local/share/nebula/credentials
    pub base_path: PathBuf,
    
    /// Automatically create directory if it doesn't exist
    /// Default: true
    pub create_dir: bool,
    
    /// File extension for credential files
    /// Default: "enc.json"
    pub file_extension: String,
    
    /// Enable file locking for concurrent access
    /// Default: true
    pub enable_locking: bool,
}
```

**Validation Rules**:
- `base_path` must be absolute path
- `base_path` must be writable (if exists)
- `file_extension` must not contain path separators
- If `create_dir` is false and `base_path` doesn't exist, validation fails

**Default Implementation**:
```rust
impl Default for LocalStorageConfig {
    fn default() -> Self {
        Self {
            base_path: get_default_storage_path(),
            create_dir: true,
            file_extension: "enc.json".into(),
            enable_locking: true,
        }
    }
}
```

### AwsSecretsManagerConfig

**Purpose**: Configuration for AWS Secrets Manager integration.

**Fields**:
```rust
#[derive(Debug, Clone)]
pub struct AwsSecretsManagerConfig {
    /// AWS region (None = auto-detect from environment)
    /// Example: Some("us-east-1")
    pub region: Option<String>,
    
    /// Secret name prefix for namespacing
    /// Example: "nebula/credentials/"
    pub secret_prefix: String,
    
    /// Timeout for AWS API calls
    /// Default: 5 seconds (reads), 10 seconds (writes)
    pub timeout: Duration,
    
    /// Retry policy for transient errors
    pub retry_policy: RetryPolicy,
    
    /// KMS key ID for encryption (None = AWS managed key)
    /// Example: Some("arn:aws:kms:region:account:key/abc123...")
    pub kms_key_id: Option<String>,
    
    /// Tags to apply to all secrets
    pub default_tags: HashMap<String, String>,
}
```

**Validation Rules**:
- `secret_prefix` must not exceed 512 characters (AWS limit)
- `secret_prefix` must not contain invalid characters: `<>:{}"'/|?*`
- `timeout` must be between 1 second and 60 seconds
- If `region` is Some, must be valid AWS region code
- If `kms_key_id` is Some, must match ARN format or alias format

**Default Implementation**:
```rust
impl Default for AwsSecretsManagerConfig {
    fn default() -> Self {
        Self {
            region: None, // Auto-detect
            secret_prefix: "nebula/credentials/".into(),
            timeout: Duration::from_secs(5),
            retry_policy: RetryPolicy::default(),
            kms_key_id: None,
            default_tags: HashMap::new(),
        }
    }
}
```

### AzureKeyVaultConfig

**Purpose**: Configuration for Azure Key Vault integration.

**Fields**:
```rust
#[derive(Debug, Clone)]
pub struct AzureKeyVaultConfig {
    /// Key Vault URL
    /// Example: "https://my-vault.vault.azure.net/"
    pub vault_url: String,
    
    /// Credential type for authentication
    pub credential_type: AzureCredentialType,
    
    /// Timeout for Azure API calls
    /// Default: 5 seconds (reads), 10 seconds (writes)
    pub timeout: Duration,
    
    /// Retry policy for transient errors
    pub retry_policy: RetryPolicy,
    
    /// Secret name prefix for namespacing
    /// Example: "nebula-credentials-"
    pub secret_prefix: String,
    
    /// Default tags for all secrets
    pub default_tags: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AzureCredentialType {
    /// Managed Identity (system-assigned or user-assigned)
    ManagedIdentity {
        /// Client ID for user-assigned identity (None = system-assigned)
        client_id: Option<String>,
    },
    
    /// Service Principal with client secret
    ServicePrincipal {
        tenant_id: String,
        client_id: String,
        client_secret: SecretString,
    },
    
    /// Developer Tools (az cli, azd cli)
    DeveloperTools,
}
```

**Validation Rules**:
- `vault_url` must start with `https://`
- `vault_url` must contain `.vault.azure.net`
- `vault_url` must be valid URL
- `timeout` must be between 1 second and 60 seconds
- `secret_prefix` must not exceed 127 characters (Azure limit)
- For `ServicePrincipal`: tenant_id and client_id must be valid GUIDs

**Default Implementation**:
```rust
impl Default for AzureKeyVaultConfig {
    fn default() -> Self {
        Self {
            vault_url: std::env::var("AZURE_KEYVAULT_URL")
                .unwrap_or_else(|_| "https://example.vault.azure.net/".into()),
            credential_type: AzureCredentialType::ManagedIdentity { client_id: None },
            timeout: Duration::from_secs(5),
            retry_policy: RetryPolicy::default(),
            secret_prefix: "nebula-credentials-".into(),
            default_tags: HashMap::new(),
        }
    }
}
```

### VaultConfig

**Purpose**: Configuration for HashiCorp Vault integration.

**Fields**:
```rust
#[derive(Debug, Clone)]
pub struct VaultConfig {
    /// Vault server address
    /// Example: "https://vault.example.com:8200"
    pub address: String,
    
    /// Authentication method
    pub auth_method: VaultAuthMethod,
    
    /// KV v2 mount path
    /// Default: "secret"
    pub mount_path: String,
    
    /// Secret path prefix for namespacing
    /// Example: "nebula/credentials"
    pub path_prefix: String,
    
    /// Namespace (Vault Enterprise only, None for OSS)
    /// Example: Some("org1/team-a")
    pub namespace: Option<String>,
    
    /// Timeout for Vault API calls
    /// Default: 5 seconds
    pub timeout: Duration,
    
    /// Retry policy for transient errors
    pub retry_policy: RetryPolicy,
    
    /// Enable TLS certificate verification
    /// Default: true (NEVER disable in production)
    pub tls_verify: bool,
    
    /// Token renewal threshold (renew when TTL drops below this)
    /// Default: 1 hour
    pub token_renewal_threshold: Duration,
}

#[derive(Debug, Clone)]
pub enum VaultAuthMethod {
    /// Token-based authentication
    Token {
        /// Static token (service token, root token)
        token: SecretString,
    },
    
    /// AppRole authentication (recommended for production)
    AppRole {
        /// Role ID (static, can be embedded)
        role_id: String,
        
        /// Secret ID (dynamic, short-lived)
        secret_id: SecretString,
        
        /// AppRole mount path
        /// Default: "approle"
        mount_path: String,
    },
}
```

**Validation Rules**:
- `address` must be valid URL
- `address` must use `https://` in production (http only for dev)
- `mount_path` must not start or end with `/`
- `path_prefix` must not start with `/`, can end with `/`
- `timeout` must be between 1 second and 60 seconds
- `tls_verify` must be true if address is https://
- `token_renewal_threshold` must be less than typical token TTL

**Default Implementation**:
```rust
impl Default for VaultConfig {
    fn default() -> Self {
        Self {
            address: std::env::var("VAULT_ADDR")
                .unwrap_or_else(|_| "http://127.0.0.1:8200".into()),
            auth_method: VaultAuthMethod::Token {
                token: SecretString::new(
                    std::env::var("VAULT_TOKEN").unwrap_or_default()
                ),
            },
            mount_path: "secret".into(),
            path_prefix: "nebula/credentials".into(),
            namespace: std::env::var("VAULT_NAMESPACE").ok(),
            timeout: Duration::from_secs(5),
            retry_policy: RetryPolicy::default(),
            tls_verify: true,
            token_renewal_threshold: Duration::from_secs(3600),
        }
    }
}
```

### KubernetesSecretsConfig

**Purpose**: Configuration for Kubernetes Secrets integration.

**Fields**:
```rust
#[derive(Debug, Clone)]
pub struct KubernetesSecretsConfig {
    /// Kubernetes namespace for secrets
    /// Example: "nebula"
    pub namespace: String,
    
    /// Path to kubeconfig file (None = in-cluster service account)
    /// Example: Some(PathBuf::from("~/.kube/config"))
    pub kubeconfig_path: Option<PathBuf>,
    
    /// Secret name prefix for namespacing
    /// Example: "nebula-cred-"
    pub secret_prefix: String,
    
    /// Timeout for K8s API calls
    /// Default: 5 seconds
    pub timeout: Duration,
    
    /// Retry policy for transient errors
    pub retry_policy: RetryPolicy,
    
    /// Default labels for all secrets
    pub default_labels: HashMap<String, String>,
    
    /// Default annotations for all secrets
    pub default_annotations: HashMap<String, String>,
}
```

**Validation Rules**:
- `namespace` must be valid Kubernetes namespace name:
  - Max 63 characters
  - Lowercase alphanumeric, `-` allowed
  - Must start and end with alphanumeric
- `secret_prefix` must be valid Kubernetes name prefix (max 63 chars total including suffix)
- `timeout` must be between 1 second and 60 seconds
- If `kubeconfig_path` is Some, file must exist and be readable
- `default_labels` keys and values must be valid Kubernetes label format

**Default Implementation**:
```rust
impl Default for KubernetesSecretsConfig {
    fn default() -> Self {
        let mut default_labels = HashMap::new();
        default_labels.insert("app.kubernetes.io/name".into(), "nebula-credential".into());
        default_labels.insert("app.kubernetes.io/component".into(), "credential-storage".into());
        
        Self {
            namespace: std::env::var("KUBERNETES_NAMESPACE")
                .unwrap_or_else(|_| "default".into()),
            kubeconfig_path: None, // Auto-detect (in-cluster or ~/.kube/config)
            secret_prefix: "nebula-cred-".into(),
            timeout: Duration::from_secs(5),
            retry_policy: RetryPolicy::default(),
            default_labels,
            default_annotations: HashMap::new(),
        }
    }
}
```

---

## Shared Utilities

### RetryPolicy

**Purpose**: Unified retry configuration for exponential backoff across all cloud providers.

**Fields**:
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Maximum number of retry attempts
    /// Default: 5
    pub max_retries: u32,
    
    /// Initial delay in milliseconds
    /// Default: 100ms
    pub base_delay_ms: u64,
    
    /// Maximum delay in milliseconds (cap for exponential growth)
    /// Default: 30,000ms (30 seconds)
    pub max_delay_ms: u64,
    
    /// Backoff multiplier (exponential growth factor)
    /// Default: 2.0 (doubles each retry)
    pub multiplier: f64,
    
    /// Add jitter to prevent thundering herd
    /// Default: true (±25% randomness)
    pub jitter: bool,
}
```

**Validation Rules**:
- `max_retries` must be between 0 and 10
- `base_delay_ms` must be between 10ms and 10,000ms
- `max_delay_ms` must be greater than `base_delay_ms`
- `multiplier` must be >= 1.0 and <= 10.0

**Default Implementation**:
```rust
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay_ms: 100,
            max_delay_ms: 30_000,
            multiplier: 2.0,
            jitter: true,
        }
    }
}
```

**Usage Pattern**:
```rust
// Retry sequence with default policy:
// Attempt 1: Immediate
// Attempt 2: 100ms ± 25% jitter
// Attempt 3: 200ms ± 25% jitter
// Attempt 4: 400ms ± 25% jitter
// Attempt 5: 800ms ± 25% jitter
// Attempt 6: 1600ms ± 25% jitter
// Give up after 6 total attempts
```

### StorageMetrics

**Purpose**: Per-provider metrics for observability (foundation for Phase 8).

**Fields**:
```rust
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct StorageMetrics {
    /// Total store operations
    pub store_count: AtomicU64,
    
    /// Sum of store operation latencies (milliseconds)
    pub store_latency_sum_ms: AtomicU64,
    
    /// Total retrieve operations
    pub retrieve_count: AtomicU64,
    
    /// Sum of retrieve operation latencies (milliseconds)
    pub retrieve_latency_sum_ms: AtomicU64,
    
    /// Total delete operations
    pub delete_count: AtomicU64,
    
    /// Total list operations
    pub list_count: AtomicU64,
    
    /// Total errors across all operations
    pub error_count: AtomicU64,
    
    /// Total retries attempted
    pub retry_count: AtomicU64,
}
```

**Methods**:
```rust
impl StorageMetrics {
    /// Record an operation with duration and success status
    pub fn record_operation(
        &self,
        operation: &str,
        duration: Duration,
        success: bool,
    ) {
        let latency_ms = duration.as_millis() as u64;
        
        match operation {
            "store" => {
                self.store_count.fetch_add(1, Ordering::Relaxed);
                self.store_latency_sum_ms.fetch_add(latency_ms, Ordering::Relaxed);
            }
            "retrieve" => {
                self.retrieve_count.fetch_add(1, Ordering::Relaxed);
                self.retrieve_latency_sum_ms.fetch_add(latency_ms, Ordering::Relaxed);
            }
            "delete" => {
                self.delete_count.fetch_add(1, Ordering::Relaxed);
            }
            "list" => {
                self.list_count.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        
        if !success {
            self.error_count.fetch_add(1, Ordering::Relaxed);
        }
    }
    
    /// Record a retry attempt
    pub fn record_retry(&self) {
        self.retry_count.fetch_add(1, Ordering::Relaxed);
    }
    
    /// Calculate average store latency in milliseconds
    pub fn avg_store_latency_ms(&self) -> u64 {
        let count = self.store_count.load(Ordering::Relaxed);
        if count == 0 { return 0; }
        self.store_latency_sum_ms.load(Ordering::Relaxed) / count
    }
    
    /// Calculate average retrieve latency in milliseconds
    pub fn avg_retrieve_latency_ms(&self) -> u64 {
        let count = self.retrieve_count.load(Ordering::Relaxed);
        if count == 0 { return 0; }
        self.retrieve_latency_sum_ms.load(Ordering::Relaxed) / count
    }
    
    /// Calculate error rate (errors / total operations)
    pub fn error_rate(&self) -> f64 {
        let total = self.store_count.load(Ordering::Relaxed)
            + self.retrieve_count.load(Ordering::Relaxed)
            + self.delete_count.load(Ordering::Relaxed)
            + self.list_count.load(Ordering::Relaxed);
        
        if total == 0 { return 0.0; }
        
        self.error_count.load(Ordering::Relaxed) as f64 / total as f64
    }
}
```

### CredentialFile

**Purpose**: File format for local encrypted credential storage (LocalStorageProvider only).

**Fields**:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialFile {
    /// File format version for migration
    pub version: u32,
    
    /// Encrypted credential data (ciphertext, nonce, auth tag)
    pub encrypted_data: EncryptedData,
    
    /// Non-sensitive metadata (timestamps, tags, created_by)
    pub metadata: CredentialMetadata,
    
    /// Salt used for key derivation (optional, for password-based encryption)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salt: Option<[u8; 16]>,
}
```

**Validation Rules**:
- `version` must be <= `CredentialFile::CURRENT_VERSION`
- `encrypted_data` must have valid nonce (12 bytes) and tag (16 bytes)
- `metadata` must pass `CredentialMetadata` validation rules

**Versioning**:
```rust
impl CredentialFile {
    pub const CURRENT_VERSION: u32 = 1;
    
    pub fn new(encrypted_data: EncryptedData, metadata: CredentialMetadata) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            encrypted_data,
            metadata,
            salt: None,
        }
    }
    
    /// Check if migration is needed
    pub fn needs_migration(&self) -> bool {
        self.version < Self::CURRENT_VERSION
    }
}
```

---

## Provider Implementations

### LocalStorageProvider

**Purpose**: File-based local encrypted credential storage with atomic writes and secure permissions.

**Fields**:
```rust
pub struct LocalStorageProvider {
    config: LocalStorageConfig,
    metrics: Arc<StorageMetrics>,
}
```

**Key Operations**:
- **store()**: Atomic write to `{base_path}/{id}.enc.json` with 0600 permissions
- **retrieve()**: Read file with shared lock, deserialize JSON, return `(EncryptedData, CredentialMetadata)`
- **delete()**: Delete file (idempotent, succeeds if already deleted)
- **list()**: Scan directory, filter by metadata tags, return `Vec<CredentialId>`
- **exists()**: Check file existence without reading

**Relationships**:
- Uses `CredentialFile` for serialization format
- Uses `atomic_write()` utility for corruption prevention
- Uses `fs2::FileExt` for file locking
- Uses platform-specific permission setting (0600 Unix, ACL Windows)

### AwsSecretsManagerProvider

**Purpose**: AWS Secrets Manager integration with KMS encryption and automatic retries.

**Fields**:
```rust
pub struct AwsSecretsManagerProvider {
    client: aws_sdk_secretsmanager::Client,
    config: AwsSecretsManagerConfig,
    metrics: Arc<StorageMetrics>,
}
```

**Key Operations**:
- **store()**: `CreateSecret` or `UpdateSecret`, apply KMS encryption, tag with metadata
- **retrieve()**: `GetSecretValue`, deserialize JSON to `(EncryptedData, CredentialMetadata)`
- **delete()**: `DeleteSecret` with recovery window (7-30 days configurable)
- **list()**: `ListSecrets` with pagination, filter by prefix and tags
- **exists()**: `DescribeSecret` (lighter than `GetSecretValue`)

**Relationships**:
- Uses `AwsSecretsManagerConfig` for client initialization
- Converts `CredentialMetadata` to AWS tags (max 50 tags per secret)
- Maps AWS errors (`ResourceNotFoundException`, `AccessDeniedException`) to `StorageError`
- Leverages AWS SDK built-in retry for 503 errors

### AzureKeyVaultProvider

**Purpose**: Azure Key Vault integration with Managed Identity and RBAC.

**Fields**:
```rust
pub struct AzureKeyVaultProvider {
    client: azure_security_keyvault_secrets::SecretClient,
    config: AzureKeyVaultConfig,
    metrics: Arc<StorageMetrics>,
}
```

**Key Operations**:
- **store()**: `SetSecret` with tags and expiration, automatic token refresh
- **retrieve()**: `GetSecret`, deserialize JSON to `(EncryptedData, CredentialMetadata)`
- **delete()**: `DeleteSecret` (soft-delete by default, configurable recovery period)
- **list()**: `ListSecrets` with pagination, filter by prefix and tags
- **exists()**: `GetSecret` with metadata-only request

**Relationships**:
- Uses `AzureKeyVaultConfig` for authentication (Managed Identity or Service Principal)
- Converts `CredentialMetadata` to Azure tags (max 15 tags per secret)
- Maps Azure errors (`Forbidden`, `NotFound`) to `StorageError` with RBAC context
- Automatic token refresh when TTL expires

### HashiCorpVaultProvider

**Purpose**: HashiCorp Vault KV v2 integration with versioning and token renewal.

**Fields**:
```rust
pub struct HashiCorpVaultProvider {
    client: Arc<vaultrs::client::VaultClient>,
    config: VaultConfig,
    metrics: Arc<StorageMetrics>,
    token_renewal_task: Option<JoinHandle<()>>,
}
```

**Key Operations**:
- **store()**: `kv2::set()` with automatic versioning, path: `{mount}/{path_prefix}/{id}`
- **retrieve()**: `kv2::read()` for latest version, deserialize JSON to `(EncryptedData, CredentialMetadata)`
- **delete()**: `kv2::delete_metadata()` (permanent) or `kv2::delete_latest()` (soft-delete)
- **list()**: `kv2::list()` with path prefix, return keys only
- **exists()**: `kv2::read()` metadata-only request

**Relationships**:
- Uses `VaultConfig` for client initialization and auth method (Token or AppRole)
- Spawns background task for automatic token renewal (when TTL < threshold)
- Preserves version metadata from KV v2 engine (current_version, versions map)
- Maps Vault errors (`permission denied`, `not found`) to `StorageError` with policy context

### KubernetesSecretsProvider

**Purpose**: Kubernetes Secrets integration with namespace isolation and RBAC.

**Fields**:
```rust
pub struct KubernetesSecretsProvider {
    client: kube::Client,
    secrets_api: kube::Api<k8s_openapi::api::core::v1::Secret>,
    config: KubernetesSecretsConfig,
    metrics: Arc<StorageMetrics>,
}
```

**Key Operations**:
- **store()**: Create or update Secret in namespace, type `Opaque`, base64 encode data
- **retrieve()**: Get Secret, base64 decode, deserialize JSON to `(EncryptedData, CredentialMetadata)`
- **delete()**: Delete Secret (idempotent)
- **list()**: List Secrets with label selector, filter by prefix
- **exists()**: Get Secret metadata-only request

**Relationships**:
- Uses `KubernetesSecretsConfig` for namespace and authentication
- Converts `CredentialMetadata` to K8s labels (max 63 chars) and annotations (max 256KB)
- Maps K8s errors (`Forbidden`, `NotFound`) to `StorageError` with RBAC context
- Automatically detects in-cluster (service account) vs out-of-cluster (kubeconfig)

---

## Validation Rules

### Provider-Specific Size Limits

| Provider | Hard Limit | Recommended Max | Notes |
|----------|------------|-----------------|-------|
| Local Storage | No limit (filesystem) | 1MB | Performance degrades with large files |
| AWS Secrets Manager | 64KB | 32KB | Base64 encoding included in limit |
| Azure Key Vault | 25KB | 10KB | Base64 encoding included in limit |
| HashiCorp Vault | Configurable (default 1MB) | 256KB | Depends on Vault server config |
| Kubernetes Secrets | 1MB | 256KB | Base64 overhead ~33%, etcd limit 1.5MB |

**Validation Function**:
```rust
impl StorageProvider for AwsSecretsManagerProvider {
    async fn store(&self, id: &CredentialId, data: EncryptedData, ...) 
        -> Result<(), StorageError> 
    {
        const AWS_SECRET_MAX_SIZE: usize = 64 * 1024; // 64KB
        
        if data.total_size() > AWS_SECRET_MAX_SIZE {
            return Err(StorageError::CredentialTooLarge {
                size: data.total_size(),
                limit: AWS_SECRET_MAX_SIZE,
                provider: "AWS Secrets Manager",
            });
        }
        
        // Proceed with storage...
    }
}
```

### Metadata Conversion Rules

**AWS Tags**:
- Max 50 tags per secret
- Key: 1-128 characters
- Value: 0-256 characters
- Reserved prefix: `aws:`

**Azure Tags**:
- Max 15 tags per secret
- Key: No strict length limit
- Value: No strict length limit

**K8s Labels** (for filtering):
- Max 63 characters per key/value
- Format: `(prefix/)name` where prefix is optional
- Alphanumeric, `-`, `_`, `.` allowed

**K8s Annotations** (for metadata):
- Max 256KB total
- No format restrictions

**Vault Metadata**:
- Stored in KV v2 metadata path
- No strict limits
- Preserves version history

---

## State Transitions

### Provider Lifecycle

```
┌──────────────┐
│ Uninitialized│
└──────┬───────┘
       │ new() / Builder::build()
       ▼
┌──────────────┐
│ Configured   │ (config validated, client not created)
└──────┬───────┘
       │ init() / connect()
       ▼
┌──────────────┐
│ Connected    │ (client created, authenticated)
└──────┬───────┘
       │ store() / retrieve() / delete() / list()
       ▼
┌──────────────┐
│ Operating    │ (normal operations, metrics collected)
└──────┬───────┘
       │ Error occurs
       ▼
┌──────────────┐
│ Retrying     │ (exponential backoff, retry logic)
└──────┬───────┘
       │ Success: return to Operating
       │ Max retries exceeded: return error
       │
       │ shutdown() / drop()
       ▼
┌──────────────┐
│ Disconnected │ (resources released, background tasks stopped)
└──────────────┘
```

### Token Renewal (Vault only)

```
┌──────────────┐
│ Token Active │ (TTL > renewal_threshold)
└──────┬───────┘
       │ Periodic check (every 5 minutes)
       ▼
┌──────────────┐
│ TTL < 1 hour?│
└──────┬───────┘
       │ Yes
       ▼
┌──────────────┐
│ Renewing     │ (call token::renew_self())
└──────┬───────┘
       │ Success
       ▼
┌──────────────┐
│ Token Active │ (new TTL, return to normal)
└──────────────┘
       │ Failure
       ▼
┌──────────────┐
│ Token Expired│ (must re-authenticate with AppRole)
└──────────────┘
```

### Soft-Delete Recovery (Azure, Vault)

**Azure Key Vault**:
```
┌──────────────┐
│ Secret Active│
└──────┬───────┘
       │ delete_secret()
       ▼
┌──────────────┐
│ Soft-Deleted │ (retention period: 7-90 days)
└──────┬───────┘
       │ recover_deleted_secret()
       ▼
┌──────────────┐
│ Secret Active│ (restored, same name/version)
└──────────────┘
       │ purge_deleted_secret() OR wait 90 days
       ▼
┌──────────────┐
│ Purged       │ (permanent, name reusable)
└──────────────┘
```

**HashiCorp Vault KV v2**:
```
┌──────────────┐
│ Version N    │ (latest version active)
└──────┬───────┘
       │ delete_latest()
       ▼
┌──────────────┐
│ Version N    │ (deletion_time set, can undelete)
│ Soft-Deleted │
└──────┬───────┘
       │ undelete()
       ▼
┌──────────────┐
│ Version N    │ (deletion_time cleared, active)
└──────────────┘
       │ destroy() or delete_metadata()
       ▼
┌──────────────┐
│ Destroyed    │ (permanent, version N unrecoverable)
└──────────────┘
```

---

## Summary

This data model provides:

✅ **Type-Safe Configurations**: Each provider has strongly-typed config with validation  
✅ **Shared Utilities**: Unified retry logic, metrics foundation, common types  
✅ **Provider Implementations**: Clear structure for all 5 storage backends  
✅ **Validation Rules**: Size limits, metadata conversion, configuration constraints  
✅ **State Transitions**: Lifecycle management, token renewal, soft-delete recovery  

**Next Steps**: Generate API contracts (OpenAPI for REST-like operations) and quickstart guide.
