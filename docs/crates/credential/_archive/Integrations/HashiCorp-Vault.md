---
title: "HashiCorp Vault Integration"
tags: [integration, provider, vault, hashicorp, multi-cloud, dynamic-secrets]
status: published
lang: ru
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate, advanced]
estimated_reading: 25
priority: P2
---

# HashiCorp Vault Integration

> **TL;DR**: Integrate nebula-credential with HashiCorp Vault for multi-cloud secret management with dynamic secrets, encryption-as-a-service (Transit engine), and lease-based lifecycle management.

## Overview

HashiCorp Vault is a cloud-agnostic secrets management platform with advanced features: dynamic secrets generation, encryption-as-a-service, and lease-based credential lifecycle. This guide covers KV v2 storage, Transit engine encryption, and AppRole authentication.

**Что вы получите**:
- Complete Vault setup for nebula-credential
- KV v2 secrets engine configuration
- Transit engine for encryption-as-a-service
- AppRole authentication for applications
- Dynamic database secrets generation
- Lease renewal and revocation
- High availability and disaster recovery patterns

**Когда использовать**:
- Multi-cloud environments (AWS, Azure, GCP)
- Dynamic secrets required (short-lived database credentials)
- Encryption-as-a-service needed (zero-knowledge key rotation)
- Centralized audit logging across infrastructure
- Kubernetes, Nomad, or other HashiCorp stack
- Zero-trust security architecture

## Prerequisites

- Vault server installed and unsealed
- Vault CLI configured (`export VAULT_ADDR=...`)
- nebula-credential v0.1.0+
- Понимание: [[Core-Concepts]]
- Понимание: [[How-To/Store-Credentials]]

## Architecture Overview

```mermaid
graph TB
    App[Your Application<br/>nebula-credential] --> VaultLib[vaultrs Rust SDK]
    
    VaultLib --> Auth{Authentication}
    Auth -->|AppRole| AppRole[AppRole Auth<br/>role_id + secret_id]
    Auth -->|Kubernetes| K8sSA[K8s ServiceAccount]
    Auth -->|Token| Token[Root/User Token]
    
    AppRole --> Vault[HashiCorp Vault Server]
    K8sSA --> Vault
    Token --> Vault
    
    Vault --> KV[KV v2 Engine<br/>Static Secrets]
    Vault --> Transit[Transit Engine<br/>Encryption]
    Vault --> DB[Database Engine<br/>Dynamic Secrets]
    Vault --> AWS[AWS Engine<br/>Temp Credentials]
    
    KV --> Versioning[Secret Versioning]
    Transit --> Rewrap[Key Rotation<br/>Rewrap]
    DB --> Lease[Lease Management<br/>TTL Renewal]
    
    Vault --> Audit[Audit Devices<br/>file, syslog, socket]
    
    style Vault fill:#00CA8E
    style Transit fill:#00CA8E
    style DB fill:#00CA8E
```

## Step 1: Vault Server Setup

### Install Vault

**Docker** (development):
```bash
docker run -d --name vault \
  --cap-add=IPC_LOCK \
  -e 'VAULT_DEV_ROOT_TOKEN_ID=dev-root-token' \
  -e 'VAULT_DEV_LISTEN_ADDRESS=0.0.0.0:8200' \
  -p 8200:8200 \
  hashicorp/vault:latest

export VAULT_ADDR='http://127.0.0.1:8200'
export VAULT_TOKEN='dev-root-token'
```

**Production** (Linux):
```bash
# Install from HashiCorp repo
wget -O- https://apt.releases.hashicorp.com/gpg | sudo gpg --dearmor -o /usr/share/keyrings/hashicorp-archive-keyring.gpg
echo "deb [signed-by=/usr/share/keyrings/hashicorp-archive-keyring.gpg] https://apt.releases.hashicorp.com $(lsb_release -cs) main" | sudo tee /etc/apt/sources.list.d/hashicorp.list
sudo apt update && sudo apt install vault

# Configure Vault (config.hcl)
cat > /etc/vault.d/vault.hcl <<EOF
storage "raft" {
  path    = "/opt/vault/data"
  node_id = "vault-1"
}

listener "tcp" {
  address     = "0.0.0.0:8200"
  tls_disable = 0
  tls_cert_file = "/opt/vault/tls/vault.crt"
  tls_key_file  = "/opt/vault/tls/vault.key"
}

api_addr = "https://vault.example.com:8200"
cluster_addr = "https://vault-1.example.com:8201"
ui = true
EOF

# Start Vault
sudo systemctl enable vault
sudo systemctl start vault

# Initialize Vault (first time only)
vault operator init -key-shares=5 -key-threshold=3
# Save unseal keys and root token securely!

# Unseal Vault (3 keys required)
vault operator unseal <key-1>
vault operator unseal <key-2>
vault operator unseal <key-3>
```

