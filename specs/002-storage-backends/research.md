# Phase 0 Research: Storage Backend Technologies

**Feature**: Production-Ready Storage Backends (002-storage-backends)  
**Date**: 2026-02-03  
**Status**: Complete  

This document consolidates research findings from parallel investigation of all storage backend technologies for Phase 2 implementation.

---

## Table of Contents

1. [AWS Secrets Manager](#1-aws-secrets-manager)
2. [Azure Key Vault](#2-azure-key-vault)
3. [HashiCorp Vault](#3-hashicorp-vault)
4. [Kubernetes Secrets](#4-kubernetes-secrets)
5. [Local Encrypted Storage](#5-local-encrypted-storage)
6. [Cross-Cutting Concerns](#6-cross-cutting-concerns)

---

## 1. AWS Secrets Manager

### SDK and Client Initialization

**Decision**: Use `aws-sdk-secretsmanager` with automatic credential chain resolution.

**Rationale**: Official AWS SDK for Rust provides robust credential discovery (environment → IAM instance profile → ECS task role), automatic retries, and built-in region selection. Mature ecosystem with strong type safety.

**Implementation Pattern**:
```rust
use aws_config::BehaviorVersion;
use aws_sdk_secretsmanager::{Client, Config};

// Automatic credential chain + region resolution
let config = aws_config::defaults(BehaviorVersion::latest()).load().await;
let client = Client::new(&config);
```

**Alternatives Considered**:
- Manual credential configuration: Rejected because automatic chain is more secure (no hardcoded credentials) and handles instance profiles seamlessly

### Retry Strategy

**Decision**: Use AWS SDK built-in retry with exponential backoff (3 attempts, starting at 100ms), supplement with custom retry for application-specific errors.

**Rationale**: AWS SDK includes automatic retry for transient errors (503, network timeouts) with full jitter to prevent thundering herd. Built-in behavior handles 95% of cases correctly.

**Parameters**:
- Base delay: 100ms
- Max attempts: 3 (AWS SDK default)
- Backoff multiplier: 2x
- Jitter: Full jitter (random 0 to calculated delay)

**Custom Retry Wrapper** (for non-retryable errors that should retry):
```rust
async fn retry_with_backoff<F, T>(
    operation: F,
    max_retries: u32,
) -> Result<T, StorageError>
where
    F: Fn() -> Pin<Box<dyn Future<Output = Result<T, SdkError<E>>>>>,
{
    let mut attempt = 0;
    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) if attempt < max_retries && is_retryable(&e) => {
                let delay = 100 * 2u64.pow(attempt);
                tokio::time::sleep(Duration::from_millis(delay)).await;
                attempt += 1;
            }
            Err(e) => return Err(e.into()),
        }
    }
}
```

### Error Handling

**Decision**: Map AWS SDK errors to `StorageError` with actionable context including IAM permission requirements.

**Error Mapping Table**:
| AWS Error | HTTP Code | Application Error | Context |
|-----------|-----------|-------------------|---------|
| `ResourceNotFoundException` | 404 | `StorageError::NotFound` | Secret does not exist |
| `AccessDeniedException` | 403 | `StorageError::PermissionDenied` | Missing IAM permission (include which one) |
| `DecryptionFailure` | 400 | `StorageError::DecryptionFailed` | KMS key issue or corrupted data |
| `InvalidRequestException` | 400 | `StorageError::InvalidRequest` | Malformed request parameters |
| Network errors | - | `StorageError::Timeout` | Connection timeout or DNS failure |

**Actionable Error Example**:
```rust
Err(SdkError::ServiceError { err, .. }) if err.is_access_denied_exception() => {
    StorageError::PermissionDenied {
        resource: "AWS Secrets Manager".into(),
        required_permission: "secretsmanager:GetSecretValue OR secretsmanager:CreateSecret".into(),
        fix: "Add IAM policy: { \"Effect\": \"Allow\", \"Action\": \"secretsmanager:GetSecretValue\", \"Resource\": \"*\" }".into(),
    }
}
```

### Security and IAM Permissions

**Decision**: Document minimum IAM permissions for read-only and read-write access.

**Read-Only Permissions**:
```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Action": [
      "secretsmanager:GetSecretValue",
      "secretsmanager:DescribeSecret"
    ],
    "Resource": "arn:aws:secretsmanager:region:account-id:secret:nebula/*"
  }]
}
```

**Read-Write Permissions** (add to above):
```json
{
  "Action": [
    "secretsmanager:CreateSecret",
    "secretsmanager:UpdateSecret",
    "secretsmanager:DeleteSecret",
    "secretsmanager:TagResource",
    "secretsmanager:UntagResource"
  ],
  "Resource": "arn:aws:secretsmanager:region:account-id:secret:nebula/*"
}
```

**KMS Encryption**: AWS Secrets Manager uses AES-256-GCM with envelope encryption (data encrypted with data key, data key encrypted with KMS CMK).

### Performance

**Typical Latency** (p50/p95/p99):
- GetSecretValue: 50ms / 120ms / 250ms
- CreateSecret: 80ms / 180ms / 400ms
- UpdateSecret: 80ms / 180ms / 400ms

**Rate Limits**:
- 5,000 GetSecretValue requests/second per account
- Adjustable via AWS Service Quotas

**Optimization**: Share `Client` instance across all operations (connection pooling handled automatically by SDK).

### Size Limits and Metadata

**Decision**: Validate credential size before submission (64KB hard limit).

**Size Limit**: 64KB (65,536 bytes) per secret value.

**Handling Strategy**:
```rust
const AWS_SECRET_MAX_SIZE: usize = 64 * 1024; // 64KB

if credential_data.len() > AWS_SECRET_MAX_SIZE {
    return Err(StorageError::CredentialTooLarge {
        size: credential_data.len(),
        limit: AWS_SECRET_MAX_SIZE,
        provider: "AWS Secrets Manager".into(),
    });
}
```

**Metadata Conversion**:
- Convert `CredentialMetadata` tags to AWS Tags (max 50 tags per secret)
- Tag keys: 1-128 characters
- Tag values: 0-256 characters
- Format: `{ "Key": "environment", "Value": "production" }`

---

## 2. Azure Key Vault

### SDK and Authentication

**Decision**: Use `azure_security_keyvault_secrets` with Managed Identity for production, `DeveloperToolsCredential` for local development.

**Rationale**: Azure SDK for Rust provides seamless Managed Identity authentication (no credentials in code), automatic token refresh, and RBAC integration. Industry-standard approach for Azure workloads.

**Authentication Pattern** (Production):
```rust
use azure_identity::ManagedIdentityCredential;
use azure_security_keyvault_secrets::SecretClient;

let credential = ManagedIdentityCredential::new(None)?;
let client = SecretClient::new(vault_url, credential, None)?;
```

**Local Development**:
```rust
use azure_identity::DeveloperToolsCredential;

// Uses `az login` credentials
let credential = DeveloperToolsCredential::new(None)?;
```

**Alternatives Considered**:
- Service Principal with client secret: Rejected for production (credentials in environment variables), but acceptable for CI/CD
- Certificate-based authentication: Higher security but added complexity

### Retry Strategy

**Decision**: Use Azure SDK built-in retry for 429/503, add custom retry with exponential backoff for network errors.

**Azure SDK Behavior**:
- Automatically retries HTTP 429 (Too Many Requests)
- Automatically retries HTTP 503 (Service Unavailable)
- Respects `Retry-After` header when present
- Default: 3 attempts with exponential backoff

**Custom Retry Configuration**:
- Initial delay: 100ms
- Max retries: 5
- Backoff multiplier: 2x
- Max delay cap: 30 seconds
- Jitter: ±25% to prevent thundering herd

### RBAC Roles

**Decision**: Document required RBAC roles for read-only and full access scenarios.

**Key Vault Secrets User** (Read-Only):
- `secrets/get` - Retrieve secret values
- `secrets/list` - List secrets in vault
- Use case: Applications that only need to read credentials

**Key Vault Secrets Officer** (Full Management):
- All Secrets User permissions, plus:
- `secrets/set` - Create or update secrets
- `secrets/delete` - Delete secrets
- `secrets/recover` - Recover soft-deleted secrets
- `secrets/purge` - Permanently delete secrets

**Grant Command**:
```bash
az role assignment create \
  --role "Key Vault Secrets User" \
  --assignee <managed-identity-object-id> \
  --scope /subscriptions/<sub-id>/resourceGroups/<rg>/providers/Microsoft.KeyVault/vaults/<vault-name>
```

### Error Handling

**Decision**: Map Azure SDK errors to `StorageError` with RBAC-specific guidance.

**Error Mapping**:
| Azure Error | HTTP Code | Application Error | Fix |
|-------------|-----------|-------------------|-----|
| `Forbidden` | 403 | `StorageError::PermissionDenied` | Grant "Key Vault Secrets User" role |
| `NotFound` | 404 | `StorageError::NotFound` | Secret does not exist |
| `Unauthorized` | 401 | `StorageError::AuthenticationFailed` | Check Managed Identity or token expiration |
| `TooManyRequests` | 429 | Automatic retry | SDK handles with backoff |

**Actionable Error Pattern**:
```rust
ErrorKind::HttpResponse { status: StatusCode::Forbidden, .. } => {
    StorageError::PermissionDenied {
        resource: "Azure Key Vault".into(),
        required_permission: "Key Vault Secrets User (for read) or Key Vault Secrets Officer (for write)".into(),
        fix: "az role assignment create --role \"Key Vault Secrets User\" --assignee <object-id> --scope <vault-resource-id>".into(),
    }
}
```

### Performance

**Typical Latency** (p50/p95/p99):
- Get Secret: 20ms / 80ms / 200ms
- Set Secret: 50ms / 120ms / 250ms
- List Secrets: 100ms / 180ms / 400ms

**Rate Limits**:
- 2,000 requests per 10 seconds per vault
- Throttling returns HTTP 429
- Use client-side rate limiting (Semaphore) to stay within limits

**Connection Pooling**:
- `SecretClient` is thread-safe (uses `Arc` internally)
- Reuse single client instance across application
- HTTP connection pooling handled automatically by `reqwest`

### Soft-Delete Behavior

**Decision**: Document soft-delete recovery period and purge protection requirements.

**Soft-Delete** (enabled by default since Azure API version 2020-04-01):
- Retention period: 7-90 days (vault-level setting, default 90 days)
- Deleted secrets can be recovered during retention
- Secret name unavailable for new secrets until purged or retention expires

**Purge Protection**:
- When enabled: Deleted secrets CANNOT be purged during retention
- Irreversible once enabled
- Required for compliance scenarios (PCI-DSS, HIPAA)

**Operations**:
```rust
// Soft-delete (enter deleted state, recoverable)
client.delete_secret(secret_name).await?;

// Recover soft-deleted secret
client.recover_deleted_secret(secret_name).await?;

// Purge (permanent, only if purge protection disabled)
client.purge_deleted_secret(secret_name).await?;
```

### Metadata and Tags

**Decision**: Convert `CredentialMetadata` to Azure Key Vault tags for organization.

**Tag Structure**:
- Tags: Key-value pairs (max 15 tags per secret)
- Keys: No length limit specified
- Values: No length limit specified
- Use for filtering and organization

**Conversion Pattern**:
```rust
let mut tags = HashMap::new();
tags.insert("environment".to_string(), "production".to_string());
tags.insert("application".to_string(), "nebula-credential".to_string());
tags.insert("owner".to_string(), metadata.owner.clone());

let options = SetSecretOptions { tags: Some(tags), ..Default::default() };
client.set_secret(name, value, Some(options)).await?;
```

---

## 3. HashiCorp Vault

### SDK and Client Initialization

**Decision**: Use `vaultrs` v0.7+ with token-based authentication for production (AppRole), direct token for development.

**Rationale**: vaultrs is the most mature Rust client for Vault, supporting KV v2 engine with versioning, automatic token renewal patterns, and comprehensive API coverage.

**Client Pattern** (AppRole):
```rust
use vaultrs::client::{VaultClient, VaultClientSettingsBuilder};
use vaultrs::auth::approle;

// Initialize unauthenticated client
let client = VaultClient::new(
    VaultClientSettingsBuilder::default()
        .address("https://vault.example.com:8200")
        .build()?
)?;

// Authenticate with AppRole
let auth_info = approle::login(&client, "approle", role_id, secret_id).await?;
let client = client.with_token(&auth_info.client_token);
```

**Development Pattern**:
```rust
let client = VaultClient::new(
    VaultClientSettingsBuilder::default()
        .address("http://127.0.0.1:8200")
        .token("dev-root-token")
        .build()?
)?;
```

### KV v2 Versioning

**Decision**: Preserve version metadata when storing/retrieving credentials, use `kv2::read()` for latest version.

**Versioning Behavior**:
- Every write creates new version (immutable versions)
- Version numbers start at 1, increment monotonically
- Configurable max versions (default: unlimited)
- Oldest version auto-deleted when limit exceeded

**Path Structure**:
```
API Access:
  vault kv put secret/nebula/credentials/db username=admin password=secret
  ↓ Translates to:
  PUT /v1/secret/data/nebula/credentials/db
  
Metadata Path:
  GET /v1/secret/metadata/nebula/credentials/db
```

**Operations**:
```rust
use vaultrs::kv2;

// Store (creates version N+1)
kv2::set(&client, "nebula", "credentials/db", &data).await?;

// Retrieve latest version
let secret = kv2::read(&client, "nebula", "credentials/db").await?;

// Retrieve specific version
let secret = kv2::read_version(&client, "nebula", "credentials/db", 3).await?;

// Delete (soft-delete, can undelete)
kv2::delete_latest(&client, "nebula", "credentials/db").await?;

// Destroy metadata (permanent)
kv2::delete_metadata(&client, "nebula", "credentials/db").await?;
```

### Token Renewal

**Decision**: Implement automatic token renewal when TTL drops below 1 hour using background task.

**Rationale**: Vault service tokens have TTL (typically 1-4 hours), must be renewed before expiration to prevent authentication failures. Proactive renewal (TTL < 1 hour threshold) ensures continuous operation.

**Renewal Pattern**:
```rust
use vaultrs::auth::token;
use tokio::time::{interval, Duration};

async fn token_renewal_task(client: VaultClient) {
    let mut check_interval = interval(Duration::from_secs(300)); // Check every 5 min
    let renewal_threshold = Duration::from_secs(3600); // Renew when TTL < 1 hour
    
    loop {
        check_interval.tick().await;
        
        match token::lookup_self(&client).await {
            Ok(token_info) => {
                let ttl = token_info.ttl.unwrap_or(0);
                if ttl < renewal_threshold.as_secs() as i64 {
                    match token::renew_self(&client, Some(3600)).await {
                        Ok(_) => tracing::info!("Token renewed, new TTL: 3600s"),
                        Err(e) => tracing::error!("Token renewal failed: {}", e),
                    }
                }
            }
            Err(e) => tracing::error!("Token lookup failed: {}", e),
        }
    }
}
```

### Retry Strategy

**Decision**: Use exponential backoff with jitter for network errors, 5 max retries starting at 100ms.

**Retryable Conditions**:
- Network errors (connection refused, timeout)
- HTTP 503 (Service Unavailable)
- HTTP 429 (Rate Limited - rare in Vault OSS)

**Non-Retryable**:
- HTTP 403 (Permission Denied - policy issue)
- HTTP 404 (Not Found - secret doesn't exist)
- HTTP 400 (Invalid Request - client error)

**Pattern**:
```rust
fn is_retryable_error(error: &vaultrs::error::ClientError) -> bool {
    match error {
        ClientError::RestClientError(_) => true, // Network errors
        ClientError::APIError { code: 503, .. } => true,
        ClientError::APIError { code: 429, .. } => true,
        _ => false,
    }
}
```

### Policy Requirements

**Decision**: Document minimum Vault policies for KV v2 data and metadata paths.

**Policy Structure** (secret/data/* for values, secret/metadata/* for versions):
```hcl
# nebula-credential-policy.hcl

# KV v2 data access (CRUD operations)
path "secret/data/nebula/credentials/*" {
  capabilities = ["create", "read", "update", "delete"]
}

# KV v2 metadata access (versioning, listing)
path "secret/metadata/nebula/credentials/*" {
  capabilities = ["read", "list", "delete"]
}

# List credentials (requires list on parent path)
path "secret/metadata/nebula/credentials" {
  capabilities = ["list"]
}

# Token renewal
path "auth/token/renew-self" {
  capabilities = ["update"]
}

# Token lookup (check TTL)
path "auth/token/lookup-self" {
  capabilities = ["read"]
}
```

**Application**:
```bash
vault policy write nebula-credential nebula-credential-policy.hcl

vault write auth/approle/role/nebula-credential \
  token_ttl=1h \
  token_max_ttl=4h \
  token_policies=nebula-credential
```

### Error Handling

**Decision**: Map Vault errors to `StorageError` with policy-specific context.

**Error Mapping**:
| Vault Error | HTTP Code | Application Error | Context |
|-------------|-----------|-------------------|---------|
| `permission denied` | 403 | `StorageError::PermissionDenied` | Token lacks policy capability on path |
| `* not found` | 404 | `StorageError::NotFound` | Secret does not exist |
| `invalid token` | 403 | `StorageError::AuthenticationFailed` | Token expired or revoked |
| `connection refused` | - | `StorageError::Unavailable` | Vault server unreachable |

**Actionable Error**:
```rust
ClientError::APIError { code: 403, errors } if errors.iter().any(|e| e.contains("permission denied")) => {
    StorageError::PermissionDenied {
        resource: "Vault secret".into(),
        required_permission: "Policy with read/write on secret/data/nebula/credentials/*".into(),
        fix: "vault policy write nebula-credential <policy-file.hcl>".into(),
    }
}
```

### Performance

**Typical Latency** (local deployment, p50/p95/p99):
- Token authentication: 50ms / 150ms / 300ms
- KV v2 read: 20ms / 80ms / 200ms
- KV v2 write: 30ms / 120ms / 250ms
- KV v2 list: 40ms / 180ms / 400ms

**Rate Limits**:
- Vault OSS: No hard rate limits (capacity depends on hardware)
- Vault Enterprise: Configurable rate limits per namespace
- Typical deployment: 5,000-10,000 req/s

**Connection Pooling**:
- `VaultClient` uses `reqwest` internally (automatic pooling)
- Share single `Arc<VaultClient>` across application
- Limit concurrent requests with `Semaphore` (100-200 recommended)

### Namespace Support (Enterprise)

**Decision**: Support namespace configuration for multi-tenant Vault Enterprise deployments.

**Namespace Pattern**:
```rust
let client = VaultClient::new(
    VaultClientSettingsBuilder::default()
        .address("https://vault.example.com:8200")
        .namespace("org1/team-a") // Vault Enterprise feature
        .build()?
)?;

// All operations scoped to namespace
kv2::read(&client, "secret", "credentials/db").await?;
// ↓ Translates to:
// GET /v1/org1/team-a/secret/data/credentials/db
```

**Use Cases**:
- Separate namespaces for dev/staging/prod
- Team isolation in shared Vault deployment
- Independent audit logs per namespace

---

## 4. Kubernetes Secrets

### SDK and Client Initialization

**Decision**: Use `kube` 0.87+ with automatic in-cluster vs out-of-cluster detection.

**Rationale**: kube-rs is the official Rust client for Kubernetes, actively maintained by CNCF, supports all K8s API features including watch streams, RBAC, and namespace isolation.

**Client Pattern**:
```rust
use kube::Client;
use k8s_openapi::api::core::v1::Secret;
use kube::api::{Api, PostParams, DeleteParams, ListParams};

// Automatic detection (in-cluster service account or kubeconfig)
let client = Client::try_default().await?;

// Namespace-scoped API for Secrets
let secrets: Api<Secret> = Api::namespaced(client, "nebula");
```

**In-Cluster vs Out-of-Cluster**:
- In-cluster: Uses mounted service account token at `/var/run/secrets/kubernetes.io/serviceaccount/token`
- Out-of-cluster: Uses `~/.kube/config` or `KUBECONFIG` environment variable

### RBAC Configuration

**Decision**: Document required RBAC permissions for read-only and full management access.

**Read-Only ServiceAccount**:
```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: nebula-credential-reader
  namespace: nebula
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: nebula-credential-reader
  namespace: nebula
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list", "watch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: nebula-credential-reader
  namespace: nebula
subjects:
- kind: ServiceAccount
  name: nebula-credential-reader
  namespace: nebula
roleRef:
  kind: Role
  name: nebula-credential-reader
  apiGroup: rbac.authorization.k8s.io
```

**Full Management**:
```yaml
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
```

**Important**: Built-in `view` ClusterRole does NOT grant Secrets access (privilege escalation concern). Must explicitly grant Secret permissions.

### Namespace Isolation

**Decision**: Enforce namespace-scoped operations, document cross-namespace access patterns (forbidden by default).

**Default Behavior**:
- ServiceAccounts can only access resources in same namespace
- Attempting cross-namespace access returns HTTP 403 Forbidden
- Cross-namespace requires ClusterRole + ClusterRoleBinding (discouraged)

**Best Practice**:
- One namespace per environment (dev, staging, prod)
- Use separate ServiceAccounts per application
- Never use ClusterRole for application credentials

### Retry Strategy

**Decision**: Always use `.default_backoff()` on watch streams, implement exponential backoff for API calls (200ms initial, 2min max).

**Rationale**: Kubernetes API server implements API Priority and Fairness (APF) with rate limiting. Aggressive retry without backoff can starve other clients.

**Backoff Parameters**:
- Initial delay: 200ms
- Max delay: 2 minutes
- Multiplier: 2x
- Max retries: 5

**Watch Pattern** (CRITICAL):
```rust
use kube::runtime::watcher;
use futures::TryStreamExt;

let secrets: Api<Secret> = Api::namespaced(client, "nebula");
let watch_config = watcher::Config::default();

let mut stream = watcher(secrets, watch_config)
    .default_backoff() // MUST INCLUDE - prevents API server spam
    .boxed();

while let Some(event) = stream.try_next().await? {
    // Handle event
}
```

### Error Handling

**Decision**: Map K8s API errors to `StorageError` with RBAC-specific context and `kubectl` remediation commands.

**Error Mapping**:
| K8s Error | HTTP Code | Application Error | Fix |
|-----------|-----------|-------------------|-----|
| `Forbidden` | 403 | `StorageError::PermissionDenied` | Grant RBAC permissions |
| `NotFound` | 404 | `StorageError::NotFound` | Secret does not exist in namespace |
| `Unauthorized` | 401 | `StorageError::AuthenticationFailed` | Invalid service account token |
| `Conflict` | 409 | `StorageError::ConflictError` | Resource version mismatch |
| `TooManyRequests` | 429 | Automatic retry with backoff | APF rate limiting |

**Actionable Error**:
```rust
kube::Error::Api(api_error) if api_error.code == 403 => {
    StorageError::PermissionDenied {
        resource: format!("Secret '{}' in namespace '{}'", secret_name, namespace),
        required_permission: "RBAC permissions: secrets.get, secrets.list (Role or ClusterRole)".into(),
        fix: format!(
            "kubectl auth can-i get secrets --as=system:serviceaccount:{}:{} -n {}",
            namespace, service_account, namespace
        ),
    }
}
```

### Performance

**Typical Latency** (p50/p95/p99):
- GET Secret: 5-50ms (in-cluster)
- LIST Secrets: 50-500ms (depends on count)
- CREATE Secret: 20-100ms
- WATCH (initial): 10-100ms then streaming

**Rate Limits**:
- API Priority and Fairness (APF): ~100 QPS per ServiceAccount
- Configurable via FlowSchema and PriorityLevelConfiguration
- Throttling returns HTTP 429

**Optimization**:
- Use WATCH instead of polling LIST (streaming is efficient)
- Cache with 5+ minute TTL (use moka or similar)
- Reuse single `Client` instance (connection pooling)
- Limit concurrent requests with `Semaphore`

### Size Limits

**Decision**: Validate credential size before submission (1MB hard limit, 256KB recommended).

**Size Limit**: 1MB (1,048,576 bytes) per Secret
- Base64 encoding overhead: ~33% increase
- Recommended max: 256KB raw data
- etcd total size limit: 1.5MB per object (includes metadata)

**Validation**:
```rust
const K8S_SECRET_MAX_SIZE: usize = 1_048_576; // 1MB
const K8S_SECRET_RECOMMENDED_SIZE: usize = 256 * 1024; // 256KB

if credential_data.len() > K8S_SECRET_MAX_SIZE {
    return Err(StorageError::CredentialTooLarge {
        size: credential_data.len(),
        limit: K8S_SECRET_MAX_SIZE,
        provider: "Kubernetes Secrets".into(),
    });
}

if credential_data.len() > K8S_SECRET_RECOMMENDED_SIZE {
    tracing::warn!(
        size = credential_data.len(),
        "Credential size exceeds recommended limit (256KB)"
    );
}
```

### Labels and Annotations

**Decision**: Convert `CredentialMetadata` to K8s labels (for filtering) and annotations (for extended metadata).

**Labels** (indexed, max 63 chars):
- `app.kubernetes.io/name=nebula-credential`
- `app.kubernetes.io/component=credential-storage`
- `credential-type=oauth2`
- `environment=production`

**Annotations** (not indexed, max 256KB total):
- `nebula.io/created-by=platform-team`
- `nebula.io/rotation-policy=quarterly`
- `nebula.io/description=GitHub OAuth2 credentials`

**Conversion Pattern**:
```rust
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

let mut labels = BTreeMap::new();
labels.insert("app.kubernetes.io/name".to_string(), "nebula-credential".to_string());
labels.insert("credential-type".to_string(), metadata.credential_type.clone());

let mut annotations = BTreeMap::new();
annotations.insert("nebula.io/created-by".to_string(), metadata.owner.clone());
annotations.insert("nebula.io/rotation-policy".to_string(), "quarterly".to_string());

let object_meta = ObjectMeta {
    name: Some(credential_id.to_string()),
    namespace: Some("nebula".into()),
    labels: Some(labels),
    annotations: Some(annotations),
    ..Default::default()
};
```

---

## 5. Local Encrypted Storage

### Atomic Writes

**Decision**: Use write-to-temp-then-rename pattern with UUID temp filenames in same directory as target.

**Rationale**: Atomic rename prevents partial writes and corruption during crashes. Using same directory avoids cross-filesystem issues (rename across filesystems fails).

**Implementation**:
```rust
use uuid::Uuid;
use std::path::Path;

fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    // Generate UUID temp filename in same directory
    let temp_name = format!(".{}.tmp", Uuid::new_v4().simple());
    let temp_path = path.parent().unwrap().join(temp_name);
    
    // Write to temp file
    let mut temp_file = File::create(&temp_path)?;
    temp_file.write_all(data)?;
    temp_file.sync_all()?; // fsync before rename
    
    // Atomic rename (POSIX guarantees atomicity)
    std::fs::rename(&temp_path, path)?;
    
    // Sync parent directory (POSIX requirement for metadata durability)
    #[cfg(unix)]
    {
        let dir = File::open(path.parent().unwrap())?;
        dir.sync_all()?;
    }
    
    Ok(())
}
```

**Recommended Crate**: `atomicwrites` or `atomic-write-file` for production use (handles edge cases).

### File Permissions

**Decision**: Use 0600 on Unix (owner read/write), Windows ACL with owner-only access.

**Unix Pattern**:
```rust
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;

#[cfg(unix)]
fn create_secure_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600) // Owner read/write only
        .open(path)?;
    
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}
```

**Windows Pattern**:
```rust
#[cfg(windows)]
use windows_acl::acl::ACL;

#[cfg(windows)]
fn set_owner_only_permissions(path: &Path) -> std::io::Result<()> {
    // Requires windows-acl crate
    let user_sid = windows_acl::helper::current_user()?;
    let mut acl = ACL::from_file_path(path, false)?;
    
    // Remove all entries
    acl.all_entries().for_each(|entry| { acl.remove_entry(&entry).ok(); });
    
    // Add owner read/write entry
    acl.add_entry(&user_sid, true, true)?; // read, write
    
    Ok(())
}
```

**Recommended Crate**: `windows-acl` for Windows ACL manipulation.

### Directory Creation

**Decision**: Use `DirBuilder` with `mode(0o700)` for atomic secure directory creation on Unix.

**Pattern**:
```rust
use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;

#[cfg(unix)]
fn create_credential_storage_dir(path: &Path) -> std::io::Result<()> {
    DirBuilder::new()
        .recursive(true)  // Create all missing parents
        .mode(0o700)      // Owner read/write/execute only
        .create(path)?;
    Ok(())
}
```

**Windows**: Create directory first, then apply ACL permissions (no atomic operation).

### Encryption

**Decision**: Continue using existing AES-256-GCM implementation from Phase 1, switch to random nonces for file storage.

**Rationale**: Phase 1 implementation is solid (AES-256-GCM, Argon2id key derivation, automatic zeroization). For file storage, random nonces are preferred over counter-based (survives application restarts).

**Random Nonce Pattern**:
```rust
use rand::RngCore;

pub fn generate_file_nonce() -> [u8; 12] {
    let mut nonce = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce); // Cryptographically secure RNG
    nonce
}

pub fn encrypt_for_storage(key: &EncryptionKey, plaintext: &[u8]) 
    -> Result<EncryptedData, CryptoError> 
{
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes())?;
    let nonce_bytes = generate_file_nonce();
    let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);
    
    let ciphertext = cipher.encrypt(nonce, plaintext)?;
    
    // EncryptedData includes nonce, ciphertext, and auth tag
    Ok(EncryptedData::new(nonce_bytes, ciphertext))
}
```

**Security**: 96-bit random nonce with AES-GCM is safe for 2^48 encryptions before 50% collision probability.

### File Format

**Decision**: JSON serialization with version field, encrypted data, and metadata.

**Structure**:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct CredentialFile {
    pub version: u32,                    // File format version
    pub encrypted_data: EncryptedData,   // Ciphertext, nonce, tag
    pub metadata: CredentialMetadata,    // Non-sensitive metadata
}

impl CredentialFile {
    pub const CURRENT_VERSION: u32 = 1;
    
    pub fn new(encrypted_data: EncryptedData, metadata: CredentialMetadata) -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            encrypted_data,
            metadata,
        }
    }
}
```

**File Extension**: `.enc.json` (clearly indicates encrypted JSON content) or `.cred` (shorter custom extension).

**Serialization**:
```rust
pub fn write_credential_file(path: &Path, cred_file: &CredentialFile) 
    -> Result<(), StorageError> 
{
    let json_bytes = serde_json::to_vec_pretty(cred_file)?;
    atomic_write(path, &json_bytes)?;
    set_secure_permissions(path)?;
    Ok(())
}
```

### Concurrency

**Decision**: Use `fs2::FileExt` for advisory file locking (exclusive for writes, shared for reads).

**Rationale**: Advisory file locking prevents concurrent writes from corrupting credentials. Works cross-platform (flock on Unix, LockFileEx on Windows).

**Locking Pattern**:
```rust
use fs2::FileExt;

pub fn write_with_lock<F>(path: &Path, f: F) -> Result<(), StorageError>
where
    F: FnOnce(&File) -> Result<(), StorageError>,
{
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    
    file.lock_exclusive()?; // Blocks until acquired
    let result = f(&file);
    file.unlock()?;
    
    result
}

pub fn read_with_lock<F, T>(path: &Path, f: F) -> Result<T, StorageError>
where
    F: FnOnce(&File) -> Result<T, StorageError>,
{
    let file = File::open(path)?;
    file.lock_shared()?; // Multiple readers allowed
    let result = f(&file);
    file.unlock()?;
    result
}
```

**Recommended Crate**: `fs2` for cross-platform file locking.

### Error Handling

**Decision**: Convert `std::io::ErrorKind` to actionable `StorageError` variants with platform-specific fix suggestions.

**Error Mapping**:
```rust
use std::io::ErrorKind;

impl From<io::Error> for StorageError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            ErrorKind::NotFound => StorageError::NotFound { /* ... */ },
            ErrorKind::PermissionDenied => StorageError::PermissionDenied {
                fix: "chmod 600 <file> (Unix) or adjust Windows ACLs".into(),
                /* ... */
            },
            ErrorKind::StorageFull => StorageError::DiskFull { /* ... */ },
            ErrorKind::ReadOnlyFilesystem => StorageError::ReadOnlyFilesystem { /* ... */ },
            _ => StorageError::WriteFailure(error.to_string()),
        }
    }
}
```

**Actionable Errors**: Include specific commands to fix issues (`chmod`, `az role assignment`, `kubectl auth can-i`).

### Cross-Platform Considerations

**Decision**: Use `directories` crate for platform-appropriate base paths, handle case sensitivity differences.

**Base Path Pattern**:
```rust
use directories::ProjectDirs;

fn get_credential_storage_dir() -> Result<PathBuf, StorageError> {
    let proj_dirs = ProjectDirs::from("com", "nebula", "Nebula")?;
    
    // Returns platform-appropriate paths:
    // - Windows: C:\Users\<User>\AppData\Roaming\nebula\Nebula\data
    // - macOS: /Users/<User>/Library/Application Support/com.nebula.Nebula
    // - Linux: /home/<user>/.local/share/nebula
    Ok(proj_dirs.data_dir().join("credentials"))
}
```

**Path Handling**:
- Always use `Path::join()` for cross-platform path building
- Never hardcode `/` or `\` separators
- Remember `Path::starts_with()` is case-sensitive on ALL platforms

**Filesystem Limits**:
| Filesystem | Max Filename | Max Path | Case Sensitive |
|------------|--------------|----------|----------------|
| NTFS | 255 chars | 32,767 chars | No |
| FAT32 | 255 chars | 260 chars | No |
| ext4 | 255 bytes | 4096 bytes | Yes |
| APFS | 255 UTF-8 chars | ~1024 chars | Optional |

**Recommended Crate**: `directories` for platform-appropriate paths.

### Performance

**Decision**: Use `std::fs` in `spawn_blocking` for async operations (NOT `tokio::fs` directly).

**Rationale**: `tokio::fs` is 64x slower for sequential operations, 25x slower for concurrent operations compared to `std::fs`. `tokio::fs` internally uses `spawn_blocking` anyway, so explicitly using it with `std::fs` is faster.

**Pattern**:
```rust
pub async fn store_credential_async(
    id: &CredentialId,
    data: EncryptedData,
) -> Result<(), StorageError> {
    let path = get_credential_path(id)?;
    let cred_file = CredentialFile::new(data, metadata);
    
    tokio::task::spawn_blocking(move || {
        write_credential_file(&path, &cred_file)
    })
    .await??;
    
    Ok(())
}
```

**Latency Expectations** (SSD):
- Small file read (<10KB): 0.1-1ms
- Small file write with fsync: 1-10ms
- Directory listing (100 files): 1-5ms
- File lock acquisition: <0.1ms

**Buffering**: Not needed for credentials (single read/write operations). `serde_json` handles internal buffering.

---

## 6. Cross-Cutting Concerns

### Shared Retry Logic

**Decision**: Extract exponential backoff with jitter to `nebula-credential/src/utils/retry.rs` for use across all cloud providers.

**Unified Retry Module**:
```rust
// src/utils/retry.rs
use tokio::time::{sleep, Duration};
use rand::Rng;

pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub multiplier: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay_ms: 100,
            max_delay_ms: 30_000,
            multiplier: 2.0,
        }
    }
}

pub async fn retry_with_backoff<F, T, E>(
    policy: &RetryPolicy,
    is_retryable: impl Fn(&E) -> bool,
    operation: F,
) -> Result<T, E>
where
    F: Fn() -> futures::future::BoxFuture<'static, Result<T, E>>,
{
    let mut attempt = 0;
    let mut rng = rand::thread_rng();
    
    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) if attempt < policy.max_retries && is_retryable(&e) => {
                let delay = (policy.base_delay_ms as f64 * policy.multiplier.powi(attempt as i32)) as u64;
                let delay = delay.min(policy.max_delay_ms);
                
                // Add jitter: ±25%
                let jitter_range = (delay as f64 * 0.25) as u64;
                let jittered_delay = delay + rng.gen_range(0..jitter_range);
                
                tracing::warn!(
                    attempt = attempt + 1,
                    max_retries = policy.max_retries,
                    delay_ms = jittered_delay,
                    "Operation failed, retrying"
                );
                
                sleep(Duration::from_millis(jittered_delay)).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}
```

**Usage Across Providers**:
```rust
// AWS
retry_with_backoff(&policy, |e| is_aws_retryable(e), || Box::pin(aws_operation())).await?;

// Azure
retry_with_backoff(&policy, |e| is_azure_retryable(e), || Box::pin(azure_operation())).await?;

// Vault
retry_with_backoff(&policy, |e| is_vault_retryable(e), || Box::pin(vault_operation())).await?;
```

### Provider Configuration

**Decision**: Each provider has its own config struct with builder pattern for optional fields.

**Base Configuration Trait**:
```rust
pub trait ProviderConfig: Send + Sync {
    fn validate(&self) -> Result<(), ConfigError>;
}
```

**Provider-Specific Configs**:
```rust
// Local
pub struct LocalStorageConfig {
    pub base_path: PathBuf,
    pub create_dir: bool, // Auto-create if missing
}

// AWS
pub struct AwsSecretsManagerConfig {
    pub region: Option<String>, // None = auto-detect
    pub timeout: Duration,
    pub retry_policy: RetryPolicy,
}

// Azure
pub struct AzureKeyVaultConfig {
    pub vault_url: String,
    pub credential_type: AzureCredentialType, // ManagedIdentity, ServicePrincipal, etc.
    pub timeout: Duration,
    pub retry_policy: RetryPolicy,
}

// Vault
pub struct VaultConfig {
    pub address: String,
    pub token: Option<String>, // None = use AppRole
    pub namespace: Option<String>, // Enterprise only
    pub mount_path: String,
    pub tls_verify: bool,
    pub retry_policy: RetryPolicy,
}

// Kubernetes
pub struct KubernetesSecretsConfig {
    pub namespace: String,
    pub kubeconfig_path: Option<PathBuf>, // None = in-cluster
    pub timeout: Duration,
    pub retry_policy: RetryPolicy,
}
```

### Metrics Foundation

**Decision**: Prepare metrics struct with per-provider operation tracking for Phase 8 observability.

**Metrics Structure**:
```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub struct StorageMetrics {
    pub store_count: AtomicU64,
    pub store_latency_sum_ms: AtomicU64,
    pub retrieve_count: AtomicU64,
    pub retrieve_latency_sum_ms: AtomicU64,
    pub delete_count: AtomicU64,
    pub list_count: AtomicU64,
    pub error_count: AtomicU64,
    pub retry_count: AtomicU64,
}

impl StorageMetrics {
    pub fn record_operation(&self, operation: &str, duration: Duration, success: bool) {
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
            _ => {}
        }
        
        if !success {
            self.error_count.fetch_add(1, Ordering::Relaxed);
        }
    }
    
    pub fn avg_store_latency_ms(&self) -> u64 {
        let count = self.store_count.load(Ordering::Relaxed);
        if count == 0 { return 0; }
        self.store_latency_sum_ms.load(Ordering::Relaxed) / count
    }
}
```

**Integration with Providers**:
```rust
pub struct LocalStorageProvider {
    config: LocalStorageConfig,
    metrics: Arc<StorageMetrics>,
}

