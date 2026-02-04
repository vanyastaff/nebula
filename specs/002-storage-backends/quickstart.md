# Quickstart Guide: Storage Backend Selection

**Feature**: Production-Ready Storage Backends (002-storage-backends)  
**Audience**: Developers integrating nebula-credential into applications  
**Time**: 10-15 minutes per provider  

This guide helps you choose and configure the right storage backend for your use case.

---

## Decision Tree: Which Provider Should I Use?

```
Start Here
    │
    ├─ Deploying to AWS? ──────────► AWS Secrets Manager
    │
    ├─ Deploying to Azure? ────────► Azure Key Vault
    │
    ├─ Using HashiCorp Vault? ─────► HashiCorp Vault Provider
    │
    ├─ Running on Kubernetes? ─────► Kubernetes Secrets
    │
    └─ Local development/testing? ─► Local Storage
       Or on-premise deployment
       without cloud providers
```

---

## 1. Local Storage (Quick Start)

**Best For**: Local development, testing, on-premise deployments without cloud providers

### Prerequisites
- Writable filesystem
- No external dependencies

### Setup (30 seconds)

```rust
use nebula_credential::prelude::*;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure local storage
    let config = LocalStorageConfig {
        base_path: PathBuf::from("~/.nebula/credentials"), // Auto-expanded
        create_dir: true,  // Automatically create directory
        ..Default::default()
    };
    
    // Initialize provider
    let provider = LocalStorageProvider::new(config).await?;
    
    println!("Local storage ready at ~/.nebula/credentials");
    Ok(())
}
```

### Store and Retrieve

```rust
// Store credential
let id = CredentialId::new("database_password")?;
let plaintext = b"super_secret_password";

// Encrypt (using EncryptionKey from Phase 1)
let key = EncryptionKey::derive_from_password("master_password", &salt)?;
let encrypted_data = encrypt(&key, plaintext)?;

let metadata = CredentialMetadata {
    created_at: Utc::now(),
    tags: vec!["environment:dev".into()],
    ..Default::default()
};

let context = CredentialContext::new("user_123");
provider.store(&id, encrypted_data, metadata, &context).await?;

// Retrieve credential
let (retrieved_data, retrieved_metadata) = provider.retrieve(&id, &context).await?;
let decrypted = decrypt(&key, &retrieved_data)?;

println!("Retrieved password: {}", String::from_utf8(decrypted)?);
```

### File Structure

```
~/.nebula/credentials/
├── database_password.enc.json    (File permissions: 0600)
├── github_token.enc.json
└── api_key.enc.json

# Each file contains:
{
  "version": 1,
  "encrypted_data": {
    "nonce": [12 bytes base64],
    "ciphertext": [encrypted bytes base64],
    "tag": [16 bytes base64]
  },
  "metadata": {
    "created_at": "2026-02-03T14:30:00Z",
    "tags": ["environment:dev"]
  }
}
```

---

## 2. AWS Secrets Manager (Quick Start)

**Best For**: AWS deployments, teams already using AWS infrastructure

### Prerequisites
- AWS account with Secrets Manager enabled
- IAM credentials (environment variables, instance profile, or ECS task role)
- `secretsmanager:GetSecretValue`, `secretsmanager:CreateSecret` permissions

### Setup (2 minutes)

**Step 1**: Configure AWS Credentials

```bash
# Option A: Environment variables
export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
export AWS_REGION=us-east-1

# Option B: Use IAM instance profile (EC2/ECS) - nothing to configure!
```

**Step 2**: Initialize Provider

```rust
use nebula_credential::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AwsSecretsManagerConfig {
        region: None, // Auto-detect from environment
        secret_prefix: "nebula/credentials/".into(),
        ..Default::default()
    };
    
    let provider = AwsSecretsManagerProvider::new(config).await?;
    println!("AWS Secrets Manager ready");
    Ok(())
}
```

### Store and Retrieve

```rust
// Store credential (automatically uses KMS for encryption)
let id = CredentialId::new("database_password")?;
let encrypted_data = encrypt(&key, b"super_secret_password")?;

let metadata = CredentialMetadata {
    created_at: Utc::now(),
    tags: vec!["environment:production".into()],
    ..Default::default()
};

provider.store(&id, encrypted_data, metadata, &context).await?;
// ↑ Creates secret: nebula/credentials/database_password
// ↑ Encrypted with AWS KMS, tagged with environment:production

// Retrieve credential
let (data, metadata) = provider.retrieve(&id, &context).await?;
// ↑ Automatically decrypts using AWS KMS
```