### Verify Installation

```bash
vault status
# Output: Sealed: false, HA Enabled: true
```

## Step 2: KV v2 Secrets Engine

KV v2 provides versioned key-value storage with rollback capability.

### Enable KV v2 Engine

```bash
# Enable KV v2 at path "nebula"
vault secrets enable -path=nebula -version=2 kv

# Verify
vault secrets list
```

### Configure Max Versions

```bash
# Keep last 10 versions of each secret
vault write nebula/config max_versions=10
```

### Store and Retrieve Secrets

```bash
# Store secret (creates version 1)
vault kv put nebula/credentials/database \
  username=admin \
  password=super_secret

# Retrieve current version
vault kv get nebula/credentials/database

# Retrieve specific version
vault kv get -version=1 nebula/credentials/database

# List all versions
vault kv metadata get nebula/credentials/database
```

## Step 3: Transit Engine (Encryption-as-a-Service)

Transit engine provides encryption without storing data - Vault only stores keys.

### Enable Transit Engine

```bash
# Enable Transit engine
vault secrets enable transit

# Create encryption key
vault write -f transit/keys/nebula-credentials \
  type=aes256-gcm96

# Allow key rotation
vault write transit/keys/nebula-credentials/config \
  min_decryption_version=1 \
  min_encryption_version=0 \
  deletion_allowed=false
```

### Encrypt and Decrypt Data

```bash
# Encrypt data (returns base64 ciphertext with version prefix)
vault write transit/encrypt/nebula-credentials \
  plaintext=$(echo -n "my secret data" | base64)
# Output: vault:v1:8SDd3WHDOjf7mq69CyCqYjBXAiQQAVZRkFM96jCcgN==

# Decrypt data
vault write transit/decrypt/nebula-credentials \
  ciphertext="vault:v1:8SDd3WHDOjf7mq69CyCqYjBXAiQQAVZRkFM96jCcgN=="
# Output: plaintext: bXkgc2VjcmV0IGRhdGE=

# Decode base64
echo "bXkgc2VjcmV0IGRhdGE=" | base64 -d
# Output: my secret data
```

### Key Rotation (Zero-Knowledge)

```bash
# Rotate key (creates new version)
vault write -f transit/keys/nebula-credentials/rotate

# Rewrap ciphertext to use latest key version (no plaintext exposure!)
vault write transit/rewrap/nebula-credentials \
  ciphertext="vault:v1:8SDd3WHDOjf7mq69CyCqYjBXAiQQAVZRkFM96jCcgN=="
# Output: vault:v2:new_ciphertext_with_new_key_version
```

**Key Benefits**:
- Ciphertext automatically upgraded to new key version
- No plaintext ever exposed during rotation
- Old ciphertexts remain decryptable
- Zero-downtime key rotation

## Step 4: AppRole Authentication

AppRole is recommended for applications (vs user tokens).

### Create AppRole

```bash
# Enable AppRole auth
vault auth enable approle

# Create policy for nebula-credential app
cat > nebula-policy.hcl <<EOF
# KV v2 access
path "nebula/data/*" {
  capabilities = ["create", "read", "update", "delete", "list"]
}

path "nebula/metadata/*" {
  capabilities = ["read", "list"]
}

# Transit engine access
path "transit/encrypt/nebula-credentials" {
  capabilities = ["update"]
}

path "transit/decrypt/nebula-credentials" {
  capabilities = ["update"]
}

path "transit/rewrap/nebula-credentials" {
  capabilities = ["update"]
}
EOF

vault policy write nebula-credential nebula-policy.hcl

# Create AppRole
vault write auth/approle/role/nebula-credential \
  token_ttl=1h \
  token_max_ttl=4h \
  secret_id_ttl=24h \
  token_policies=nebula-credential

# Get role_id (static, can be embedded in app)
vault read auth/approle/role/nebula-credential/role-id
# role_id: 12345678-1234-1234-1234-123456789012

# Generate secret_id (dynamic, short-lived)
vault write -f auth/approle/role/nebula-credential/secret-id
# secret_id: abcdef12-3456-7890-abcd-ef1234567890
# secret_id_ttl: 24h
```