impl StorageProvider for LocalStorageProvider {
    async fn store(&self, id: &CredentialId, data: EncryptedData, ...) -> Result<(), StorageError> {
        let start = Instant::now();
        let result = self.store_impl(id, data).await;
        self.metrics.record_operation("store", start.elapsed(), result.is_ok());
        result
    }
}
```

### Testing Strategy

**Decision**: Use MockStorageProvider for unit tests, testcontainers for integration tests with real backends.

**Mock Pattern**:
```rust
pub struct MockStorageProvider {
    storage: Arc<RwLock<HashMap<CredentialId, (EncryptedData, CredentialMetadata)>>>,
}

#[async_trait]
impl StorageProvider for MockStorageProvider {
    async fn store(&self, id: &CredentialId, data: EncryptedData, metadata: CredentialMetadata, ...) 
        -> Result<(), StorageError> 
    {
        let mut storage = self.storage.write().await;
        storage.insert(id.clone(), (data, metadata));
        Ok(())
    }
    
    async fn retrieve(&self, id: &CredentialId, ...) 
        -> Result<(EncryptedData, CredentialMetadata), StorageError> 
    {
        let storage = self.storage.read().await;
        storage.get(id).cloned().ok_or(StorageError::NotFound { /* ... */ })
    }
}
```

**Testcontainers Integration** (example for Vault):
```rust
#[cfg(test)]
mod integration_tests {
    use testcontainers::{clients::Cli, images::generic::GenericImage};
    
