---
title: CredentialManager
tags: [nebula, nebula-credential, docs]
status: published
created: 2025-08-24
---

# CredentialManager API Reference

Central management interface for all credential operations.

## Overview

The `CredentialManager` is the main entry point for credential operations, providing:

- Credential lifecycle management (create, read, update, delete)
- Token retrieval and refresh
- Credential rotation
- Storage backend management
- Caching and performance optimization
- Security and audit logging

## Constructor

### new

Create a new credential manager with default configuration.

```rust
pub fn new() -> Self
```

**Example:**

```rust
let manager = CredentialManager::new();
```

### with_config

Create a credential manager with custom configuration.

```rust
pub fn with_config(config: ManagerConfig) -> Self
```

**Example:**

```rust
let config = ManagerConfig {
    storage_backend: StorageBackend::Vault,
    cache_config: CacheConfig::default(),
    security_config: SecurityConfig::strict(),
};

let manager = CredentialManager::with_config(config);
```

### builder

Create a credential manager using the builder pattern.

```rust
pub fn builder() -> CredentialManagerBuilder
```

**Example:**

```rust
let manager = CredentialManager::builder()
    .storage_backend(StorageBackend::Database {
        url: "postgresql://localhost/credentials",
    })
    .cache_ttl(Duration::from_secs(300))
    .enable_audit_logging()
    .build()
    .await?;
```

## Credential Operations

### create_credential

Create a new credential.

```rust
pub async fn create_credential(
    &self,
    credential_type: &str,
    input: Value,
    context: &UserContext,
) -> Result<CredentialId, CredentialError>
```

**Parameters:**

- `credential_type`: Type identifier (e.g., "api_key", "oauth2")
- `input`: JSON input parameters for the credential type
- `context`: User context for audit and access control

**Returns:** `CredentialId` - Unique identifier for the created credential

**Example:**

```rust
let id = manager.create_credential(
    "api_key",
    json!({
        "api_key": "sk_live_abc123",
        "param_name": "X-API-Key"
    }),
    &UserContext::new("user123", "192.168.1.1")
).await?;
```

### create_credential_idempotent

Create a credential with idempotency support.

```rust
pub async fn create_credential_idempotent(
    &self,
    credential_type: &str,
    input: Value,
    context: &UserContext,
    idempotency_key: Option<&str>,
) -> Result<CredentialId, CredentialError>
```

**Parameters:**

- `idempotency_key`: Optional key for idempotent creation

**Example:**

```rust
let id = manager.create_credential_idempotent(
    "api_key",
    input,
    &context,
    Some("unique-request-123")
).await?;
```

### get_credential

Retrieve credential details.

```rust
pub async fn get_credential(
    &self,
    id: &CredentialId,
) -> Result<Credential, CredentialError>
```

**Parameters:**

- `id`: Credential identifier

**Returns:** `Credential` - Credential details (without sensitive data)

**Example:**

```rust
let credential = manager.get_credential(&id).await?;
println!("Type: {}", credential.credential_type);
println!("Created: {}", credential.created_at);
```

### get_token

Retrieve the authentication token from a credential.

```rust
pub async fn get_token(
    &self,
    id: &CredentialId,
) -> Result<Token, CredentialError>
```

**Parameters:**

- `id`: Credential identifier

**Returns:** `Token` - Authentication token with sensitive data

**Example:**

```rust
let token = manager.get_token(&id).await?;

// Use token in request
client.request()
    .header("Authorization", token.value.expose())
    .send()
    .await?;
```

### get_token_with_context

Retrieve token with additional context for access control.

```rust
pub async fn get_token_with_context(
    &self,
    id: &CredentialId,
    context: &CredentialContext,
) -> Result<Token, CredentialError>
```

**Parameters:**

- `id`: Credential identifier
- `context`: Credential context for access control and audit

**Example:**

```rust
let context = CredentialContext::builder()
    .execution_id(execution_id)
    .workflow_id(workflow_id)
    .user_id(user_id)
    .build();

let token = manager.get_token_with_context(&id, &context).await?;
```

### update_credential

Update credential metadata or non-sensitive properties.

```rust
pub async fn update_credential(
    &self,
    id: &CredentialId,
    updates: CredentialUpdate,
) -> Result<(), CredentialError>
```

**Parameters:**

- `id`: Credential identifier
- `updates`: Update operations to apply