### Authenticate with AppRole

```bash
# Login with role_id and secret_id
vault write auth/approle/login \
  role_id=12345678-1234-1234-1234-123456789012 \
  secret_id=abcdef12-3456-7890-abcd-ef1234567890
# Returns: client_token (use for API calls)
```

## Step 5: nebula-credential Integration

### Rust Code Implementation

```rust
// File: examples/vault_integration.rs

use nebula_credential::{StorageProvider, CredentialId, Credential, SecretString};
use vaultrs::client::{VaultClient, VaultClientSettingsBuilder};
use vaultrs::kv2;
use vaultrs::transit;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

#[derive(Clone)]
pub struct VaultProvider {
    client: VaultClient,
    mount_path: String,
    transit_key: Option<String>,
}

impl VaultProvider {
    /// Create new Vault provider with AppRole authentication
    pub async fn new_with_approle(
        vault_addr: &str,
        role_id: &str,
        secret_id: &str,
        mount_path: String,
        transit_key: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Login with AppRole
        let client = VaultClient::new(
            VaultClientSettingsBuilder::default()
                .address(vault_addr)
                .build()?
        )?;
        
        // Authenticate
        let auth_info = vaultrs::auth::approle::login(
            &client,
            "approle",
            role_id,
            secret_id,
        ).await?;
        
        // Set token
        let client = client.with_token(&auth_info.client_token);
        
        Ok(Self {
            client,
            mount_path,
            transit_key,
        })
    }
    
    /// Create with existing token (for development)
    pub fn new_with_token(
        vault_addr: &str,
        token: &str,
        mount_path: String,
        transit_key: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let client = VaultClient::new(
            VaultClientSettingsBuilder::default()
                .address(vault_addr)
                .token(token)
                .build()?
        )?;
        
        Ok(Self {
            client,
            mount_path,
            transit_key,
        })
    }
    
    /// Build secret path
    fn secret_path(&self, credential_id: &CredentialId) -> String {
        format!("credentials/{}", credential_id)
    }
    
    /// Encrypt data with Transit engine (if configured)
    async fn encrypt(&self, plaintext: &str) -> Result<String, Box<dyn std::error::Error>> {
        if let Some(key_name) = &self.transit_key {
            let encrypted = transit::data::encrypt(
                &self.client,
                "transit",
                key_name,
                plaintext.as_bytes(),
            ).await?;
            
            Ok(encrypted.ciphertext)
        } else {
            Ok(plaintext.to_string())
        }
    }
    
    /// Decrypt data with Transit engine (if configured)
    async fn decrypt(&self, ciphertext: &str) -> Result<String, Box<dyn std::error::Error>> {
        if let Some(key_name) = &self.transit_key {
            let decrypted = transit::data::decrypt(
                &self.client,
                "transit",
                key_name,
                ciphertext,
            ).await?;
            
            let plaintext = String::from_utf8(decrypted.plaintext)?;
            Ok(plaintext)
        } else {
            Ok(ciphertext.to_string())
        }
    }
    
    /// Store credential in KV v2
    pub async fn store(
        &self,
        credential_id: CredentialId,
        credential: &Credential,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let path = self.secret_path(&credential_id);
        
        // Serialize credential
        let mut data = HashMap::new();
        data.insert("username", credential.username.clone());
        
        // Encrypt password if Transit configured
        let password = self.encrypt(credential.password.expose()).await?;
        data.insert("password", password);
        
        // Store in KV v2
        kv2::set(
            &self.client,
            &self.mount_path,
            &path,
            &data,
        ).await?;
        
        println!("✓ Stored secret: {}/{}", self.mount_path, path);
        
        Ok(())
    }
    
    /// Retrieve credential from KV v2
    pub async fn retrieve(
        &self,
        credential_id: &CredentialId,
    ) -> Result<Credential, Box<dyn std::error::Error>> {
        let path = self.secret_path(credential_id);
        
        // Read from KV v2
        let secret: HashMap<String, String> = kv2::read(
            &self.client,
            &self.mount_path,
            &path,
        ).await?;
        
        let username = secret.get("username")
            .ok_or("Username not found")?
            .clone();
        
        let encrypted_password = secret.get("password")
            .ok_or("Password not found")?;
        
        // Decrypt password if Transit configured
        let password = self.decrypt(encrypted_password).await?;
        
        println!("✓ Retrieved secret: {}/{}", self.mount_path, path);
        
        Ok(Credential {
            username,
            password: SecretString::new(password),
        })
    }
    
    /// Delete credential
    pub async fn delete(
        &self,
        credential_id: &CredentialId,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let path = self.secret_path(credential_id);
        
        // Delete all versions (metadata delete)
        kv2::delete_metadata(
            &self.client,
            &self.mount_path,
            &path,
        ).await?;
        
        println!("✓ Deleted secret: {}/{}", self.mount_path, path);
        
        Ok(())
    }
    
    /// List all credentials
    pub async fn list(&self) -> Result<Vec<CredentialId>, Box<dyn std::error::Error>> {
        let secrets: Vec<String> = kv2::list(
            &self.client,
            &self.mount_path,
            "credentials",
        ).await?;
        
        let credential_ids: Vec<CredentialId> = secrets
            .iter()
            .filter_map(|s| CredentialId::from_str(s).ok())
            .collect();
        
        Ok(credential_ids)
    }
    
    /// Renew token lease
    pub async fn renew_token(&self) -> Result<(), Box<dyn std::error::Error>> {
        vaultrs::auth::token::renew_self(
            &self.client,
            Some(3600), // Renew for 1 hour
        ).await?;
        
        println!("✓ Token lease renewed");
        
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 HashiCorp Vault Integration Example");
    println!("========================================\n");
    
    // Initialize provider (development mode with root token)
    let provider = VaultProvider::new_with_token(
        "http://127.0.0.1:8200",
        "dev-root-token",
        "nebula".to_string(),
        Some("nebula-credentials".to_string()), // Enable Transit encryption
    )?;
    
    println!("✓ Vault provider initialized");
    println!("  Address: http://127.0.0.1:8200");
    println!("  Mount: nebula");
    println!("  Transit: nebula-credentials\n");
    
    // Store credential
    let credential_id = CredentialId::new();
    let credential = Credential {
        username: "admin".to_string(),
        password: SecretString::new("super_secret_password"),
    };
    
    provider.store(credential_id, &credential).await?;
    
    // Retrieve credential
    let retrieved = provider.retrieve(&credential_id).await?;
    println!("✓ Credential retrieved successfully");
    println!("  Username: {}", retrieved.username);
    
    // List credentials
    let all_credentials = provider.list().await?;
    println!("\n✓ Total credentials: {}", all_credentials.len());
    
    // Renew token lease
    provider.renew_token().await?;
    
    println!("\n✅ Example complete!");
    
    Ok(())
}
```