    #[tokio::test]
    async fn test_vault_provider_integration() {
        let docker = Cli::default();
        let vault = docker.run(GenericImage::new("vault", "latest")
            .with_env_var("VAULT_DEV_ROOT_TOKEN_ID", "test-token"));
        
        let vault_url = format!("http://127.0.0.1:{}", vault.get_host_port_ipv4(8200));
        let config = VaultConfig {
            address: vault_url,
            token: Some("test-token".into()),
            ..Default::default()
        };
        
        let provider = VaultProvider::new(config).await?;
        
        // Test CRUD operations against real Vault instance
        // ...
    }
}
```

---

## Summary and Next Steps

All research areas have been thoroughly investigated and documented:

✅ **AWS Secrets Manager**: SDK usage, retry, IAM permissions, error handling, performance, size limits, tagging  
✅ **Azure Key Vault**: SDK usage, Managed Identity, RBAC, retry, soft-delete, performance  
✅ **HashiCorp Vault**: vaultrs SDK, KV v2 versioning, token renewal, policies, namespaces  
✅ **Kubernetes Secrets**: kube-rs SDK, RBAC, namespace isolation, watch patterns, size limits, labels/annotations  
✅ **Local Encrypted Storage**: Atomic writes, permissions, encryption, file format, concurrency, cross-platform  

**Key Decisions Made**:
1. Shared retry logic in `utils/retry.rs` with exponential backoff and jitter
2. Provider-specific configs with builder pattern for flexibility
3. Metrics foundation for Phase 8 observability
4. TDD with MockStorageProvider for unit tests, testcontainers for integration tests
5. All providers implement `StorageProvider` trait from Phase 1 without breaking changes

**Proceed to Phase 1**: Generate data-model.md, contracts/, and quickstart.md based on these research findings.