### IAM Policy (Minimum Permissions)

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "secretsmanager:GetSecretValue",
        "secretsmanager:CreateSecret",
        "secretsmanager:UpdateSecret",
        "secretsmanager:TagResource"
      ],
      "Resource": "arn:aws:secretsmanager:*:*:secret:nebula/credentials/*"
    },
    {
      "Effect": "Allow",
      "Action": "kms:Decrypt",
      "Resource": "*",
      "Condition": {
        "StringEquals": {
          "kms:ViaService": "secretsmanager.us-east-1.amazonaws.com"
        }
      }
    }
  ]
}
```

---

## 3. Azure Key Vault (Quick Start)

**Best For**: Azure deployments, teams using Azure managed services

### Prerequisites
- Azure subscription with Key Vault created
- Azure CLI installed (`az login` for local dev)
- Managed Identity (production) or Service Principal (CI/CD)
- "Key Vault Secrets User" or "Key Vault Secrets Officer" RBAC role

### Setup (2 minutes)

**Step 1**: Create Key Vault (Azure Portal or CLI)

```bash
az keyvault create \
  --name my-credential-vault \
  --resource-group my-resource-group \
  --location eastus
```

**Step 2**: Grant RBAC Permissions

```bash
# For read-only access
az role assignment create \
  --role "Key Vault Secrets User" \
  --assignee <user-or-managed-identity-object-id> \
  --scope /subscriptions/<sub-id>/resourceGroups/<rg>/providers/Microsoft.KeyVault/vaults/my-credential-vault

# For full access (create, update, delete)
az role assignment create \
  --role "Key Vault Secrets Officer" \
  --assignee <object-id> \
  --scope <vault-resource-id>
```

**Step 3**: Initialize Provider

```rust
use nebula_credential::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = AzureKeyVaultConfig {
        vault_url: "https://my-credential-vault.vault.azure.net/".into(),
        credential_type: AzureCredentialType::ManagedIdentity {
            client_id: None, // System-assigned
        },
        ..Default::default()
    };
    
    let provider = AzureKeyVaultProvider::new(config).await?;
    println!("Azure Key Vault ready");
    Ok(())
}
```

### Store and Retrieve

```rust
// Store credential (HSM-backed encryption)
provider.store(&id, encrypted_data, metadata, &context).await?;
// ↑ Stored in: https://my-credential-vault.vault.azure.net/secrets/nebula-credentials-database-password

// Retrieve credential (automatic token refresh)
let (data, metadata) = provider.retrieve(&id, &context).await?;
```

### Local Development Setup

```rust
// Use Azure CLI credentials for local development
let config = AzureKeyVaultConfig {
    vault_url: "https://my-credential-vault.vault.azure.net/".into(),
    credential_type: AzureCredentialType::DeveloperTools,
    ..Default::default()
};

// Requires: az login
```

---

## 4. HashiCorp Vault (Quick Start)

**Best For**: Multi-cloud, hybrid deployments, teams already using Vault

### Prerequisites
- Vault server running (OSS or Enterprise)
- Token with KV v2 read/write permissions
- KV v2 engine mounted (default: `secret/`)

### Setup (3 minutes)

**Step 1**: Start Vault (Dev Mode for testing)

```bash
vault server -dev -dev-root-token-id="dev-root-token"

# In another terminal:
export VAULT_ADDR='http://127.0.0.1:8200'
export VAULT_TOKEN='dev-root-token'
```

**Step 2**: Create Policy

```hcl
# nebula-credential-policy.hcl
path "secret/data/nebula/credentials/*" {
  capabilities = ["create", "read", "update", "delete"]
}

path "secret/metadata/nebula/credentials/*" {
  capabilities = ["read", "list", "delete"]
}