### Cargo.toml Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
vaultrs = "0.7"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

## Step 6: Dynamic Database Secrets

Vault can generate short-lived database credentials on-demand.

### Enable Database Engine

```bash
# Enable database secrets engine
vault secrets enable database

# Configure PostgreSQL connection
vault write database/config/my-postgresql-database \
  plugin_name=postgresql-database-plugin \
  allowed_roles="nebula-app" \
  connection_url="postgresql://{{username}}:{{password}}@postgres:5432/mydb?sslmode=disable" \
  username="vault" \
  password="vault-password"

# Create role for dynamic credentials
vault write database/roles/nebula-app \
  db_name=my-postgresql-database \
  creation_statements="CREATE ROLE \"{{name}}\" WITH LOGIN PASSWORD '{{password}}' VALID UNTIL '{{expiration}}'; \
    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO \"{{name}}\";" \
  default_ttl="1h" \
  max_ttl="24h"
```

### Generate Dynamic Credentials

```rust
use vaultrs::database;

// Generate credentials (TTL: 1 hour)
let creds = database::generate_credentials(
    &client,
    "database",
    "nebula-app",
).await?;

println!("Username: {}", creds.username);
println!("Password: {}", creds.password);
println!("Lease ID: {}", creds.lease_id);
println!("Lease Duration: {} seconds", creds.lease_duration);

// Renew lease before expiration
database::renew_lease(
    &client,
    &creds.lease_id,
    Some(3600), // Extend by 1 hour
).await?;

// Revoke lease when done
database::revoke_lease(
    &client,
    &creds.lease_id,
).await?;
```