**Example:**

```rust
let updates = CredentialUpdate::builder()
    .description("Updated API key for production")
    .tags(vec!["production", "api"])
    .metadata(json!({
        "last_used": "2024-08-24",
        "owner": "api-team"
    }))
    .build();

manager.update_credential(&id, updates).await?;
```

### delete_credential

Delete a credential.

```rust
pub async fn delete_credential(
    &self,
    id: &CredentialId,
) -> Result<(), CredentialError>
```

**Parameters:**

- `id`: Credential identifier

**Example:**

```rust
manager.delete_credential(&id).await?;
```

## Lifecycle Management

### refresh_credential

Manually refresh an expiring credential.

```rust
pub async fn refresh_credential(
    &self,
    id: &CredentialId,
) -> Result<Token, CredentialError>
```

**Parameters:**

- `id`: Credential identifier

**Returns:** `Token` - New refreshed token

**Example:**

```rust
let new_token = manager.refresh_credential(&id).await?;
```

### refresh_all_expired

Refresh all expired credentials that support refresh.

```rust
pub async fn refresh_all_expired(&self) -> Result<Vec<CredentialId>, CredentialError>
```

**Returns:** `Vec<CredentialId>` - List of refreshed credential IDs

**Example:**

```rust
let refreshed = manager.refresh_all_expired().await?;
println!("Refreshed {} credentials", refreshed.len());
```

### rotate_credential

Rotate a credential (create new, migrate, delete old).

```rust
pub async fn rotate_credential(
    &self,
    id: &CredentialId,
) -> Result<CredentialId, CredentialError>
```

**Parameters:**

- `id`: Current credential identifier

**Returns:** `CredentialId` - New credential identifier

**Example:**

```rust
let new_id = manager.rotate_credential(&old_id).await?;
println!("Rotated {} to {}", old_id, new_id);
```

### schedule_rotation

Schedule credential rotation for a future time.

```rust
pub async fn schedule_rotation(
    &self,
    id: &CredentialId,
    at: DateTime<Utc>,
) -> Result<(), CredentialError>
```

**Parameters:**

- `id`: Credential identifier
- `at`: When to rotate the credential

**Example:**

```rust
manager.schedule_rotation(
    &id,
    Utc::now() + Duration::days(30)
).await?;
```

### set_rotation_policy

Set automatic rotation policy for a credential.

```rust
pub async fn set_rotation_policy(
    &self,
    id: &CredentialId,
    policy: RotationPolicy,
) -> Result<(), CredentialError>
```

**Parameters:**

- `id`: Credential identifier
- `policy`: Rotation policy configuration

**Example:**

```rust
let policy = RotationPolicy::builder()
    .interval(Duration::days(90))
    .warning_period(Duration::days(7))
    .strategy(RotationStrategy::BlueGreen)
    .build();

manager.set_rotation_policy(&id, policy).await?;
```

## Interactive Flows

### start_interactive_flow

Start an interactive authentication flow (e.g., OAuth).

```rust
pub async fn start_interactive_flow(
    &self,
    credential_type: &str,
    input: Value,
    context: &UserContext,
) -> Result<InteractiveFlow, CredentialError>
```

**Returns:** `InteractiveFlow` - Flow information including auth URL

**Example:**

```rust
let flow = manager.start_interactive_flow(
    "oauth2",
    json!({
        "provider": "google",
        "client_id": "...",
        "scopes": ["email", "profile"]
    }),
    &context
).await?;

match flow.interaction_type {
    InteractionType::BrowserAuth { auth_url, .. } => {
        println!("Open browser to: {}", auth_url);
    }
    _ => {}
}
```

### complete_interactive_flow

Complete an interactive flow with callback data.

```rust
pub async fn complete_interactive_flow(
    &self,
    flow_id: &FlowId,
    callback_data: CallbackData,
) -> Result<CredentialId, CredentialError>
```

**Parameters:**

- `flow_id`: Flow identifier from `start_interactive_flow`
- `callback_data`: Data from OAuth callback or user input

**Example:**

```rust
let callback_data = CallbackData {
    code: "auth_code_123",
    state: "csrf_token",
};

let credential_id = manager.complete_interactive_flow(
    &flow.id,
    callback_data
).await?;
```

## Storage Configuration

### set_storage_backend

Change the storage backend.

```rust
pub fn set_storage_backend(&mut self, backend: StorageBackend)
```