path "auth/token/renew-self" {
  capabilities = ["update"]
}
```

```bash
vault policy write nebula-credential nebula-credential-policy.hcl
```

**Step 3**: Initialize Provider

```rust
use nebula_credential::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = VaultConfig {
        address: "http://127.0.0.1:8200".into(),
        auth_method: VaultAuthMethod::Token {
            token: SecretString::new("dev-root-token".into()),
        },
        mount_path: "secret".into(),
        path_prefix: "nebula/credentials".into(),
        ..Default::default()
    };
    
    let provider = HashiCorpVaultProvider::new(config).await?;
    println!("HashiCorp Vault ready");
    Ok(())
}
```

### Store and Retrieve (with Versioning)

```rust
// Store credential (creates version 1)
provider.store(&id, encrypted_data, metadata, &context).await?;
// ↑ Stored at: secret/data/nebula/credentials/database_password
// ↑ Version: 1

// Update credential (creates version 2, keeps version 1)
provider.store(&id, new_encrypted_data, new_metadata, &context).await?;
// ↑ Version: 2 (version 1 still accessible)

// Retrieve latest version
let (data, metadata) = provider.retrieve(&id, &context).await?;
// ↑ Returns version 2

// List credentials (with versioning metadata)
let ids = provider.list(None, &context).await?;
```

### Production: AppRole Authentication

```rust
let config = VaultConfig {
    address: "https://vault.example.com:8200".into(),
    auth_method: VaultAuthMethod::AppRole {
        role_id: "a1b2c3d4-...".into(),
        secret_id: SecretString::new("e5f6g7h8-...".into()),
        mount_path: "approle".into(),
    },
    tls_verify: true, // MUST be true in production
    ..Default::default()
};
```

---

## 5. Kubernetes Secrets (Quick Start)

**Best For**: Container deployments on Kubernetes, cloud-native applications

### Prerequisites
- Kubernetes cluster (1.21+)
- kubectl configured
- ServiceAccount with Secrets RBAC permissions

### Setup (3 minutes)

**Step 1**: Create RBAC Resources

```yaml
# rbac.yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: nebula-credential-manager
  namespace: nebula
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: nebula-credential-manager
  namespace: nebula
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list", "watch", "create", "update", "patch", "delete"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: nebula-credential-manager
  namespace: nebula
subjects:
- kind: ServiceAccount
  name: nebula-credential-manager
  namespace: nebula
roleRef:
  kind: Role
  name: nebula-credential-manager
  apiGroup: rbac.authorization.k8s.io
```

```bash
kubectl apply -f rbac.yaml
```

**Step 2**: Initialize Provider (In-Cluster)

```rust
use nebula_credential::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = KubernetesSecretsConfig {
        namespace: "nebula".into(),
        kubeconfig_path: None, // Auto-detect (uses service account in pod)
        secret_prefix: "nebula-cred-".into(),
        ..Default::default()
    };
    
    let provider = KubernetesSecretsProvider::new(config).await?;
    println!("Kubernetes Secrets ready in namespace: nebula");
    Ok(())
}
```

### Store and Retrieve

```rust
// Store credential (creates Kubernetes Secret)
provider.store(&id, encrypted_data, metadata, &context).await?;
// ↑ Creates Secret: nebula-cred-database-password in namespace "nebula"
// ↑ Type: Opaque
// ↑ Labels: app.kubernetes.io/name=nebula-credential, environment=production

// Retrieve credential
let (data, metadata) = provider.retrieve(&id, &context).await?;
// ↑ Automatically base64 decodes Secret data field
```

### Deployment Manifest

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: app-using-credentials
  namespace: nebula
spec:
  template:
    spec:
      serviceAccountName: nebula-credential-manager
      containers:
      - name: app
        image: myapp:latest
        env:
        - name: KUBERNETES_NAMESPACE
          value: "nebula"
```

### Local Development (Out-of-Cluster)

```rust
let config = KubernetesSecretsConfig {
    namespace: "nebula".into(),
    kubeconfig_path: Some(PathBuf::from("~/.kube/config")),
    ..Default::default()
};
```

---

## Switching Between Providers

**Key Design Principle**: All providers implement the same `StorageProvider` trait. Switching requires changing only configuration, not application code.

### Example: Environment-Based Provider Selection

