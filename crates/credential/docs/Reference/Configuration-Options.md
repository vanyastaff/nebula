---
title: Configuration Options
tags: [reference, configuration, config, builder-pattern, p2, priority-2]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: intermediate
---

# Configuration Options

> [!NOTE] Complete Configuration Reference
> This document covers all configuration types and builder patterns in nebula-credential. All configurations use builder pattern with compile-time validation.

## Table of Contents

1. [CredentialManager Configuration](#credentialmanager-configuration)
2. [Storage Provider Configurations](#storage-provider-configurations)
3. [Encryption Configuration](#encryption-configuration)
4. [Cache Configuration](#cache-configuration)
5. [Rotation Configuration](#rotation-configuration)
6. [Audit Configuration](#audit-configuration)
7. [Protocol-Specific Configurations](#protocol-specific-configurations)

---

## CredentialManager Configuration

### CredentialManagerConfig

Main configuration for the credential management system.

```rust
pub struct CredentialManagerConfig {
    pub storage: Arc<dyn StorageProvider>,
    pub encryption: EncryptionConfig,
    pub cache: Option<CacheConfig>,
    pub rotation: Option<RotationConfig>,
    pub audit: Option<AuditConfig>,
}
```

**Builder Example**:
```rust
use nebula_credential::prelude::*;

let config = CredentialManagerConfig::builder()
    .storage(LocalStorage::new("./secrets"))
    .encryption(EncryptionConfig::default())
    .cache(CacheConfig {
        enabled: true,
        ttl: Duration::from_secs(300),
        max_size: 1_000_000_000,
        eviction: EvictionPolicy::LRU,
    })
    .rotation(RotationConfig {
        enabled: true,
        check_interval: Duration::from_secs(3600),
    })
    .audit(AuditConfig {
        enabled: true,
        logger: Arc::new(FileAuditLogger::new("./audit.log")),
    })
    .build()?;

let manager = CredentialManager::new(config);
```

---

## Storage Provider Configurations

### LocalStorageConfig

Configuration for local encrypted storage (SQLite).

```rust
pub struct LocalStorageConfig {
    /// Path to storage directory
    pub path: PathBuf,
    
    /// Enable WAL mode for better concurrency
    pub wal_mode: bool,
    
    /// Sync mode (Normal, Full, Off)
    pub sync_mode: SyncMode,
    
    /// Maximum connections in pool
    pub max_connections: u32,
}

impl Default for LocalStorageConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("./secrets"),
            wal_mode: true,
            sync_mode: SyncMode::Normal,
            max_connections: 10,
        }
    }
}
```

**Example**:
```rust
let storage = LocalStorage::builder()
    .path("./my-secrets")
    .wal_mode(true)
    .max_connections(20)
    .build()?;
```

---

### AwsSecretsManagerConfig

Configuration for AWS Secrets Manager.

```rust
pub struct AwsSecretsManagerConfig {
    /// AWS region
    pub region: String,
    
    /// KMS key ID for encryption (optional)
    pub kms_key_id: Option<String>,
    
    /// Custom endpoint (for testing)
    pub endpoint: Option<String>,
    
    /// Retry configuration
    pub retry: RetryConfig,
    
    /// Timeout for operations
    pub timeout: Duration,
}

impl AwsSecretsManagerConfig {
    pub fn builder() -> AwsSecretsManagerConfigBuilder {
        AwsSecretsManagerConfigBuilder::new()
    }
}
```

**Example**:
```rust
let storage = AwsSecretsManager::builder()
    .region("us-east-1")
    .kms_key_id("arn:aws:kms:us-east-1:123456789:key/abc")
    .timeout(Duration::from_secs(30))
    .retry(RetryConfig {
        max_attempts: 3,
        initial_backoff: Duration::from_millis(100),
        max_backoff: Duration::from_secs(10),
    })
    .build()?;
```

---

### VaultStorageConfig

Configuration for HashiCorp Vault.

```rust
pub struct VaultStorageConfig {
    /// Vault server URL
    pub url: String,
    
    /// Authentication method
    pub auth: VaultAuth,
    
    /// KV engine path (default: "secret")
    pub kv_path: String,
    
    /// KV engine version (v1 or v2)
    pub kv_version: KvVersion,
    
    /// Use Transit engine for encryption
    pub use_transit: bool,
    
    /// Transit engine path
    pub transit_path: String,
    
    /// Connection pool size
    pub max_connections: usize,
}

pub enum VaultAuth {
    Token(String),
    AppRole { role_id: String, secret_id: String },
    Kubernetes { role: String, jwt: String },
    Aws { role: String },
}

pub enum KvVersion {
    V1,
    V2,
}
```

**Example**:
```rust
let storage = VaultStorage::builder()
    .url("https://vault.example.com")
    .auth(VaultAuth::Token("s.abc123".into()))
    .kv_path("secret")
    .kv_version(KvVersion::V2)
    .use_transit(true)
    .transit_path("transit")
    .build()?;
```

---

### AzureKeyVaultConfig

Configuration for Azure Key Vault.

```rust
pub struct AzureKeyVaultConfig {
    /// Key Vault name
    pub vault_name: String,
    
    /// Authentication credential
    pub credential: AzureCredential,
    
    /// Timeout for operations
    pub timeout: Duration,
}

pub enum AzureCredential {
    ManagedIdentity,
    ServicePrincipal {
        tenant_id: String,
        client_id: String,
        client_secret: String,
    },
}
```

**Example**:
```rust
let storage = AzureKeyVault::builder()
    .vault_name("my-vault")
    .credential(AzureCredential::ManagedIdentity)
    .timeout(Duration::from_secs(30))
    .build()?;
```

---

### KubernetesSecretsConfig

Configuration for Kubernetes Secrets.

```rust
pub struct KubernetesSecretsConfig {
    /// Namespace for secrets
    pub namespace: String,
    
    /// Label selector for filtering secrets
    pub label_selector: Option<String>,
    
    /// Use in-cluster config or kubeconfig
    pub config: K8sConfig,
}

pub enum K8sConfig {
    InCluster,
    Kubeconfig(PathBuf),
}
```

**Example**:
```rust
let storage = KubernetesSecrets::builder()
    .namespace("default")
    .label_selector("app=nebula")
    .config(K8sConfig::InCluster)
    .build()?;
```

---

## Encryption Configuration

### EncryptionConfig

Cryptographic settings.

```rust
pub struct EncryptionConfig {
    /// Encryption algorithm
    pub algorithm: EncryptionAlgorithm,
    
    /// Key derivation function
    pub key_derivation: KeyDerivation,
    
    /// Nonce generation strategy
    pub nonce_strategy: NonceStrategy,
}

pub enum EncryptionAlgorithm {
    Aes256Gcm,
    ChaCha20Poly1305,
}

pub enum KeyDerivation {
    Argon2id {
        memory_cost: u32,  // KiB
        time_cost: u32,
        parallelism: u32,
    },
    Pbkdf2 {
        iterations: u32,
    },
}

pub enum NonceStrategy {
    /// Monotonic counter + random + timestamp
    Hybrid,
    /// Pure random (requires strong RNG)
    Random,
    /// Monotonic counter only (requires persistent state)
    Counter,
}
```

**Default (Secure)**:
```rust
impl Default for EncryptionConfig {
    fn default() -> Self {
        Self {
            algorithm: EncryptionAlgorithm::Aes256Gcm,
            key_derivation: KeyDerivation::Argon2id {
                memory_cost: 19456, // 19 MiB
                time_cost: 2,
                parallelism: 1,
            },
            nonce_strategy: NonceStrategy::Hybrid,
        }
    }
}
```

**Example**:
```rust
let encryption = EncryptionConfig {
    algorithm: EncryptionAlgorithm::Aes256Gcm,
    key_derivation: KeyDerivation::Argon2id {
        memory_cost: 65536, // 64 MiB (more secure, slower)
        time_cost: 3,
        parallelism: 2,
    },
    nonce_strategy: NonceStrategy::Hybrid,
};
```

---

## Cache Configuration

### CacheConfig

In-memory cache settings.

```rust
pub struct CacheConfig {
    /// Enable caching
    pub enabled: bool,
    
    /// Time-to-live for cached credentials
    pub ttl: Duration,
    
    /// Maximum cache size in bytes
    pub max_size: usize,
    
    /// Eviction policy
    pub eviction: EvictionPolicy,
    
    /// Write-through or write-behind
    pub write_strategy: WriteStrategy,
}

pub enum EvictionPolicy {
    LRU,  // Least Recently Used
    LFU,  // Least Frequently Used
    FIFO, // First In First Out
}

pub enum WriteStrategy {
    /// Write to cache and storage atomically
    WriteThrough,
    
    /// Write to cache immediately, storage asynchronously
    WriteBehind { flush_interval: Duration },
}
```

**Default**:
```rust
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ttl: Duration::from_secs(300), // 5 minutes
            max_size: 1_000_000_000, // 1 GB
            eviction: EvictionPolicy::LRU,
            write_strategy: WriteStrategy::WriteThrough,
        }
    }
}
```

**Example (High Performance)**:
```rust
let cache = CacheConfig {
    enabled: true,
    ttl: Duration::from_secs(600), // 10 minutes
    max_size: 5_000_000_000, // 5 GB
    eviction: EvictionPolicy::LRU,
    write_strategy: WriteStrategy::WriteBehind {
        flush_interval: Duration::from_secs(5),
    },
};
```

---

## Rotation Configuration

### RotationConfig

Automatic credential rotation settings.

```rust
pub struct RotationConfig {
    /// Enable automatic rotation
    pub enabled: bool,
    
    /// How often to check for credentials needing rotation
    pub check_interval: Duration,
    
    /// Grace period for blue-green rotation
    pub grace_period: Duration,
    
    /// Maximum concurrent rotations
    pub max_concurrent: usize,
    
    /// Retry configuration
    pub retry: RetryConfig,
}

impl Default for RotationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            check_interval: Duration::from_secs(3600), // 1 hour
            grace_period: Duration::from_secs(86400), // 24 hours
            max_concurrent: 5,
            retry: RetryConfig::default(),
        }
    }
}
```

**Example**:
```rust
let rotation = RotationConfig {
    enabled: true,
    check_interval: Duration::from_secs(1800), // 30 minutes
    grace_period: Duration::from_secs(43200), // 12 hours
    max_concurrent: 10,
    retry: RetryConfig {
        max_attempts: 5,
        initial_backoff: Duration::from_millis(100),
        max_backoff: Duration::from_secs(60),
    },
};
```

### RotationPolicy

Policies for when to rotate credentials.

```rust
pub enum RotationPolicy {
    /// Rotate every N duration
    Periodic(Duration),
    
    /// Rotate X duration before expiry
    BeforeExpiry(Duration),
    
    /// Rotate on cron schedule
    Scheduled(String),
    
    /// Manual rotation only
    Manual,
}
```

**Examples**:
```rust
// Rotate every 90 days
let policy = RotationPolicy::Periodic(Duration::from_days(90));

// Rotate 5 minutes before expiry
let policy = RotationPolicy::BeforeExpiry(Duration::from_secs(300));

// Rotate every Sunday at 2 AM
let policy = RotationPolicy::Scheduled("0 2 * * SUN".into());

// Only rotate when explicitly triggered
let policy = RotationPolicy::Manual;
```

---

## Audit Configuration

### AuditConfig

Audit logging configuration.

```rust
pub struct AuditConfig {
    /// Enable audit logging
    pub enabled: bool,
    
    /// Audit logger implementation
    pub logger: Arc<dyn AuditLogger>,
    
    /// What events to log
    pub event_filter: EventFilter,
    
    /// Include sensitive data in logs (DANGEROUS)
    pub log_sensitive: bool,
}

pub struct EventFilter {
    /// Log credential access events
    pub access: bool,
    
    /// Log credential creation events
    pub creation: bool,
    
    /// Log rotation events
    pub rotation: bool,
    
    /// Log security violations
    pub violations: bool,
    
    /// Log authentication attempts
    pub authentication: bool,
}

impl Default for EventFilter {
    fn default() -> Self {
        Self {
            access: true,
            creation: true,
            rotation: true,
            violations: true,
            authentication: true,
        }
    }
}
```

**Example (Compliance Mode)**:
```rust
let audit = AuditConfig {
    enabled: true,
    logger: Arc::new(StructuredAuditLogger::new(
        "./audit.jsonl",
        RotationPolicy::Daily,
    )),
    event_filter: EventFilter::default(),
    log_sensitive: false, // NEVER set to true in production!
};
```

---

## Protocol-Specific Configurations

### OAuth2Config

OAuth 2.0 flow configuration.

```rust
pub struct OAuth2Config {
    pub client_id: String,
    pub client_secret: SecretString,
    pub auth_url: Url,
    pub token_url: Url,
    pub redirect_url: Url,
    pub scopes: Vec<String>,
    
    /// Enable PKCE (recommended)
    pub pkce: bool,
    
    /// Code verifier length (43-128 chars)
    pub pkce_verifier_length: usize,
    
    /// Grant type
    pub grant_type: OAuth2GrantType,
}

pub enum OAuth2GrantType {
    AuthorizationCode,
    ClientCredentials,
    DeviceCode,
    RefreshToken,
}
```

**Example**:
```rust
let config = OAuth2Config::builder()
    .client_id("my_client_id")
    .client_secret(SecretString::new("secret"))
    .auth_url("https://provider.com/oauth/authorize".parse()?)
    .token_url("https://provider.com/oauth/token".parse()?)
    .redirect_url("http://localhost:8080/callback".parse()?)
    .scopes(vec!["read".into(), "write".into()])
    .enable_pkce()
    .pkce_verifier_length(64)
    .grant_type(OAuth2GrantType::AuthorizationCode)
    .build()?;
```

---

### SAMLConfig

SAML 2.0 configuration.

```rust
pub struct SAMLConfig {
    /// Service Provider entity ID
    pub sp_entity_id: String,
    
    /// Identity Provider entity ID
    pub idp_entity_id: String,
    
    /// SSO URL for IdP
    pub sso_url: Url,
    
    /// Assertion Consumer Service URL
    pub acs_url: Url,
    
    /// Certificate for signature validation
    pub idp_certificate: X509Certificate,
    
    /// SP private key for signing requests
    pub sp_private_key: Option<PrivateKey>,
    
    /// Sign authentication requests
    pub sign_requests: bool,
    
    /// Require signed assertions
    pub require_signed_assertions: bool,
}
```

---

### LDAPConfig

LDAP/Active Directory configuration.

```rust
pub struct LDAPConfig {
    /// LDAP server URL
    pub url: String,
    
    /// Bind DN for authentication
    pub bind_dn: String,
    
    /// Bind password
    pub bind_password: SecretString,
    
    /// Base DN for searches
    pub base_dn: String,
    
    /// User search filter
    pub user_filter: String,
    
    /// TLS configuration
    pub tls: LDAPTlsConfig,
    
    /// Connection pool size
    pub pool_size: usize,
    
    /// Connection timeout
    pub timeout: Duration,
}

pub enum LDAPTlsConfig {
    None,
    StartTLS,
    LDAPS { verify_cert: bool },
}
```

**Example**:
```rust
let config = LDAPConfig {
    url: "ldap://ldap.example.com:389".into(),
    bind_dn: "cn=admin,dc=example,dc=com".into(),
    bind_password: SecretString::new("password"),
    base_dn: "ou=users,dc=example,dc=com".into(),
    user_filter: "(&(objectClass=person)(uid={}))".into(),
    tls: LDAPTlsConfig::StartTLS,
    pool_size: 10,
    timeout: Duration::from_secs(30),
};
```

---

### JWTConfig

JWT validation configuration.

```rust
pub struct JWTConfig {
    /// Signing algorithm
    pub algorithm: JWTAlgorithm,
    
    /// Secret key (for HS256/HS384/HS512)
    pub secret: Option<SecretString>,
    
    /// Public key (for RS256/RS384/RS512/ES256/ES384)
    pub public_key: Option<Vec<u8>>,
    
    /// Required claims
    pub required_claims: Vec<String>,
    
    /// Validate expiration
    pub validate_exp: bool,
    
    /// Validate not-before
    pub validate_nbf: bool,
    
    /// Allowed issuers
    pub allowed_issuers: Vec<String>,
    
    /// Allowed audiences
    pub allowed_audiences: Vec<String>,
    
    /// Clock skew tolerance
    pub clock_skew: Duration,
}

pub enum JWTAlgorithm {
    HS256, HS384, HS512,
    RS256, RS384, RS512,
    ES256, ES384,
}
```

---

## Retry Configuration

### RetryConfig

Used across multiple components.

```rust
pub struct RetryConfig {
    /// Maximum retry attempts
    pub max_attempts: usize,
    
    /// Initial backoff duration
    pub initial_backoff: Duration,
    
    /// Maximum backoff duration
    pub max_backoff: Duration,
    
    /// Backoff multiplier
    pub multiplier: f64,
    
    /// Jitter to prevent thundering herd
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(30),
            multiplier: 2.0,
            jitter: true,
        }
    }
}
```

---

## Environment-Specific Configurations

### Development

```rust
let dev_config = CredentialManagerConfig::builder()
    .storage(LocalStorage::new("./dev-secrets"))
    .encryption(EncryptionConfig::default())
    .cache(CacheConfig {
        enabled: true,
        ttl: Duration::from_secs(60), // Short TTL for testing
        max_size: 100_000_000, // 100 MB
        eviction: EvictionPolicy::LRU,
        write_strategy: WriteStrategy::WriteThrough,
    })
    .rotation(RotationConfig {
        enabled: false, // Disable auto-rotation in dev
        ..Default::default()
    })
    .audit(AuditConfig {
        enabled: true,
        logger: Arc::new(ConsoleAuditLogger::new()),
        event_filter: EventFilter::default(),
        log_sensitive: false,
    })
    .build()?;
```

### Production

```rust
let prod_config = CredentialManagerConfig::builder()
    .storage(AwsSecretsManager::builder()
        .region("us-east-1")
        .kms_key_id("arn:aws:kms:...")
        .build()?)
    .encryption(EncryptionConfig {
        algorithm: EncryptionAlgorithm::Aes256Gcm,
        key_derivation: KeyDerivation::Argon2id {
            memory_cost: 65536, // 64 MiB
            time_cost: 3,
            parallelism: 2,
        },
        nonce_strategy: NonceStrategy::Hybrid,
    })
    .cache(CacheConfig {
        enabled: true,
        ttl: Duration::from_secs(300),
        max_size: 5_000_000_000, // 5 GB
        eviction: EvictionPolicy::LRU,
        write_strategy: WriteStrategy::WriteThrough,
    })
    .rotation(RotationConfig {
        enabled: true,
        check_interval: Duration::from_secs(3600),
        grace_period: Duration::from_secs(86400),
        max_concurrent: 10,
        retry: RetryConfig::default(),
    })
    .audit(AuditConfig {
        enabled: true,
        logger: Arc::new(CloudWatchAuditLogger::new("nebula-audit")),
        event_filter: EventFilter::default(),
        log_sensitive: false, // NEVER true in production
    })
    .build()?;
```

---

## See Also

- [[Reference/API-Reference|API Reference]]
- [[Architecture|System Architecture]]
- [[How-To/Configure-Caching|Caching Guide]]
- [[Integrations/AWS-Secrets-Manager|AWS Configuration]]
- [[Integrations/HashiCorp-Vault|Vault Configuration]]
- [[Advanced/Performance-Tuning|Performance Tuning]]
