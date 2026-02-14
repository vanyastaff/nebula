---
title: "How to Store Credentials Securely"
tags: [how-to, storage, credentials, encryption, tutorial]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate]
estimated_reading: 15
priority: P1
---

# How to Store Credentials Securely

> **TL;DR**: Learn how to store credentials with encryption using LocalStorage, AWS Secrets Manager, HashiCorp Vault, Azure Key Vault, or Kubernetes Secrets.

## Overview

This guide walks you through storing credentials securely using nebula-credential's storage providers. You'll learn how to choose the right storage backend, configure encryption, and implement best practices for credential persistence.

**What you'll achieve**:
- Store credentials with automatic AES-256-GCM encryption
- Configure different storage backends (local, cloud, enterprise)
- Implement secure credential lifecycle management
- Set up metadata and tags for organization

## Prerequisites

> [!note] Required knowledge
> Ensure you've completed the following before starting:

- [x] Read: [[Quick-Start]]
- [x] Read: [[Core-Concepts#Storage Providers]]
- [x] Understand: [[Architecture#Storage Abstraction]]
- [x] Installed nebula-credential v0.1.0+

## Step-by-Step Guide

### Step 1: Choose Your Storage Backend

Select the storage provider that matches your infrastructure:

| Provider | Use Case | Best For |
|----------|----------|----------|
| **LocalStorage** | Development, single-machine apps | Testing, local tools, desktop applications |
| **AWS Secrets Manager** | AWS cloud deployments | Production AWS workloads |
| **HashiCorp Vault** | Enterprise multi-cloud | High-security environments, compliance |
| **Azure Key Vault** | Azure cloud deployments | Production Azure workloads |
| **Kubernetes Secrets** | K8s cluster deployments | Containerized microservices |

```rust
use nebula_credential::storage::{
    LocalStorage,
    AwsSecretsManager,
    VaultStorage,
    AzureKeyVault,
    KubernetesSecrets,
};

// Choose one based on your environment
let storage = LocalStorage::new("./credentials.db").await?;
// or
let storage = AwsSecretsManager::new("us-east-1").await?;
// or
let storage = VaultStorage::new("https://vault.example.com", "your-token").await?;
```

**Expected result**:
```
✓ Storage provider initialized successfully
```

### Step 2: Initialize the Credential Manager

Create a `CredentialManager` with your chosen storage backend:

```rust
use nebula_credential::CredentialManager;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize storage
    let storage = LocalStorage::new("./credentials.db").await?;
    
    // Create manager
    let manager = CredentialManager::new(storage);
    
    println!("✓ Credential manager ready");
    
    Ok(())
}
```

> [!tip] Singleton Pattern
> In production applications, create the `CredentialManager` once at application startup and pass it as a dependency. Don't create multiple managers for the same storage backend.

### Step 3: Store a Simple Credential

Store your first credential with automatic encryption:

```rust
use nebula_credential::{ApiKeyCredential, SecretString};

// Create an API key credential
let github_key = ApiKeyCredential::new(
    "github-api",
    SecretString::from("ghp_abc123def456xyz789"),
);

// Store with a unique identifier
manager.store("github-production", github_key).await?;

println!("✓ Credential stored and encrypted");
```

**What happens internally**:
1. Credential is serialized to bytes
2. AES-256-GCM encryption is applied with a unique 96-bit nonce
3. Encrypted data is stored in your chosen backend
4. Original memory is zeroized

### Step 4: Store Credentials with Metadata

Add metadata for better organization and lifecycle management:

```rust
use nebula_credential::{ApiKeyCredential, SecretString, Metadata, CredentialTags};
use std::time::{Duration, SystemTime};

// Create credential with metadata
let mut api_key = ApiKeyCredential::new(
    "stripe-api",
    SecretString::from("sk_live_abc123"),
);

// Add expiration
let expires_at = SystemTime::now() + Duration::from_secs(7_776_000); // 90 days
api_key.set_metadata(Metadata {
    expires_at: Some(expires_at),
    created_at: SystemTime::now(),
    tags: CredentialTags::new()
        .add("environment", "production")
        .add("team", "payments")
        .add("rotation-policy", "90-days"),
    description: Some("Stripe production API key for payments service".to_string()),
    ..Default::default()
});

// Store with metadata
manager.store("stripe-prod-payments", api_key).await?;

println!("✓ Credential stored with metadata");
```

## Storage Provider Configuration

### LocalStorage (Development/Testing)

Perfect for development and desktop applications:

```rust
use nebula_credential::storage::LocalStorage;

// Basic configuration
let storage = LocalStorage::new("./credentials.db").await?;

// With custom encryption key
let storage = LocalStorage::builder()
    .path("./secure_credentials.db")
    .encryption_key_derivation(KeyDerivation::Argon2id {
        memory_cost: 19456,  // 19 MiB
        time_cost: 2,
        parallelism: 1,
    })
    .build()
    .await?;
```

### AWS Secrets Manager (Production)

For AWS cloud deployments:

```rust
use nebula_credential::storage::AwsSecretsManager;
use aws_config::BehaviorVersion;

// Use default AWS credentials from environment
let aws_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
let storage = AwsSecretsManager::from_config(&aws_config);

// Or specify region explicitly
let storage = AwsSecretsManager::new("us-west-2").await?;
```

**Dependencies** (`Cargo.toml`):
```toml
[dependencies]
nebula-credential = { version = "0.1.0", features = ["aws-storage"] }
aws-config = "1.0"
aws-sdk-secretsmanager = "1.0"
```

### HashiCorp Vault (Enterprise)

For high-security enterprise environments:

```rust
use nebula_credential::storage::VaultStorage;

// Connect to Vault
let storage = VaultStorage::builder()
    .address("https://vault.company.com")
    .token(std::env::var("VAULT_TOKEN")?)
    .namespace("production/credentials")
    .build()
    .await?;

// With AppRole authentication
let storage = VaultStorage::builder()
    .address("https://vault.company.com")
    .approle_auth(
        "my-role-id",
        "my-secret-id",
    )
    .build()
    .await?;
```

**Dependencies**:
```toml
[dependencies]
nebula-credential = { version = "0.1.0", features = ["vault-storage"] }
vaultrs = "0.7"
```

### Azure Key Vault

For Azure deployments:

```rust
use nebula_credential::storage::AzureKeyVault;
use azure_identity::DefaultAzureCredential;

// Use managed identity (recommended for Azure VMs/AKS)
let credential = DefaultAzureCredential::new()?;
let storage = AzureKeyVault::builder()
    .vault_url("https://my-vault.vault.azure.net")
    .credential(credential)
    .build()
    .await?;
```

**Dependencies**:
```toml
[dependencies]
nebula-credential = { version = "0.1.0", features = ["azure-storage"] }
azure_security_keyvault = "0.20"
azure_identity = "0.20"
```

### Kubernetes Secrets

For containerized deployments:

```rust
use nebula_credential::storage::KubernetesSecrets;
use kube::Client;

// Auto-detect in-cluster configuration
let k8s_client = Client::try_default().await?;
let storage = KubernetesSecrets::builder()
    .client(k8s_client)
    .namespace("production")
    .build();

// Store credentials as Kubernetes secrets
manager.store("db-password", db_credential).await?;
```

**Dependencies**:
```toml
[dependencies]
nebula-credential = { version = "0.1.0", features = ["k8s-storage"] }
kube = { version = "0.88", features = ["runtime", "derive"] }
k8s-openapi = { version = "0.21", features = ["latest"] }
```

## Complete Example

Here's a full working example combining all steps:

```rust
// File: examples/store_credentials.rs
use nebula_credential::{
    CredentialManager,
    ApiKeyCredential,
    OAuth2Credential,
    DatabaseCredential,
    SecretString,
    Metadata,
    CredentialTags,
    storage::LocalStorage,
};
use std::time::{Duration, SystemTime};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("💾 Storing Credentials Guide\n");
    
    // Step 1: Initialize storage
    let storage = LocalStorage::new("./demo_credentials.db").await?;
    let manager = CredentialManager::new(storage);
    println!("✓ Storage initialized\n");
    
    // Step 2: Store API key with metadata
    println!("Storing API key...");
    let mut github_key = ApiKeyCredential::new(
        "github-api",
        SecretString::from("ghp_xxxxxxxxxxxxxxxxxxx"),
    );
    
    github_key.set_metadata(Metadata {
        created_at: SystemTime::now(),
        expires_at: Some(SystemTime::now() + Duration::from_secs(7_776_000)),
        tags: CredentialTags::new()
            .add("environment", "production")
            .add("service", "ci-cd"),
        description: Some("GitHub API key for CI/CD pipeline".to_string()),
        ..Default::default()
    });
    
    manager.store("github-ci", github_key).await?;
    println!("✓ GitHub API key stored\n");
    
    // Step 3: Store OAuth2 credential
    println!("Storing OAuth2 credential...");
    let oauth2 = OAuth2Credential::builder()
        .client_id("my-oauth-client-id")
        .client_secret(SecretString::from("my-oauth-client-secret"))
        .access_token(SecretString::from("ya29.access_token_here"))
        .refresh_token(Some(SecretString::from("1//refresh_token_here")))
        .expires_in(Duration::from_secs(3600))
        .scopes(vec!["read", "write"])
        .build()?;
    
    manager.store("google-oauth", oauth2).await?;
    println!("✓ OAuth2 credential stored\n");
    
    // Step 4: Store database credential
    println!("Storing database credential...");
    let db_cred = DatabaseCredential::new(
        "postgresql",
        "db.example.com",
        5432,
        "myapp_user",
        SecretString::from("secure_db_password_123"),
        Some("production_db"),
    );
    
    manager.store("postgres-prod", db_cred).await?;
    println!("✓ Database credential stored\n");
    
    // Step 5: List all stored credentials
    println!("📋 All stored credentials:");
    let all_creds = manager.list().await?;
    for (idx, cred_id) in all_creds.iter().enumerate() {
        println!("  {}. {}", idx + 1, cred_id);
    }
    
    println!("\n🎉 Successfully stored {} credentials!", all_creds.len());
    
    // Cleanup
    std::fs::remove_file("./demo_credentials.db").ok();
    
    Ok(())
}
```

**Cargo.toml**:
```toml
[dependencies]
nebula-credential = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
```

## Verification

To confirm everything works correctly:

1. **Run the example**:
   ```bash
   cargo run --example store_credentials
   ```

2. **Check the output**:
   ```
   💾 Storing Credentials Guide
   
   ✓ Storage initialized
   
   Storing API key...
   ✓ GitHub API key stored
   
   Storing OAuth2 credential...
   ✓ OAuth2 credential stored
   
   Storing database credential...
   ✓ Database credential stored
   
   📋 All stored credentials:
     1. github-ci
     2. google-oauth
     3. postgres-prod
   
   🎉 Successfully stored 3 credentials!
   ```

3. **Verify encryption**: For LocalStorage, check the database file:
   ```bash
   # Credentials are encrypted - you should see binary data, not plaintext
   sqlite3 demo_credentials.db "SELECT length(encrypted_data) FROM credentials;"
   ```

## Troubleshooting

### Problem: "Permission denied" when creating database

**Symptoms**:
- Error: `IO error: Permission denied (os error 13)`
- LocalStorage fails to initialize

**Cause**: Application doesn't have write permissions in the target directory

**Solution**:
```rust
// Use absolute path with proper permissions
let home = std::env::var("HOME")?;
let db_path = format!("{}/.config/myapp/credentials.db", home);

// Create directory if it doesn't exist
std::fs::create_dir_all(format!("{}/.config/myapp", home))?;

let storage = LocalStorage::new(&db_path).await?;
```

### Problem: AWS credentials not found

**Symptoms**:
- Error: `NoCredentialsError` when using AwsSecretsManager
- Authentication fails

**Cause**: AWS credentials not configured

**Solution**:
```bash
# Set environment variables
export AWS_ACCESS_KEY_ID="your-access-key"
export AWS_SECRET_ACCESS_KEY="your-secret-key"
export AWS_REGION="us-east-1"

# Or use AWS CLI to configure
aws configure
```

### Problem: Vault connection timeout

**Symptoms**:
- Error: `Connection timeout` when connecting to Vault
- `VaultStorageError::NetworkError`

**Solution**:
```rust
// Increase timeout and verify connectivity
let storage = VaultStorage::builder()
    .address("https://vault.company.com")
    .token(std::env::var("VAULT_TOKEN")?)
    .timeout(Duration::from_secs(30))  // Increase timeout
    .build()
    .await?;

// Verify Vault is reachable
// curl https://vault.company.com/v1/sys/health
```

## Best Practices

> [!tip] Production Recommendations
> - **Use cloud storage in production**: LocalStorage is great for development, but use AWS/Azure/Vault for production
> - **Set expiration times**: Always set `expires_at` for time-sensitive credentials
> - **Use meaningful IDs**: Credential IDs should be descriptive (e.g., "stripe-prod-payments", not "cred1")
> - **Tag everything**: Use tags for filtering and organization (environment, team, service)
> - **Rotate regularly**: Implement automatic rotation for long-lived credentials (see [[How-To/RotateCredentials]])

> [!warning] Security Considerations
> - **Never commit credentials.db**: Add it to `.gitignore`
> - **Secure your master key**: For LocalStorage, ensure the database file has restrictive permissions (chmod 600)
> - **Use managed identities**: In cloud environments, prefer managed identities over static credentials
> - **Audit access**: Enable audit logging for production storage backends

## Next Steps

After mastering credential storage, explore:

- **Retrieval**: [[How-To/Retrieve-Credentials]] - Query and filter stored credentials
- **Rotation**: [[How-To/RotateCredentials]] - Implement automatic rotation
- **Integration Guides**:
  - [[Integrations/AWS-Secrets-Manager]] - AWS setup and IAM policies
  - [[Integrations/HashiCorp-Vault]] - Vault policies and namespaces
  - [[Integrations/Azure-Key-Vault]] - Azure RBAC configuration
  - [[Integrations/Kubernetes-Secrets]] - K8s service accounts and RBAC

## See Also

- **Concept**: [[Core-Concepts#Storage Providers]] - Storage architecture overview
- **Example**: [[Examples/API-Key-Basic]] - Basic credential storage example
- **Architecture**: [[Architecture#Storage Abstraction]] - Storage provider design
- **Troubleshooting**: [[Troubleshooting/Storage-Provider-Issues]] - Storage-specific problems
- **API Reference**: [[API-Reference#StorageProvider]] - Complete storage API
- **Configuration**: [[Configuration-Options#Storage Configuration]] - All configuration options

---

**Validation Checklist**:
- [x] All storage providers documented
- [x] Step-by-step instructions with code
- [x] Each step has expected output
- [x] Complete working example provided
- [x] Prerequisites explicitly listed
- [x] Verification steps included
- [x] Common issues with solutions
- [x] Security best practices highlighted
- [x] Multiple credential types demonstrated