**Example:**

```rust
manager.set_storage_backend(StorageBackend::Vault {
    url: "https://vault.example.com",
    token: "vault_token",
});
```

### set_encryption_config

Configure encryption settings.

```rust
pub fn set_encryption_config(&mut self, config: EncryptionConfig)
```

**Example:**

```rust
manager.set_encryption_config(EncryptionConfig {
    algorithm: EncryptionAlgorithm::Aes256Gcm,
    key_rotation_interval: Duration::days(90),
    key_derivation: KeyDerivation::Argon2id,
});
```

## Cache Management

### set_cache_config

Configure caching behavior.

```rust
pub fn set_cache_config(&mut self, config: CacheConfig)
```

**Example:**

```rust
manager.set_cache_config(CacheConfig {
    enabled: true,
    ttl: Duration::from_secs(300),
    max_size: 1000,
    strategy: CacheStrategy::Lru,
});
```

### clear_cache

Clear all cached tokens.

```rust
pub async fn clear_cache(&self) -> Result<(), CacheError>
```

**Example:**

```rust
manager.clear_cache().await?;
```

### clear_cache_for

Clear cache for specific credential.

```rust
pub async fn clear_cache_for(&self, id: &CredentialId) -> Result<(), CacheError>
```

**Example:**

```rust
manager.clear_cache_for(&id).await?;
```

## Monitoring & Health

### health_check

Check health of credential system.

```rust
pub async fn health_check(&self) -> HealthStatus
```

**Returns:** `HealthStatus` - System health information

**Example:**

```rust
let health = manager.health_check().await;
match health {
    HealthStatus::Healthy => println!("System healthy"),
    HealthStatus::Degraded(issues) => println!("Issues: {:?}", issues),
    HealthStatus::Unhealthy(error) => println!("Error: {}", error),
}
```

### get_metrics

Retrieve system metrics.

```rust
pub async fn get_metrics(&self) -> Metrics
```

**Returns:** `Metrics` - Performance and usage metrics

**Example:**

```rust
let metrics = manager.get_metrics().await;
println!("Total credentials: {}", metrics.total_credentials);
println!("Cache hit rate: {:.2}%", metrics.cache_hit_rate * 100.0);
println!("Average refresh time: {:?}", metrics.avg_refresh_time);
```

### get_credential_metrics

Get metrics for specific credential.

```rust
pub async fn get_credential_metrics(
    &self,
    id: &CredentialId,
) -> Result<CredentialMetrics, CredentialError>
```

**Example:**

```rust
let metrics = manager.get_credential_metrics(&id).await?;
println!("Usage count: {}", metrics.usage_count);
println!("Last used: {:?}", metrics.last_used);
println!("Refresh count: {}", metrics.refresh_count);
```

## Audit & Compliance

### get_audit_logs

Retrieve audit logs with filtering.

```rust
pub async fn get_audit_logs(
    &self,
    filter: AuditFilter,
) -> Result<Vec<AuditEntry>, AuditError>
```

**Parameters:**

- `filter`: Filter criteria for audit logs

**Example:**

```rust
let filter = AuditFilter::builder()
    .credential_id(Some(id))
    .action_type(Some(AuditAction::GetToken))
    .date_range(
        Utc::now() - Duration::days(7),
        Utc::now()
    )
    .build();

let logs = manager.get_audit_logs(filter).await?;
for log in logs {
    println!("{}: {} by {}", 
        log.timestamp, 
        log.action, 
        log.user_id
    );
}
```

### export_audit_report

Export audit report for compliance.

```rust
pub async fn export_audit_report(
    &self,
    format: ExportFormat,
    filter: AuditFilter,
) -> Result<Vec<u8>, AuditError>
```

**Parameters:**

- `format`: Export format (JSON, CSV, PDF)
- `filter`: Filter criteria

**Example:**

```rust
let report = manager.export_audit_report(
    ExportFormat::Csv,
    AuditFilter::last_days(30)
).await?;

std::fs::write("audit_report.csv", report)?;
```

## Batch Operations

### batch_create

Create multiple credentials in a batch.

```rust
pub async fn batch_create(
    &self,
    credentials: Vec<CredentialInput>,
    context: &UserContext,
) -> Result<Vec<Result<CredentialId, CredentialError>>, CredentialError>
```

**Example:**