## Monitoring and Troubleshooting

### Audit Logging

Enable audit device for all Vault operations:

```bash
# File audit device
vault audit enable file file_path=/var/log/vault-audit.log

# Syslog audit device
vault audit enable syslog tag="vault" facility="LOCAL7"

# Socket audit device (send to SIEM)
vault audit enable socket address=siem.example.com:514 socket_type=tcp
```

**Audit log entry example**:
```json
{
  "time": "2026-02-03T14:30:00Z",
  "type": "response",
  "auth": {
    "client_token": "hmac-sha256:...",
    "token_type": "service"
  },
  "request": {
    "operation": "read",
    "path": "nebula/data/credentials/database"
  },
  "response": {
    "data": {"username": "hmac-sha256:..."}
  }
}
```

### Health Monitoring

```bash
# Check Vault health
curl http://127.0.0.1:8200/v1/sys/health

# Prometheus metrics
curl http://127.0.0.1:8200/v1/sys/metrics?format=prometheus
```

### Common Issues

**Issue: Sealed Vault**

**Symptoms**: `Error: Vault is sealed`

**Resolution**:
```bash
vault operator unseal <key-1>
vault operator unseal <key-2>
vault operator unseal <key-3>
```

**Issue: Expired Token**

**Symptoms**: `Error: permission denied`

**Resolution**: Renew token or re-authenticate with AppRole

**Issue: Lease Expired**

**Symptoms**: Dynamic credentials no longer work

**Resolution**: Generate new credentials or renew lease proactively

## High Availability Setup

### Raft Storage Backend

```hcl
# vault-1.hcl
storage "raft" {
  path    = "/opt/vault/data"
  node_id = "vault-1"
  
  retry_join {
    leader_api_addr = "https://vault-2.example.com:8200"
  }
  retry_join {
    leader_api_addr = "https://vault-3.example.com:8200"
  }
}

listener "tcp" {
  address     = "0.0.0.0:8200"
  tls_cert_file = "/opt/vault/tls/vault.crt"
  tls_key_file  = "/opt/vault/tls/vault.key"
}

api_addr = "https://vault-1.example.com:8200"
cluster_addr = "https://vault-1.example.com:8201"
```

**Join cluster**:
```bash
# On vault-2 and vault-3
vault operator raft join https://vault-1.example.com:8200
```

### Disaster Recovery

**Backup**:
```bash
# Snapshot Raft data
vault operator raft snapshot save backup.snap

# Upload to S3
aws s3 cp backup.snap s3://vault-backups/$(date +%Y%m%d).snap
```

**Restore**:
```bash
# Download from S3
aws s3 cp s3://vault-backups/20260203.snap restore.snap

# Restore snapshot
vault operator raft snapshot restore -force restore.snap
```

## Cost Optimization

**Vault Enterprise** (licensed):
- Namespaces for multi-tenancy
- Performance replication
- HSM auto-unseal
- Disaster recovery replication

**Vault OSS** (free):
- Core features sufficient for most use cases
- Self-hosted (infrastructure costs only)
- Community support

**Best Practices**:
- Use short TTLs for dynamic secrets (reduce storage)
- Enable lease revocation (automatic cleanup)
- Monitor audit log size (rotate logs)
- Use AppRole (avoid long-lived tokens)

## See Also

- Provider Comparison: [[Integrations/Provider-Comparison]]
- Migration: [[Integrations/Migration-Guide]]
- Caching: [[How-To/Configure-Caching]]
- Rotation: [[How-To/Rotate-Credentials]]
- Troubleshooting: [[Troubleshooting/Provider-Connectivity]]
- Architecture: [[Architecture#storage-provider]]

---

**Validation Checklist**:
- [x] Vault installation documented
- [x] KV v2 configuration explained
- [x] Transit engine setup included
- [x] AppRole authentication configured
- [x] Dynamic database secrets covered
- [x] Complete Rust code example
- [x] Audit logging setup
- [x] HA and DR patterns documented
- [x] Troubleshooting guidance provided
