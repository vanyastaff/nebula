# Kubernetes Secrets Integration - Best Practices Research

**Research Date**: 2026-02-03  
**Purpose**: Comprehensive research on Kubernetes Secrets integration patterns for Rust applications using kube-rs  
**Target**: nebula-credential Phase 2 (Storage Backends)  
**Status**: Research Complete

---

## Table of Contents

1. [SDK Usage Patterns](#1-sdk-usage-patterns)
2. [RBAC Configuration](#2-rbac-configuration)
3. [Namespace Isolation](#3-namespace-isolation)
4. [Retry Strategies](#4-retry-strategies)
5. [Error Handling](#5-error-handling)
6. [Security](#6-security)
7. [Performance](#7-performance)
8. [Size Limits](#8-size-limits)
9. [Labels and Annotations](#9-labels-and-annotations)
10. [Implementation Recommendations](#10-implementation-recommendations)

---

## 1. SDK Usage Patterns

### 1.1 kube-rs Crate Overview

**Current Version**: 3.0.0 (as of 2026)

**Dependencies**:
```toml
[dependencies]
kube = { version = "3.0.0", features = ["runtime", "derive"] }
k8s-openapi = { version = "0.27.0", features = ["latest"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
```

**Architecture**: kube-rs is a CNCF Sandbox Project providing a Kubernetes client similar to client-go, with runtime abstractions inspired by controller-runtime and CRD macros inspired by kubebuilder.

### 1.2 Client Initialization

#### Automatic Configuration (Recommended)

```rust
use kube::Client;

// Try in-cluster first, then kubeconfig
let client = Client::try_default().await?;
```

**Configuration Priority**:
1. **In-Cluster**: Uses `KUBERNETES_SERVICE_HOST`, `KUBERNETES_SERVICE_PORT`, and service account token at `/var/run/secrets/kubernetes.io/serviceaccount/`
2. **Kubeconfig**: Loads from `$KUBECONFIG` or `~/.kube/config`
3. **Error**: Returns error if both fail

#### Explicit Configuration

```rust
use kube::{Client, Config};

// Load in-cluster config explicitly
let config = Config::incluster()?;
let client = Client::try_from(config)?;

// Load kubeconfig explicitly
let config = Config::from_kubeconfig(&kube::config::KubeConfigOptions::default()).await?;
let client = Client::try_from(config)?;
```

### 1.3 API Operations

#### Creating an API Instance

```rust
use kube::Api;
use k8s_openapi::api::core::v1::Secret;

// Namespace-scoped API
let secrets: Api<Secret> = Api::namespaced(client.clone(), "nebula-prod");

// Use current namespace (from service account)
let secrets: Api<Secret> = Api::default_namespaced(client.clone());

// Cluster-scoped API (for ClusterRoles)
let secrets: Api<Secret> = Api::all(client.clone());
```

#### CRUD Operations

```rust
use kube::api::{PostParams, DeleteParams, ListParams, PatchParams, Patch};
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::ByteString;
use std::collections::BTreeMap;

// CREATE
let mut data = BTreeMap::new();
data.insert("credential.json".to_string(), ByteString(credential_json.into_bytes()));

let secret = Secret {
    metadata: ObjectMeta {
        name: Some("my-secret".to_string()),
        namespace: Some("nebula-prod".to_string()),
        labels: Some(labels),
        ..Default::default()
    },
    data: Some(data),
    type_: Some("Opaque".to_string()),
    ..Default::default()
};

let created = secrets.create(&PostParams::default(), &secret).await?;

// GET
let secret = secrets.get("my-secret").await?;

// LIST
let list_params = ListParams::default()
    .labels("app=nebula-credential");
let secret_list = secrets.list(&list_params).await?;

for item in secret_list.items {
    println!("Found: {}", item.metadata.name.unwrap_or_default());
}

// UPDATE (replace)
let updated = secrets.replace("my-secret", &PostParams::default(), &secret).await?;

// PATCH (partial update)
let patch = json!({
    "metadata": {
        "labels": {
            "updated": "true"
        }
    }
});
let patched = secrets.patch(
    "my-secret",
    &PatchParams::apply("nebula-credential"),
    &Patch::Merge(patch)
).await?;

// DELETE
secrets.delete("my-secret", &DeleteParams::default()).await?;
```

### 1.4 Watching Resources

```rust
use kube::runtime::{watcher, WatchStreamExt};
use futures::TryStreamExt;

let secrets: Api<Secret> = Api::namespaced(client, "nebula-prod");

// Watch for changes with automatic reconnection
let stream = watcher(secrets, watcher::Config::default())
    .default_backoff()  // CRITICAL: Always use backoff!
    .applied_objects();

stream.try_for_each(|secret| async move {
    println!("Secret changed: {}", secret.metadata.name.unwrap_or_default());
    Ok(())
}).await?;
```

**Key Points**:
- `watcher()` automatically handles reconnection on failures
- `default_backoff()` is **REQUIRED** to avoid spamming the API server
- Without backoff, continuous retry loops can starve resources and trigger rate limiting
- Events: `Init`, `Applied`, `Deleted`, `Restarted`

---

## 2. RBAC Configuration

### 2.1 Core Concepts

**RBAC Components**:
- **ServiceAccount**: Non-human identity for pods (namespace-scoped)
- **Role**: Permissions within a namespace
- **ClusterRole**: Permissions across all namespaces
- **RoleBinding**: Binds Role to ServiceAccount (namespace-scoped)
- **ClusterRoleBinding**: Binds ClusterRole to ServiceAccount (cluster-scoped)

### 2.2 Required Permissions for Secrets

**Read-Only Access**:
```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: secret-reader
  namespace: nebula-prod
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list", "watch"]
  # OPTIONAL: Restrict to specific secrets
  resourceNames:
  - nebula-credentials-*
```

**Full Management Access**:
```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: secret-manager
  namespace: nebula-prod
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
  resourceNames:
  - nebula-credentials-*
```

**RoleBinding**:
```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: nebula-secret-reader
  namespace: nebula-prod
subjects:
- kind: ServiceAccount
  name: nebula-app
  namespace: nebula-prod
roleRef:
  kind: Role
  name: secret-reader
  apiGroup: rbac.authorization.k8s.io
```

### 2.3 ClusterRole for Multi-Namespace Access

**When to Use**: Admin tools, cross-namespace credential sharing (use with extreme caution)

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: global-secret-reader
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list", "watch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: nebula-global-secrets
subjects:
- kind: ServiceAccount
  name: nebula-admin
  namespace: nebula-system
roleRef:
  kind: ClusterRole
  name: global-secret-reader
  apiGroup: rbac.authorization.k8s.io
```

### 2.4 RBAC Best Practices

1. **Principle of Least Privilege**: Grant only required verbs (e.g., read-only for consumers)
2. **Use Role over ClusterRole**: Namespace isolation by default
3. **Avoid Wildcards**: Never use `resources: ["*"]` or `verbs: ["*"]`
4. **Restrict resourceNames**: Limit to specific secret prefixes
5. **Separate Read/Write Roles**: Different ServiceAccounts for read vs write operations
6. **Regular Audits**: Review permissions quarterly

**Security Note**: The built-in `view` ClusterRole does **NOT** grant access to Secrets due to privilege escalation concerns.

---

## 3. Namespace Isolation

### 3.1 How Namespaces Provide Isolation

**Kubernetes Namespace Isolation**:
- **RBAC Boundary**: Roles, RoleBindings, and ServiceAccounts are namespace-scoped
- **Network Isolation**: NetworkPolicies can restrict pod-to-pod communication across namespaces
- **Resource Quotas**: Limit CPU, memory, and object counts per namespace
- **Default Deny**: Resources in one namespace cannot access resources in another without ClusterRole

### 3.2 Cross-Namespace Access Patterns

#### Forbidden by Default

```rust
// This FAILS without ClusterRole
let client = Client::try_default().await?;
let secrets_prod: Api<Secret> = Api::namespaced(client.clone(), "prod");
let secrets_dev: Api<Secret> = Api::namespaced(client.clone(), "dev");

// ServiceAccount in 'prod' cannot access 'dev' secrets
let secret = secrets_dev.get("dev-secret").await?;  // ❌ Forbidden (403)
```

#### Allowed with ClusterRole

```yaml
# Grant cross-namespace access (use sparingly)
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: cross-namespace-secrets
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list"]
  # Cannot restrict namespaces in ClusterRole
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: nebula-cross-ns
subjects:
- kind: ServiceAccount
  name: nebula-admin
  namespace: nebula-system
roleRef:
  kind: ClusterRole
  name: cross-namespace-secrets
  apiGroup: rbac.authorization.k8s.io
```

### 3.3 Service Account Scope

**Key Constraints**:
- ServiceAccounts exist within a namespace
- Cannot directly reference secrets in other namespaces
- Cross-namespace access requires ClusterRole + ClusterRoleBinding
- External services use fully qualified DNS: `<service>.<namespace>.svc.cluster.local`

### 3.4 Network Policies for Isolation

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: deny-cross-namespace
  namespace: nebula-prod
spec:
  podSelector: {}
  policyTypes:
  - Ingress
  - Egress
  ingress:
  - from:
    - namespaceSelector:
        matchLabels:
          name: nebula-prod  # Only from same namespace
  egress:
  - to:
    - namespaceSelector:
        matchLabels:
          name: nebula-prod
```

### 3.5 Best Practices

1. **One Namespace Per Environment**: `dev`, `staging`, `prod`
2. **Namespace-Scoped Credentials**: Store credentials in the namespace where they're used
3. **Avoid ClusterRoles**: Use only for admin tools and monitoring
4. **Label Namespaces**: Use labels for NetworkPolicy selectors
5. **Resource Quotas**: Prevent secret sprawl per namespace

---

## 4. Retry Strategies

### 4.1 Kubernetes API Rate Limiting

**API Priority and Fairness (APF)**:
- Introduced in Kubernetes 1.20+
- Rate limits requests to prevent API server overload
- Returns `429 Too Many Requests` when limits exceeded
- Uses **shuffle sharding** and **fair queuing** for request distribution

### 4.2 Exponential Backoff

**Default Behavior**: kube-rs watchers use exponential backoff inspired by client-go

**Backoff Configuration**:
```rust
use kube::runtime::watcher;
use backon::{ExponentialBuilder, Retryable};

// Default backoff (recommended for most cases)
let stream = watcher(api, watcher::Config::default())
    .default_backoff()
    .applied_objects();

// Custom backoff
use kube::runtime::WatchStreamExt;
let backoff = ExponentialBuilder::default()
    .with_min_delay(std::time::Duration::from_millis(200))
    .with_max_delay(std::time::Duration::from_secs(120))
    .with_max_times(10);

let stream = watcher(api, watcher::Config::default())
    .backoff(backoff)
    .applied_objects();
```

**Backoff Parameters** (inspired by client-go):
- **Initial Delay**: 200ms
- **Max Delay**: 2 minutes
- **Multiplier**: 2x (exponential)
- **Jitter**: Randomized to avoid thundering herd

### 4.3 Watch Reconnection

**Automatic Reconnection**:
```rust
// Watcher automatically reconnects on:
// - Network failures
// - API server restarts
// - 410 Gone (resource version too old)
// - 429 Too Many Requests
let stream = watcher(secrets, watcher::Config::default())
    .default_backoff()  // CRITICAL
    .applied_objects();

// Reconnection behavior:
// 1. Attempts to resume from last resource version
// 2. If resource version invalid, starts fresh with Event::Init
// 3. Retries with exponential backoff on failures
```

### 4.4 Handling 429 Rate Limiting

**Client-Side Handling**:
```rust
use kube::Error;
use tokio::time::{sleep, Duration};

async fn retry_with_backoff<F, T, E>(mut f: F) -> Result<T, E>
where
    F: FnMut() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, E>>>>,
    E: std::fmt::Debug,
{
    let mut delay = Duration::from_millis(200);
    let max_delay = Duration::from_secs(120);
    let mut attempts = 0;
    
    loop {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                // Check if error is 429 (rate limit)
                // In production, parse error to check status code
                attempts += 1;
                if attempts >= 10 {
                    return Err(e);
                }
                
                tracing::warn!("Request failed, retrying in {:?}", delay);
                sleep(delay).await;
                
                // Exponential backoff with cap
                delay = std::cmp::min(delay * 2, max_delay);
            }
        }
    }
}

// Usage
let secret = retry_with_backoff(|| {
    Box::pin(secrets.get("my-secret"))
}).await?;
```

### 4.5 Best Practices

1. **Always Use Backoff**: Never retry immediately without delay
2. **Add Jitter**: Randomize delays to avoid synchronized retries
3. **Limit Retries**: Cap at 10 attempts or 2 minutes total
4. **Log Retries**: Track retry metrics for monitoring
5. **Watch over Poll**: Use watchers instead of polling LIST operations
6. **Pagination**: Use `limit` and `continue` for large LIST results

---

## 5. Error Handling

### 5.1 kube::Error Enum

**Complete Variants** (kube 3.0.1):
```rust
pub enum Error {
    Api(Status),                // Kubernetes API errors (403, 404, 409, etc)
    Auth(AuthError),            // Authentication failures
    BuildRequest(BuildError),   // Request construction errors
    Discovery(DiscoveryError),  // Service discovery failures
    FromUtf8(FromUtf8Error),    // UTF-8 encoding errors
    HttpError(HttpError),       // HTTP protocol errors
    HyperError(HyperError),     // Hyper HTTP client errors
    InferConfig(InferError),    // Config inference failures
    InferKubeconfig(KubeconfigError), // Kubeconfig parsing failures
    LinesCodecMaxLineLengthExceeded,  // Event stream line too long
    OpensslTls(OpensslTlsError),      // OpenSSL TLS errors
    ProxyProtocolDisabled,            // Proxy requires feature flag
    ProxyProtocolUnsupported,         // Unsupported proxy protocol
    ReadEvents(ReadEventsError),      // Event stream I/O errors
    RustlsTls(RustlsTlsError),        // Rustls TLS errors
    SerdeError(SerdeError),           // JSON serialization errors
    Service(BoxError),                // Generic service errors
    TlsRequired,                      // Missing TLS stack
    UpgradeConnection(UpgradeError),  // WebSocket upgrade failures
}
```

### 5.2 Common HTTP Error Codes

**Status Codes in Api(Status) variant**:
- **401 Unauthorized**: Invalid or missing authentication
- **403 Forbidden**: Valid auth, but insufficient RBAC permissions
- **404 Not Found**: Secret does not exist
- **409 Conflict**: Secret already exists (on create)
- **410 Gone**: Resource version too old (watch)
- **422 Unprocessable Entity**: Invalid secret data/format
- **429 Too Many Requests**: Rate limit exceeded
- **500 Internal Server Error**: API server error
- **503 Service Unavailable**: API server overloaded

### 5.3 Error Matching Patterns

```rust
use kube::Error;

match secrets.get("my-secret").await {
    Ok(secret) => {
        // Success
        println!("Found secret");
    }
    Err(Error::Api(status)) => {
        match status.code {
            401 => {
                // Unauthorized: Check service account token
                eprintln!("Authentication failed: {}", status.message);
            }
            403 => {
                // Forbidden: Check RBAC permissions
                eprintln!("Permission denied: {}", status.message);
                eprintln!("Reason: {}", status.reason);
                // Hint: Run `kubectl auth can-i get secrets --as=system:serviceaccount:ns:sa`
            }
            404 => {
                // Not Found: Secret doesn't exist
                eprintln!("Secret not found: {}", status.message);
            }
            409 => {
                // Conflict: Secret already exists (on create)
                eprintln!("Secret already exists, try updating instead");
            }
            410 => {
                // Gone: Resource version too old (watch)
                eprintln!("Resource version expired, restarting watch");
            }
            429 => {
                // Rate limited: Exponential backoff
                eprintln!("Rate limited, backing off");
            }
            _ => {
                eprintln!("API error {}: {}", status.code, status.message);
            }
        }
    }
    Err(Error::Auth(e)) => {
        // Authentication error
        eprintln!("Auth error: {}", e);
    }
    Err(Error::InferConfig(e)) => {
        // Could not load config (no kubeconfig, not in cluster)
        eprintln!("Failed to infer config: {}", e);
    }
    Err(e) => {
        // Other errors
        eprintln!("Error: {}", e);
    }
}
```

### 5.4 Mapping to Application Errors

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Credential not found: {0}")]
    NotFound(String),
    
    #[error("Permission denied: {0}. Check RBAC permissions with: kubectl auth can-i {1} secrets --as=system:serviceaccount:{2}:{3}")]
    PermissionDenied(String, String, String, String), // message, verb, namespace, sa
    
    #[error("Authentication failed: {0}. Verify service account token is mounted")]
    Unauthorized(String),
    
    #[error("Credential already exists: {0}")]
    AlreadyExists(String),
    
    #[error("Rate limited: {0}. Retry with exponential backoff")]
    RateLimited(String),
    
    #[error("Kubernetes API error: {0}")]
    ApiError(String),
    
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

impl From<kube::Error> for StorageError {
    fn from(err: kube::Error) -> Self {
        match err {
            kube::Error::Api(status) => {
                match status.code {
                    401 => StorageError::Unauthorized(status.message),
                    403 => {
                        // Extract namespace and service account from context
                        let namespace = "unknown";
                        let sa = "unknown";
                        let verb = "get"; // or extract from status
                        StorageError::PermissionDenied(
                            status.message,
                            verb.to_string(),
                            namespace.to_string(),
                            sa.to_string()
                        )
                    }
                    404 => StorageError::NotFound(status.message),
                    409 => StorageError::AlreadyExists(status.message),
                    429 => StorageError::RateLimited(status.message),
                    _ => StorageError::ApiError(format!("{}: {}", status.code, status.message)),
                }
            }
            kube::Error::Auth(e) => StorageError::Unauthorized(e.to_string()),
            kube::Error::InferConfig(e) => StorageError::ConfigError(e.to_string()),
            e => StorageError::ApiError(e.to_string()),
        }
    }
}
```

### 5.5 Debugging RBAC Errors

**When you see 403 Forbidden**:
```bash
# Check if ServiceAccount can perform operation
kubectl auth can-i get secrets \
  --as=system:serviceaccount:nebula-prod:nebula-app \
  -n nebula-prod

# Check RoleBindings
kubectl get rolebinding -n nebula-prod -o wide

# Describe specific RoleBinding
kubectl describe rolebinding nebula-secret-reader -n nebula-prod

# Check ServiceAccount
kubectl get serviceaccount nebula-app -n nebula-prod -o yaml

# View effective permissions
kubectl describe role secret-reader -n nebula-prod
```

### 5.6 Best Practices

1. **Enrich Context**: Add namespace, service account, and operation to error messages
2. **Suggest Remediation**: Include kubectl commands for debugging RBAC issues
3. **Log Errors**: Use structured logging with tracing/log crates
4. **Don't Retry 403/404**: These are permanent errors (except during RBAC updates)
5. **Retry 429/503**: Temporary errors that should use exponential backoff
6. **Monitor Errors**: Track error rates per code in metrics

---

## 6. Security

### 6.1 Service Account Token Authentication

**Default Mechanism** (Kubernetes 1.24+):
- **BoundServiceAccountToken**: Short-lived tokens with pod binding
- **Token Location**: `/var/run/secrets/kubernetes.io/serviceaccount/token`
- **Auto-Refresh**: kubelet rotates tokens before expiration
- **Expiration**: Default 1 hour (configurable)

**Legacy Tokens** (pre-1.24):
- Long-lived tokens stored as Secrets
- Deprecated and removed in 1.29+ (after 1 year of inactivity)

**Token Projection** (Recommended):
```yaml
apiVersion: v1
kind: Pod
metadata:
  name: nebula-app
spec:
  serviceAccountName: nebula-app
  containers:
  - name: app
    image: nebula-app:latest
    volumeMounts:
    - name: token
      mountPath: /var/run/secrets/tokens
      readOnly: true
  volumes:
  - name: token
    projected:
      sources:
      - serviceAccountToken:
          path: token
          expirationSeconds: 3600  # 1 hour
          audience: kubernetes.default.svc
```

### 6.2 Pod-Mounted Secrets

**Best Practice: Mount as Files**:
```yaml
spec:
  containers:
  - name: app
    volumeMounts:
    - name: credentials
      mountPath: /etc/nebula/credentials
      readOnly: true
  volumes:
  - name: credentials
    secret:
      secretName: nebula-credentials
      defaultMode: 0400  # Read-only for owner
```

**Avoid: Environment Variables**:
```yaml
# ❌ NOT RECOMMENDED: Visible in kubectl describe, process list
env:
- name: SECRET_KEY
  valueFrom:
    secretKeyRef:
      name: nebula-credentials
      key: secret_key
```

### 6.3 Encryption at Rest

**etcd Encryption**:
- By default, Secrets are stored in etcd as **base64-encoded plaintext**
- Encryption at rest protects against etcd compromise
- Requires KMS provider configuration

**AWS EKS**:
```bash
aws eks update-cluster-config \
  --region us-east-1 \
  --name nebula-cluster \
  --encryption-config \
    resources=secrets,provider=kms,kmsKeyId=arn:aws:kms:us-east-1:123456789012:key/12345678
```

**Azure AKS**:
```bash
az aks update \
  --resource-group nebula-rg \
  --name nebula-cluster \
  --enable-azure-keyvault-kms \
  --azure-keyvault-kms-key-id https://nebula-kv.vault.azure.net/keys/aks-encryption-key
```

**Self-Managed Kubernetes**:
```yaml
# /etc/kubernetes/encryption-config.yaml
apiVersion: apiserver.config.k8s.io/v1
kind: EncryptionConfiguration
resources:
- resources:
  - secrets
  providers:
  - kms:
      name: kms-provider
      endpoint: unix:///var/run/kmsplugin/socket.sock
      cachesize: 1000
      timeout: 3s
  - identity: {}  # Fallback to plaintext for emergency
```

### 6.4 Secret Types

**Opaque** (Default):
```yaml
apiVersion: v1
kind: Secret
metadata:
  name: nebula-credentials
type: Opaque
data:
  credential.json: <base64-encoded>
```

**kubernetes.io/service-account-token**:
```yaml
apiVersion: v1
kind: Secret
metadata:
  name: nebula-sa-token
  annotations:
    kubernetes.io/service-account.name: nebula-app
type: kubernetes.io/service-account-token
```

**Other Types**:
- `kubernetes.io/dockerconfigjson`: Docker registry credentials
- `kubernetes.io/tls`: TLS certificate and private key
- `kubernetes.io/basic-auth`: Username and password
- `kubernetes.io/ssh-auth`: SSH private key

**Recommendation**: Use `Opaque` for nebula-credential storage

### 6.5 Security Best Practices

1. **Enable Encryption at Rest**: Always use KMS provider in production
2. **Use ServiceAccounts**: Never use personal credentials in pods
3. **Mount as Files**: Avoid environment variables (visible in logs/process list)
4. **Restrict File Permissions**: Use `defaultMode: 0400` for secret volumes
5. **Rotate Tokens**: Use short-lived tokens with automatic rotation
6. **Audit Logging**: Enable audit logs for all secret access
7. **Namespace Isolation**: Use NetworkPolicies to prevent cross-namespace access
8. **Least Privilege RBAC**: Grant only required verbs on specific secrets
9. **External Secret Stores**: Consider AWS Secrets Manager, Azure Key Vault, or HashiCorp Vault with External Secrets Operator
10. **Sealed Secrets**: Use for GitOps workflows to encrypt secrets in Git

---

## 7. Performance

### 7.1 Typical Latency

**API Server Response Times**:
- **GET**: 5-50ms (single secret)
- **LIST**: 50-500ms (depends on count, use pagination)
- **WATCH**: 10-100ms initial, then streaming updates
- **CREATE/UPDATE/DELETE**: 10-100ms

**Factors Affecting Latency**:
- API server load
- etcd performance
- Network latency (in-cluster vs remote)
- Secret size
- Number of secrets in namespace

### 7.2 API Server Rate Limits

**Default Limits** (API Priority and Fairness):
- **Global**: ~400 QPS per API server
- **Per-User/ServiceAccount**: ~100 QPS (configurable)
- **Burst**: ~200 requests (configurable)

**Rate Limit Signals**:
- HTTP 429: Rate limit exceeded
- `Retry-After` header: Seconds to wait
- `X-Rate-Limit-*` headers: Limit details

### 7.3 Watch vs List-Then-Watch

**Watch Pattern** (Efficient):
```rust
// Efficient: Single initial LIST, then streaming updates
let stream = watcher(secrets, watcher::Config::default())
    .default_backoff()
    .applied_objects();

// Consumes 1 seat during initial burst, then streams
```

**Poll Pattern** (Inefficient):
```rust
// ❌ Inefficient: Repeated LIST requests
loop {
    let secrets = api.list(&ListParams::default()).await?;
    // Process secrets
    tokio::time::sleep(Duration::from_secs(60)).await;
}
// Consumes multiple seats per LIST, wastes bandwidth
```

**Best Practice**: Use `watcher()` for continuous monitoring, `list()` only for one-time queries.

### 7.4 Pagination

**For Large Namespaces**:
```rust
use kube::api::{ListParams, ObjectList};
use k8s_openapi::api::core::v1::Secret;

let mut list_params = ListParams::default()
    .labels("app=nebula-credential")
    .limit(100);  // Page size

loop {
    let secret_list: ObjectList<Secret> = secrets.list(&list_params).await?;
    
    for secret in &secret_list.items {
        // Process secret
    }
    
    // Check for next page
    match secret_list.metadata.continue_ {
        Some(continue_token) => {
            list_params = list_params.continue_token(&continue_token);
        }
        None => break,  // No more pages
    }
}
```

### 7.5 Caching

**Client-Side Caching**:
```rust
use moka::future::Cache;
use std::time::Duration;

let cache: Cache<CredentialId, Credential> = Cache::builder()
    .max_capacity(1000)
    .time_to_live(Duration::from_secs(300))  // 5 minutes
    .time_to_idle(Duration::from_secs(60))   // 1 minute idle
    .build();

// Check cache first
if let Some(credential) = cache.get(&credential_id).await {
    return Ok(credential);
}

// Cache miss, fetch from Kubernetes
let secret = secrets.get(&secret_name).await?;
let credential = parse_credential(&secret)?;
cache.insert(credential_id.clone(), credential.clone()).await;

Ok(credential)
```

### 7.6 Connection Pooling

**Built-In**: kube-rs client automatically pools connections via hyper

```rust
// Single client instance reused across operations
let client = Client::try_default().await?;
let provider = Arc::new(KubernetesSecretsProvider::with_client(
    client,
    "nebula-prod".into(),
    "nebula-credentials".into(),
));

// Share provider across tasks
let provider_clone = Arc::clone(&provider);
tokio::spawn(async move {
    provider_clone.retrieve(&credential_id).await
});
```

### 7.7 Performance Best Practices

1. **Reuse Client**: Create one client, share via Arc
2. **Use Watchers**: Prefer watch over repeated list
3. **Enable Pagination**: Limit LIST results to 100-500 per page
4. **Cache Aggressively**: Cache secrets for 5+ minutes
5. **Batch Operations**: Use `list()` for bulk retrieval
6. **Monitor Latency**: Track p50, p95, p99 latencies
7. **Optimize Labels**: Index on frequently queried labels

---

## 8. Size Limits

### 8.1 Maximum Secret Size

**Hard Limit**: 1MB (1,048,576 bytes)

**Reason**: etcd is optimized for small key-value pairs (metadata), not large blobs

**Enforcement**: API server rejects secrets exceeding 1MB with 422 Unprocessable Entity

### 8.2 Base64 Encoding Overhead

**Encoding Overhead**: ~33% increase

**Example**:
- Raw JSON: 750 KB
- Base64-encoded: ~1000 KB (1MB limit reached)

**Calculation**:
```rust
let raw_bytes = credential_json.as_bytes();
let base64_bytes = base64::encode(&raw_bytes).as_bytes();
let overhead = (base64_bytes.len() as f64 / raw_bytes.len() as f64) - 1.0;
println!("Base64 overhead: {:.1}%", overhead * 100.0);  // ~33.3%
```

### 8.3 Practical Limits

**Recommended Maximum**:
- **Per Secret**: 256 KB raw (343 KB base64)
- **Per Namespace**: 10,000 secrets (use quotas)

**Quota Configuration**:
```yaml
apiVersion: v1
kind: ResourceQuota
metadata:
  name: secret-quota
  namespace: nebula-prod
spec:
  hard:
    secrets: "100"  # Max 100 secrets
    # Note: No direct size quota, only count
```

### 8.4 Handling Large Credentials

**For Credentials > 256KB**:
1. **Split into Multiple Secrets**: Store parts separately
2. **Use External Storage**: Store in S3/Azure Blob, reference in secret
3. **Compression**: gzip before base64 encoding
4. **ConfigMaps**: If not sensitive, use ConfigMap (also 1MB limit)

**Example: Split Credential**:
```rust
const CHUNK_SIZE: usize = 200_000; // 200KB chunks

let credential_json = serde_json::to_string(&credential)?;
let chunks: Vec<&str> = credential_json
    .as_bytes()
    .chunks(CHUNK_SIZE)
    .map(|chunk| std::str::from_utf8(chunk).unwrap())
    .collect();

for (i, chunk) in chunks.iter().enumerate() {
    let secret_name = format!("{}-part-{}", base_name, i);
    // Create secret for chunk
}
```

### 8.5 Size Validation

```rust
fn validate_secret_size(data: &[u8]) -> Result<(), StorageError> {
    const MAX_SIZE: usize = 1_048_576; // 1MB
    const RECOMMENDED_MAX: usize = 262_144; // 256KB
    
    let base64_size = (data.len() * 4 + 2) / 3; // Base64 size estimate
    
    if base64_size > MAX_SIZE {
        return Err(StorageError::SecretTooLarge(
            format!("Secret size {} exceeds 1MB limit", base64_size)
        ));
    }
    
    if base64_size > RECOMMENDED_MAX {
        tracing::warn!(
            size = base64_size,
            "Secret size exceeds recommended 256KB limit"
        );
    }
    
    Ok(())
}
```

### 8.6 Best Practices

1. **Keep Secrets Small**: Aim for < 64KB per secret
2. **Monitor Size**: Track secret size distribution
3. **Enforce Quotas**: Limit secrets per namespace
4. **Use External Stores**: For large credentials (certificates, keys)
5. **Compress**: Use gzip for large JSON payloads
6. **Validate**: Reject credentials approaching 1MB limit

---

## 9. Labels and Annotations

### 9.1 Labels vs Annotations

**Labels** (for selection and filtering):
- Max key length: 63 characters
- Max value length: 63 characters
- Alphanumeric, `-`, `_`, `.` only
- Used in selectors (LIST, watch)
- Indexed by Kubernetes for fast queries

**Annotations** (for metadata):
- Max key length: 253 characters (prefix) + 63 characters (name)
- Max value length: 256 KB
- Any characters allowed
- Not used in selectors
- Not indexed (slower queries)

### 9.2 Recommended Labels for Secrets

**Standard Labels**:
```yaml
metadata:
  labels:
    # Application
    app: nebula-credential
    app.kubernetes.io/name: nebula
    app.kubernetes.io/component: credential-manager
    app.kubernetes.io/version: "0.1.0"
    app.kubernetes.io/managed-by: nebula-credential
    
    # Credential-specific
    credential-type: oauth2  # oauth2, api-key, certificate, etc
    credential-scope: user   # user, workspace, organization, global
    owner-id: alice          # User/workspace ID
    environment: production  # dev, staging, production
    
    # Rotation
    rotation-policy: enabled
    last-rotated: "2026-02-03"
```

### 9.3 Recommended Annotations

**Standard Annotations**:
```yaml
metadata:
  annotations:
    # Provenance
    nebula.io/created-by: "system:serviceaccount:nebula-prod:nebula-app"
    nebula.io/created-at: "2026-02-03T10:30:00Z"
    nebula.io/description: "GitHub OAuth2 credentials for alice"
    
    # Rotation
    nebula.io/rotation-policy: "30d"
    nebula.io/next-rotation: "2026-03-03T10:30:00Z"
    nebula.io/rotation-history: '[{"rotated_at": "2026-01-03T10:30:00Z", "reason": "scheduled"}]'
    
    # Encryption
    nebula.io/encrypted-with: "aws-kms"
    nebula.io/kms-key-id: "arn:aws:kms:us-east-1:123456789012:key/12345678"
    
    # External sync
    external-secrets.io/source: "aws-secrets-manager"
    external-secrets.io/path: "nebula/credentials/alice/github-oauth2"
```

### 9.4 Filtering by Labels

```rust
use kube::api::ListParams;

// Filter by app and credential type
let list_params = ListParams::default()
    .labels("app=nebula-credential,credential-type=oauth2");

let secrets = api.list(&list_params).await?;

// Filter by owner
let list_params = ListParams::default()
    .labels(&format!("app=nebula-credential,owner-id={}", user_id));

let user_secrets = api.list(&list_params).await?;

// Filter by environment
let list_params = ListParams::default()
    .labels("environment=production");

let prod_secrets = api.list(&list_params).await?;
```

### 9.5 Label Selectors

**Equality-Based**:
```rust
// Single label
.labels("app=nebula-credential")

// Multiple labels (AND)
.labels("app=nebula-credential,environment=production")

// Negation
.labels("app=nebula-credential,environment!=dev")
```

**Set-Based**:
```rust
// IN
.labels("environment in (staging, production)")

// NOT IN
.labels("environment notin (dev)")

// EXISTS
.labels("rotation-policy")

// NOT EXISTS
.labels("!manual-rotation")
```

### 9.6 Well-Known Labels

**Kubernetes Standard Labels** (kubernetes.io prefix):
```yaml
app.kubernetes.io/name: nebula
app.kubernetes.io/instance: nebula-prod-001
app.kubernetes.io/version: "0.1.0"
app.kubernetes.io/component: credential-manager
app.kubernetes.io/part-of: nebula-platform
app.kubernetes.io/managed-by: nebula-credential
```

### 9.7 Best Practices

1. **Use Labels for Queries**: Always filter LIST operations
2. **Use Annotations for Metadata**: Store large or non-queryable data
3. **Keep Labels Short**: < 20 characters for values
4. **Use Prefixes**: `nebula.io/` for custom labels/annotations
5. **Index on Labels**: Kubernetes indexes labels for fast queries
6. **Avoid PII in Labels**: Labels are visible in logs and audit trails
7. **Document Schema**: Maintain label/annotation schema in docs
8. **Validate Labels**: Enforce label requirements in CI/CD

---

## 10. Implementation Recommendations

### 10.1 Project Structure

```
nebula-credential/
├── src/
│   ├── storage/
│   │   ├── mod.rs
│   │   ├── kubernetes/
│   │   │   ├── mod.rs
│   │   │   ├── provider.rs      # KubernetesSecretsProvider
│   │   │   ├── error.rs          # KubernetesError
│   │   │   ├── config.rs         # Configuration
│   │   │   └── cache.rs          # Optional caching layer
│   │   ├── local.rs
│   │   └── trait.rs              # StorageProvider trait
```

### 10.2 Configuration

```rust
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct KubernetesConfig {
    /// Kubernetes namespace
    pub namespace: String,
    
    /// Secret name prefix (e.g., "nebula-credentials")
    pub secret_prefix: String,
    
    /// Enable caching
    #[serde(default)]
    pub cache_enabled: bool,
    
    /// Cache TTL (seconds)
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_secs: u64,
    
    /// Cache capacity
    #[serde(default = "default_cache_capacity")]
    pub cache_capacity: usize,
    
    /// Request timeout (seconds)
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    
    /// Retry attempts
    #[serde(default = "default_retry_attempts")]
    pub retry_attempts: usize,
    
    /// Retry backoff (milliseconds)
    #[serde(default = "default_retry_backoff")]
    pub retry_backoff_ms: u64,
}

fn default_cache_ttl() -> u64 { 300 }
fn default_cache_capacity() -> usize { 1000 }
fn default_timeout() -> u64 { 10 }
fn default_retry_attempts() -> usize { 3 }
fn default_retry_backoff() -> u64 { 200 }

impl Default for KubernetesConfig {
    fn default() -> Self {
        Self {
            namespace: "default".to_string(),
            secret_prefix: "nebula-credentials".to_string(),
            cache_enabled: true,
            cache_ttl_secs: default_cache_ttl(),
            cache_capacity: default_cache_capacity(),
            timeout_secs: default_timeout(),
            retry_attempts: default_retry_attempts(),
            retry_backoff_ms: default_retry_backoff(),
        }
    }
}
```

### 10.3 Error Handling

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum KubernetesError {
    #[error("Kubernetes API error: {0}")]
    Api(#[from] kube::Error),
    
    #[error("Secret not found: {namespace}/{name}")]
    NotFound { namespace: String, name: String },
    
    #[error("Permission denied: {operation} on {resource} in {namespace}. Check RBAC with: kubectl auth can-i {operation} {resource} --as=system:serviceaccount:{namespace}:{service_account}")]
    PermissionDenied {
        operation: String,
        resource: String,
        namespace: String,
        service_account: String,
    },
    
    #[error("Secret already exists: {namespace}/{name}")]
    AlreadyExists { namespace: String, name: String },
    
    #[error("Secret too large: {size} bytes (max: 1048576)")]
    SecretTooLarge { size: usize },
    
    #[error("Rate limited: retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("Configuration error: {0}")]
    Config(String),
    
    #[error("Timeout after {timeout_secs}s")]
    Timeout { timeout_secs: u64 },
}
```

### 10.4 Testing Strategy

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::Secret;
    use kube::Client;
    
    #[tokio::test]
    async fn test_in_cluster() {
        // Skip if not in cluster
        if Client::try_default().await.is_err() {
            return;
        }
        
        let provider = KubernetesSecretsProvider::new(
            "test".to_string(),
            "test-credentials".to_string(),
        ).await.unwrap();
        
        // Test CRUD operations
    }
    
    #[tokio::test]
    async fn test_with_mock_client() {
        // Use kube test utils for mocking
        // TODO: Implement mock client tests
    }
}
```

### 10.5 Monitoring and Observability

```rust
use prometheus::{IntCounterVec, HistogramVec, Registry};
use tracing::{info, warn, error, debug};

// Metrics
lazy_static::lazy_static! {
    static ref K8S_OPS_TOTAL: IntCounterVec = IntCounterVec::new(
        prometheus::opts!("nebula_k8s_operations_total", "Total K8s operations"),
        &["operation", "status", "namespace"]
    ).unwrap();
    
    static ref K8S_DURATION: HistogramVec = HistogramVec::new(
        prometheus::histogram_opts!(
            "nebula_k8s_operation_duration_seconds",
            "K8s operation duration"
        ),
        &["operation", "namespace"]
    ).unwrap();
    
    static ref K8S_ERRORS: IntCounterVec = IntCounterVec::new(
        prometheus::opts!("nebula_k8s_errors_total", "Total K8s errors"),
        &["error_type", "namespace"]
    ).unwrap();
}

// Instrumentation
impl KubernetesSecretsProvider {
    async fn store_with_metrics(&self, id: CredentialId, credential: &Credential) 
        -> Result<(), KubernetesError> 
    {
        let timer = K8S_DURATION
            .with_label_values(&["store", &self.namespace])
            .start_timer();
        
        let result = self.store_impl(id, credential).await;
        
        let status = if result.is_ok() { "success" } else { "error" };
        K8S_OPS_TOTAL
            .with_label_values(&["store", status, &self.namespace])
            .inc();
        
        if let Err(ref e) = result {
            K8S_ERRORS
                .with_label_values(&[error_type(e), &self.namespace])
                .inc();
            error!(
                namespace = %self.namespace,
                error = %e,
                "Failed to store credential"
            );
        } else {
            info!(
                namespace = %self.namespace,
                credential_id = %id,
                "Stored credential"
            );
        }
        
        timer.observe_duration();
        result
    }
}

fn error_type(err: &KubernetesError) -> &'static str {
    match err {
        KubernetesError::NotFound { .. } => "not_found",
        KubernetesError::PermissionDenied { .. } => "permission_denied",
        KubernetesError::AlreadyExists { .. } => "already_exists",
        KubernetesError::RateLimited { .. } => "rate_limited",
        KubernetesError::SecretTooLarge { .. } => "secret_too_large",
        _ => "other",
    }
}
```

### 10.6 Feature Flags

```toml
# Cargo.toml
[features]
default = ["kubernetes"]
kubernetes = ["kube", "k8s-openapi"]
kubernetes-watch = ["kubernetes", "kube/runtime"]
kubernetes-cache = ["kubernetes", "moka"]
```

### 10.7 Migration Checklist

- [ ] Add dependencies (kube, k8s-openapi, tokio)
- [ ] Implement KubernetesSecretsProvider
- [ ] Add error handling with RBAC context
- [ ] Implement retry with exponential backoff
- [ ] Add caching layer
- [ ] Create RBAC manifests (Role, RoleBinding)
- [ ] Add integration tests
- [ ] Document deployment (ServiceAccount, RBAC)
- [ ] Add Prometheus metrics
- [ ] Add tracing instrumentation
- [ ] Update documentation
- [ ] Create migration guide from local storage

---

## References

### Documentation
- [Kubernetes Secrets](https://kubernetes.io/docs/concepts/configuration/secret/)
- [Kubernetes RBAC](https://kubernetes.io/docs/reference/access-authn-authz/rbac/)
- [API Priority and Fairness](https://kubernetes.io/docs/concepts/cluster-administration/flow-control/)
- [Encrypting Secrets at Rest](https://kubernetes.io/docs/tasks/administer-cluster/encrypt-data/)
- [kube-rs Documentation](https://docs.rs/kube/latest/kube/)
- [kube-rs GitHub](https://github.com/kube-rs/kube)
- [kube.rs Website](https://kube.rs/)
- [External Secrets Operator](https://external-secrets.io/)
- [Sealed Secrets](https://github.com/bitnami-labs/sealed-secrets)

### Blog Posts and Guides
- [Kubernetes Namespace Isolation](https://www.synacktiv.com/en/publications/kubernetes-namespaces-isolation-what-it-is-what-it-isnt-life-universe-and-everything)
- [Kubernetes RBAC Best Practices - ARMO](https://www.armosec.io/blog/a-guide-for-using-kubernetes-rbac/)
- [Kubernetes Management with Rust](https://blog.kubesimplify.com/kubernetes-management-with-rust-a-dive-into-generic-client-go-controller-abstractions-and-crd-macros-with-kubers)
- [Building Resilient Kubernetes Controllers](https://medium.com/@vamshitejanizam/building-resilient-kubernetes-controllers-a-practical-guide-to-retry-mechanisms-0d689160fa51)
- [Secrets Management Best Practices - GitGuardian](https://blog.gitguardian.com/how-to-handle-secrets-in-kubernetes/)
- [Kubernetes API Performance Metrics](https://www.redhat.com/en/blog/kubernetes-api-performance-metrics-examples-and-best-practices)
- [Kubernetes Labels Best Practices](https://komodor.com/blog/best-practices-guide-for-kubernetes-labels-and-annotations/)

### Tools
- [kubectl](https://kubernetes.io/docs/reference/kubectl/)
- [kubeseal](https://github.com/bitnami-labs/sealed-secrets)
- [Helm](https://helm.sh/)

---

## Appendix A: Quick Reference

### Common kubectl Commands

```bash
# Create secret
kubectl create secret generic my-secret \
  --from-literal=key=value \
  -n namespace

# Get secret
kubectl get secret my-secret -n namespace

# Describe secret (shows metadata, not data)
kubectl describe secret my-secret -n namespace

# Get secret data (base64-encoded)
kubectl get secret my-secret -n namespace -o jsonpath='{.data.key}'

# Decode secret data
kubectl get secret my-secret -n namespace -o jsonpath='{.data.key}' | base64 -d

# List secrets with labels
kubectl get secrets -n namespace -l app=nebula-credential

# Delete secret
kubectl delete secret my-secret -n namespace

# Check RBAC permissions
kubectl auth can-i get secrets \
  --as=system:serviceaccount:namespace:serviceaccount \
  -n namespace
```

### Common kube-rs Patterns

```rust
// Create client
let client = Client::try_default().await?;

// Create API
let secrets: Api<Secret> = Api::namespaced(client, "namespace");

// Get secret
let secret = secrets.get("name").await?;

// List secrets
let list = secrets.list(&ListParams::default().labels("app=nebula")).await?;

// Create secret
secrets.create(&PostParams::default(), &secret).await?;

// Update secret
secrets.replace("name", &PostParams::default(), &secret).await?;

// Delete secret
secrets.delete("name", &DeleteParams::default()).await?;

// Watch secrets
watcher(secrets, watcher::Config::default())
    .default_backoff()
    .applied_objects()
    .try_for_each(|s| async move { Ok(()) })
    .await?;
```

---

## Appendix B: Example RBAC Manifests

### Minimal Read-Only

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: nebula-app
  namespace: nebula-prod
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: secret-reader
  namespace: nebula-prod
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list"]
  resourceNames: ["nebula-credentials-*"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: nebula-secret-reader
  namespace: nebula-prod
subjects:
- kind: ServiceAccount
  name: nebula-app
  namespace: nebula-prod
roleRef:
  kind: Role
  name: secret-reader
  apiGroup: rbac.authorization.k8s.io
```

### Full Management

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: nebula-manager
  namespace: nebula-prod
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: secret-manager
  namespace: nebula-prod
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: nebula-secret-manager
  namespace: nebula-prod
subjects:
- kind: ServiceAccount
  name: nebula-manager
  namespace: nebula-prod
roleRef:
  kind: Role
  name: secret-manager
  apiGroup: rbac.authorization.k8s.io
```

---

**End of Research Document**

---

## Sources

This research compiled information from the following sources:

- [GitHub - kube-rs/kube: Rust Kubernetes client and controller runtime](https://github.com/kube-rs/kube)
- [kube - crates.io: Rust Package Registry](https://crates.io/crates/kube)
- [Crate kube - Rust](https://docs.rs/kube/latest/kube/)
- [kube.rs Official Website](https://kube.rs/)
- [Using RBAC Authorization | Kubernetes](https://kubernetes.io/docs/reference/access-authn-authz/rbac/)
- [Kubernetes RBAC: the Complete Best Practices Guide - ARMO](https://www.armosec.io/blog/a-guide-for-using-kubernetes-rbac/)
- [Client in kube - Rust](https://docs.rs/kube/latest/kube/struct.Client.html)
- [Config in kube_client - Rust](https://docs.rs/kube-client/latest/kube_client/struct.Config.html)
- [API Priority and Fairness | Kubernetes](https://kubernetes.io/docs/concepts/cluster-administration/flow-control/)
- [Building Resilient Kubernetes Controllers: A Practical Guide to Retry Mechanisms](https://medium.com/@vamshitejanizam/building-resilient-kubernetes-controllers-a-practical-guide-to-retry-mechanisms-0d689160fa51)
- [Kubernetes namespaces isolation - Synacktiv](https://www.synacktiv.com/en/publications/kubernetes-namespaces-isolation-what-it-is-what-it-isnt-life-universe-and-everything)
- [Kubernetes Namespace Management for Secure, Scalable Clusters](https://atmosly.com/blog/kubernetes-namespace-management-best-practices-2025)
- [Encrypting Confidential Data at Rest | Kubernetes](https://kubernetes.io/docs/tasks/administer-cluster/encrypt-data/)
- [Secrets Management in Kubernetes: Best Practices for Security](https://dev.to/rubixkube/secrets-management-in-kubernetes-best-practices-for-security-1df0)
- [Kubernetes Secrets: Best Practices for Secure Management - GitGuardian](https://blog.gitguardian.com/how-to-handle-secrets-in-kubernetes/)
- [Why K8s Secret and ConfigMap are limited to 1MiB in size | by Able Lv](https://able8.medium.com/why-k8s-secret-and-configmap-are-limited-to-1mib-in-size-ba79d86b0372)
- [Secrets | Kubernetes](https://kubernetes.io/docs/concepts/configuration/secret/)
- [Error in kube - Rust](https://docs.rs/kube/latest/kube/enum.Error.html)
- [Troubleshooting - kube](https://kube.rs/troubleshooting/)
- [Kubernetes API Performance Metrics: Examples and Best Practices](https://www.redhat.com/en/blog/kubernetes-api-performance-metrics-examples-and-best-practices)
- [Best Practices Guide for Kubernetes Labels and Annotations](https://komodor.com/blog/best-practices-guide-for-kubernetes-labels-and-annotations/)
- [Annotations | Kubernetes](https://kubernetes.io/docs/concepts/overview/working-with-objects/annotations/)
- [watcher in kube::runtime - Rust](https://docs.rs/kube/latest/kube/runtime/fn.watcher.html)
- [default_backoff in kube::runtime::watcher - Rust](https://docs.rs/kube/0.66.0/kube/runtime/watcher/fn.default_backoff.html)
- [The Ultimate Guide to Kubernetes Secrets: Types, Creation, and Management - Cycode](https://cycode.com/blog/the-ultimate-guide-to-kubernetes-secrets-types-creation-and-management/)
- [Api in kube - Rust](https://docs.rs/kube/latest/kube/struct.Api.html)