```rust
let inputs = vec![
    CredentialInput::api_key("key1", "X-API-Key-1"),
    CredentialInput::api_key("key2", "X-API-Key-2"),
    CredentialInput::api_key("key3", "X-API-Key-3"),
];

let results = manager.batch_create(inputs, &context).await?;
```

### batch_delete

Delete multiple credentials.

```rust
pub async fn batch_delete(
    &self,
    ids: Vec<CredentialId>,
) -> Result<Vec<Result<(), CredentialError>>, CredentialError>
```

**Example:**

```rust
let results = manager.batch_delete(vec![id1, id2, id3]).await?;
```

## Error Handling

### Error Types

```rust
pub enum CredentialError {
    // Creation errors
    InvalidInput(String),
    DuplicateCredential(CredentialId),
    UnsupportedType(String),
    
    // Retrieval errors
    NotFound(CredentialId),
    AccessDenied(String),
    
    // Token errors
    Expired(CredentialId),
    NoToken,
    InvalidToken,
    
    // Refresh errors
    RefreshNotSupported,
    RefreshFailed(String),
    NoRefreshToken,
    
    // Rotation errors
    RotationFailed(String),
    RotationInProgress(CredentialId),
    
    // Storage errors
    StorageError(StorageError),
    EncryptionError(EncryptionError),
    
    // Interactive flow errors
    InteractionRequired(FlowId),
    FlowExpired(FlowId),
    InvalidCallback,
    
    // System errors
    ServiceUnavailable,
    InternalError(String),
}
```

### Error Recovery

```rust
// Retry pattern
let token = retry::retry(Fixed::from_millis(100).take(3), || async {
    manager.get_token(&id).await
}).await?;

// Fallback pattern
let token = manager.get_token(&primary_id).await
    .or_else(|e| {
        log::warn!("Primary failed: {}, trying fallback", e);
        manager.get_token(&fallback_id)
    }).await?;

// Circuit breaker pattern
let breaker = CircuitBreaker::new();
let token = breaker.call(|| async {
    manager.get_token(&id).await
}).await?;
```

## Configuration Types

### ManagerConfig

```rust
pub struct ManagerConfig {
    pub storage_backend: StorageBackend,
    pub cache_config: CacheConfig,
    pub security_config: SecurityConfig,
    pub rotation_config: RotationConfig,
    pub audit_config: AuditConfig,
    pub performance_config: PerformanceConfig,
}
```

### StorageBackend

```rust
pub enum StorageBackend {
    Memory,
    File { path: PathBuf },
    Database { url: String },
    Vault { url: String, token: String },
    AwsSecretsManager { region: String },
    AzureKeyVault { url: String },
    Custom(Box<dyn CredentialStorage>),
}
```

### CacheConfig

```rust
pub struct CacheConfig {
    pub enabled: bool,
    pub ttl: Duration,
    pub max_size: usize,
    pub strategy: CacheStrategy,
    pub refresh_ahead: bool,
}
```

## Examples

### Complete Example

```rust
use nebula_credential::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create manager
    let manager = CredentialManager::builder()
        .storage_backend(StorageBackend::Vault {
            url: "https://vault.example.com".to_string(),
            token: std::env::var("VAULT_TOKEN")?,
        })
        .cache_ttl(Duration::from_secs(300))
        .enable_audit_logging()
        .build()
        .await?;
    
    // Create credential
    let id = manager.create_credential(
        "oauth2",
        json!({
            "provider": "google",
            "client_id": "...",
            "client_secret": "...",
            "scopes": ["email", "profile"]
        }),
        &UserContext::system()
    ).await?;
    
    // Set rotation policy
    manager.set_rotation_policy(
        &id,
        RotationPolicy::periodic(Duration::days(30))
    ).await?;
    
    // Use credential
    let token = manager.get_token(&id).await?;
    
    // Make authenticated request
    let client = reqwest::Client::new();
    let response = client
        .get("https://api.example.com/data")
        .header("Authorization", format!("Bearer {}", token.value.expose()))
        .send()
        .await?;
    
    Ok(())
}
```

## Related

- [Credential Trait](https://claude.ai/chat/CredentialTrait.md)
- [Credential Types](https://claude.ai/chat/CredentialTypes.md)
- [Storage Backends](https://claude.ai/chat/StorageBackends.md)
- [Error Types](https://claude.ai/chat/ErrorTypes.md)