```rust
use nebula_credential::prelude::*;

async fn get_storage_provider() -> Result<Box<dyn StorageProvider>, Box<dyn std::error::Error>> {
    let provider_type = std::env::var("STORAGE_PROVIDER").unwrap_or_else(|_| "local".into());
    
    match provider_type.as_str() {
        "local" => {
            let config = LocalStorageConfig::default();
            Ok(Box::new(LocalStorageProvider::new(config).await?))
        }
        "aws" => {
            let config = AwsSecretsManagerConfig::default();
            Ok(Box::new(AwsSecretsManagerProvider::new(config).await?))
        }
        "azure" => {
            let config = AzureKeyVaultConfig {
                vault_url: std::env::var("AZURE_KEYVAULT_URL")?,
                ..Default::default()
            };
            Ok(Box::new(AzureKeyVaultProvider::new(config).await?))
        }
        "vault" => {
            let config = VaultConfig::default();
            Ok(Box::new(HashiCorpVaultProvider::new(config).await?))
        }
        "k8s" => {
            let config = KubernetesSecretsConfig::default();
            Ok(Box::new(KubernetesSecretsProvider::new(config).await?))
        }
        _ => Err(format!("Unknown provider: {}", provider_type).into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = get_storage_provider().await?;
    
    // Same code works with any provider
    let id = CredentialId::new("my_credential")?;
    let data = encrypt(&key, b"secret")?;
    let metadata = CredentialMetadata::default();
    let context = CredentialContext::new("user_123");
    
    provider.store(&id, data, metadata, &context).await?;
    let (retrieved_data, _) = provider.retrieve(&id, &context).await?;
    
    Ok(())
}
```

### Configuration Files

**config.toml** (local development):
```toml
[storage]
provider = "local"
base_path = "~/.nebula/credentials"
```

**config.toml** (AWS production):
```toml
[storage]
provider = "aws"
region = "us-east-1"
secret_prefix = "nebula/credentials/"
```

**config.toml** (Kubernetes production):
```toml
[storage]
provider = "k8s"
namespace = "nebula"
secret_prefix = "nebula-cred-"
```

---

## Troubleshooting

### Permission Denied Errors

**AWS**: `StorageError::PermissionDenied`
```
Error: Permission denied: AWS Secrets Manager
  Required: secretsmanager:GetSecretValue
  Fix: Add IAM policy granting secretsmanager:GetSecretValue on arn:aws:secretsmanager:*:*:secret:nebula/*
```

**Azure**: `StorageError::PermissionDenied`
```
Error: Permission denied: Azure Key Vault
  Required: Key Vault Secrets User (for read) or Key Vault Secrets Officer (for write)
  Fix: az role assignment create --role "Key Vault Secrets User" --assignee <object-id> --scope <vault-id>
```

**Vault**: `StorageError::PermissionDenied`
```
Error: Permission denied: Vault secret
  Required: Policy with read/write on secret/data/nebula/credentials/*
  Fix: vault policy write nebula-credential <policy-file.hcl>
```

**Kubernetes**: `StorageError::PermissionDenied`
```
Error: Permission denied: Secret 'nebula-cred-database-password' in namespace 'nebula'
  Required: RBAC permissions: secrets.get, secrets.list (Role or ClusterRole)
  Fix: kubectl auth can-i get secrets --as=system:serviceaccount:nebula:nebula-credential-manager -n nebula
```

### Credential Too Large Errors

```
Error: Credential too large
  Size: 128KB
  Limit: 64KB (AWS Secrets Manager)
  Fix: Split credential into chunks or use external storage (S3) with reference
```

### Connection Timeout Errors

```
Error: Operation timeout after 5s
  Operation: retrieve_credential
  Provider: AWS Secrets Manager
  Fix: Check network connectivity, increase timeout in AwsSecretsManagerConfig
```

---

## Next Steps

1. **Read Integration Guides** (detailed provider-specific documentation):
   - `docs/Integrations/Local-Storage.md`
   - `docs/Integrations/AWS-Secrets-Manager.md`
   - `docs/Integrations/Azure-Key-Vault.md`
   - `docs/Integrations/HashiCorp-Vault.md`
   - `docs/Integrations/Kubernetes-Secrets.md`

2. **Review Architecture**:
   - `docs/Meta/TECHNICAL-DESIGN.md` - Implementation details
   - `docs/Reference/StorageBackends.md` - Provider comparison matrix

3. **Explore Examples**:
   - `examples/local-storage.rs` - Complete local storage example
   - `examples/aws-secrets-manager.rs` - AWS integration
   - `examples/provider-comparison.rs` - Switch between providers

4. **Migrate Credentials** (when changing providers):
   - Use `tools/migrate-credentials.rs` to export from one provider and import to another
