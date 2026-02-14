---
title: Provider Connectivity Issues
description: Troubleshooting guide for storage provider connection problems (AWS, Azure, Vault, Kubernetes, local)
tags: [troubleshooting, storage, providers, connectivity, aws, azure, vault, kubernetes]
related:
  - "[[Common-Errors]]"
  - "[[../Integrations/AWS-Secrets-Manager]]"
  - "[[../Integrations/Azure-Key-Vault]]"
  - "[[../Integrations/HashiCorp-Vault]]"
  - "[[../Integrations/Kubernetes-Secrets]]"
  - "[[../Integrations/Local-Storage]]"
  - "[[../Integrations/Migration-Guide]]"
  - "[[Debugging-Checklist]]"
status: published
version: 1.0.0
---

# Provider Connectivity Issues

Comprehensive troubleshooting for storage provider connection problems across AWS Secrets Manager, Azure Key Vault, HashiCorp Vault, Kubernetes Secrets, and local storage.

---

## Quick Diagnosis by Provider

| Provider | Error Pattern | First Check | Detailed Section |
|----------|---------------|-------------|------------------|
| AWS | `ConnectionFailed` to secretsmanager | IAM permissions, network | [§1](#1-aws-secrets-manager) |
| Azure | `401 Unauthorized` | Managed Identity, credentials | [§2](#2-azure-key-vault) |
| Vault | Connection refused :8200 | Vault unsealed, token valid | [§3](#3-hashicorp-vault) |
| Kubernetes | `Forbidden` | RBAC, ServiceAccount | [§4](#4-kubernetes-secrets) |
| Local | SQLite errors | File permissions, disk space | [§5](#5-local-storage) |

---

## 1. AWS Secrets Manager

### 1.1 Connection Failed

**Error**: `StorageError::ConnectionFailed("Failed to connect to secretsmanager.us-east-1.amazonaws.com")`

**Diagnostic Checklist**:

```rust
use nebula_credential::prelude::*;
use aws_sdk_secretsmanager::Client;

pub async fn diagnose_aws_connection(
    config: &AwsConfig,
) -> Result<(), StorageError> {
    eprintln!("=== AWS Secrets Manager Diagnostic ===\n");
    
    // 1. Check AWS credentials
    eprintln!("Step 1: Checking AWS credentials...");
    let aws_config = aws_config::load_from_env().await;
    let sts_client = aws_sdk_sts::Client::new(&aws_config);
    
    match sts_client.get_caller_identity().send().await {
        Ok(identity) => {
            eprintln!("✓ AWS credentials valid");
            eprintln!("  Account: {}", identity.account().unwrap_or("N/A"));
            eprintln!("  ARN: {}", identity.arn().unwrap_or("N/A"));
        }
        Err(e) => {
            eprintln!("✗ AWS credentials invalid: {e}");
            eprintln!("\n  Solutions:");
            eprintln!("  1. Set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY");
            eprintln!("  2. Configure ~/.aws/credentials");
            eprintln!("  3. Use IAM role (EC2/ECS/Lambda)");
            return Err(StorageError::ConnectionFailed(e.to_string()));
        }
    }
    
    // 2. Check region configuration
    eprintln!("\nStep 2: Checking region...");
    let region = aws_config.region().map(|r| r.as_ref()).unwrap_or("none");
    eprintln!("  Configured region: {region}");
    
    if region == "none" {
        eprintln!("✗ No region configured");
        eprintln!("\n  Solutions:");
        eprintln!("  1. Set AWS_REGION environment variable");
        eprintln!("  2. Configure in ~/.aws/config");
        return Err(StorageError::ConnectionFailed("No AWS region".to_string()));
    }
    
    // 3. Test Secrets Manager connectivity
    eprintln!("\nStep 3: Testing Secrets Manager connectivity...");
    let sm_client = Client::new(&aws_config);
    
    match sm_client.list_secrets().max_results(1).send().await {
        Ok(_) => eprintln!("✓ Secrets Manager reachable"),
        Err(e) => {
            eprintln!("✗ Secrets Manager unreachable: {e}");
            eprintln!("\n  Solutions:");
            eprintln!("  1. Check network connectivity");
            eprintln!("  2. Verify VPC endpoints if using private subnets");
            eprintln!("  3. Check security groups/NACLs");
            return Err(StorageError::ConnectionFailed(e.to_string()));
        }
    }
    
    // 4. Check IAM permissions
    eprintln!("\nStep 4: Checking IAM permissions...");
    match sm_client.list_secrets().max_results(1).send().await {
        Ok(_) => eprintln!("✓ Has secretsmanager:ListSecrets permission"),
        Err(e) if e.to_string().contains("AccessDenied") => {
            eprintln!("✗ Missing secretsmanager:ListSecrets permission");
            eprintln!("\n  Required IAM policy:");
            eprintln!(r#"  {{
    "Version": "2012-10-17",
    "Statement": [{{
      "Effect": "Allow",
      "Action": [
        "secretsmanager:GetSecretValue",
        "secretsmanager:CreateSecret",
        "secretsmanager:UpdateSecret",
        "secretsmanager:DeleteSecret",
        "secretsmanager:ListSecrets"
      ],
      "Resource": "*"
    }}]
  }}"#);
            return Err(StorageError::ConnectionFailed("Insufficient IAM permissions".to_string()));
        }
        Err(e) => {
            eprintln!("✗ Permission check failed: {e}");
            return Err(StorageError::ConnectionFailed(e.to_string()));
        }
    }
    
    eprintln!("\n✓ All AWS diagnostics passed");
    Ok(())
}
```

**Common Solutions**:

1. **Missing IAM permissions**: Attach policy with `secretsmanager:*` actions
2. **VPC endpoint required**: Create VPC endpoint for `com.amazonaws.REGION.secretsmanager`
3. **Network issues**: Check security groups allow outbound HTTPS (443)

---

### 1.2 Access Denied

**Error**: `WriteFailed("Access Denied (Service: SecretsManager, Status Code: 400)")`

**Solution**: Grant IAM permissions

```bash
# Attach managed policy
aws iam attach-user-policy \
  --user-name my-user \
  --policy-arn arn:aws:iam::aws:policy/SecretsManagerReadWrite

# Or create custom policy
aws iam put-user-policy \
  --user-name my-user \
  --policy-name CredentialManagerPolicy \
  --policy-document file://policy.json
```

---

## 2. Azure Key Vault

### 2.1 Authentication Failed

**Error**: `ConnectionFailed("Authentication failed: 401 Unauthorized")`

**Diagnosis**:

```rust
use nebula_credential::prelude::*;
use azure_identity::DefaultAzureCredential;
use azure_security_keyvault::KeyvaultClient;

pub async fn diagnose_azure_connection(
    vault_url: &str,
) -> Result<(), StorageError> {
    eprintln!("=== Azure Key Vault Diagnostic ===\n");
    
    // 1. Check Azure credentials
    eprintln!("Step 1: Checking Azure credentials...");
    let credential = DefaultAzureCredential::default();
    
    // Test token acquisition
    match credential.get_token("https://vault.azure.net/.default").await {
        Ok(token) => {
            eprintln!("✓ Azure credentials valid");
            eprintln!("  Token expires: {:?}", token.expires_on);
        }
        Err(e) => {
            eprintln!("✗ Azure authentication failed: {e}");
            eprintln!("\n  Solutions:");
            eprintln!("  1. Set AZURE_CLIENT_ID, AZURE_CLIENT_SECRET, AZURE_TENANT_ID");
            eprintln!("  2. Use Managed Identity (Azure VM/App Service)");
            eprintln!("  3. Run 'az login' for interactive auth");
            return Err(StorageError::ConnectionFailed(e.to_string()));
        }
    }
    
    // 2. Check Key Vault permissions
    eprintln!("\nStep 2: Checking Key Vault access...");
    let client = KeyvaultClient::new(vault_url, credential)?;
    
    match client.list_secrets().await {
        Ok(_) => eprintln!("✓ Can access Key Vault"),
        Err(e) if e.to_string().contains("403") => {
            eprintln!("✗ Forbidden: Missing Key Vault access policy");
            eprintln!("\n  Solutions:");
            eprintln!("  1. Add access policy in Azure Portal");
            eprintln!("  2. Grant 'Key Vault Secrets User' role (RBAC)");
            eprintln!("\n  CLI command:");
            eprintln!("  az keyvault set-policy \\");
            eprintln!("    --name <vault-name> \\");
            eprintln!("    --object-id <principal-id> \\");
            eprintln!("    --secret-permissions get set delete list");
            return Err(StorageError::ConnectionFailed("Access policy missing".to_string()));
        }
        Err(e) => {
            eprintln!("✗ Key Vault error: {e}");
            return Err(StorageError::ConnectionFailed(e.to_string()));
        }
    }
    
    eprintln!("\n✓ All Azure diagnostics passed");
    Ok(())
}
```

**Solutions**:

1. **Set access policy**:
   ```bash
   az keyvault set-policy \
     --name my-keyvault \
     --object-id $(az ad signed-in-user show --query objectId -o tsv) \
     --secret-permissions get set delete list
   ```

2. **Use Managed Identity** (recommended for Azure resources):
   ```bash
   # Enable system-assigned identity
   az vm identity assign --name my-vm --resource-group my-rg
   
   # Grant Key Vault access
   az keyvault set-policy \
     --name my-keyvault \
     --object-id $(az vm show --name my-vm --resource-group my-rg --query identity.principalId -o tsv) \
     --secret-permissions get set delete list
   ```

---

## 3. HashiCorp Vault

### 3.1 Connection Refused

**Error**: `ConnectionFailed("Failed to connect to https://vault.example.com:8200")`

**Diagnosis**:

```bash
# 1. Check Vault is running
curl -v https://vault.example.com:8200/v1/sys/health

# 2. Check seal status
vault status

# Expected output if healthy:
# Sealed: false
# Total Shares: 5
# Threshold: 3
# Version: 1.13.0
# ...
```

**Solutions**:

1. **Vault sealed**: Unseal Vault
   ```bash
   vault operator unseal <key1>
   vault operator unseal <key2>
   vault operator unseal <key3>
   ```

2. **Network unreachable**: Check firewall, DNS
   ```bash
   # Test connectivity
   telnet vault.example.com 8200
   
   # Check DNS
   nslookup vault.example.com
   ```

3. **TLS certificate issues**:
   ```rust
   use nebula_credential::prelude::*;
   
   let config = VaultConfig {
       url: "https://vault.example.com:8200".to_string(),
       token: SecretString::new("s.xxxxx"),
       ca_cert: Some(std::fs::read_to_string("/path/to/ca.crt")?),
       // ...
   };
   ```

---

### 3.2 Permission Denied

**Error**: `WriteFailed("Permission denied: 403")`

**Diagnosis**:

```bash
# Check current token capabilities
vault token capabilities secret/data/credentials

# Expected output if permitted:
# create, read, update, delete, list
```

**Solution**: Create policy with required permissions

```hcl
# credential-manager-policy.hcl
path "secret/data/credentials/*" {
  capabilities = ["create", "read", "update", "delete", "list"]
}

path "secret/metadata/credentials/*" {
  capabilities = ["list", "read", "delete"]
}
```

```bash
# Apply policy
vault policy write credential-manager credential-manager-policy.hcl

# Create token with policy
vault token create -policy=credential-manager

# Or attach to existing AppRole
vault write auth/approle/role/credential-manager/policies policies=credential-manager
```

---

## 4. Kubernetes Secrets

### 4.1 Forbidden Error

**Error**: `ConnectionFailed("Forbidden: User 'system:serviceaccount:default:my-app' cannot get secrets")`

**Diagnosis**:

```bash
# Check current permissions
kubectl auth can-i get secrets --as=system:serviceaccount:default:my-app

# Check RoleBinding
kubectl get rolebinding -n default
kubectl describe rolebinding my-app-secrets -n default
```

**Solution**: Create RBAC permissions

```yaml
# secret-access-role.yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: credential-manager
  namespace: default
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: credential-manager-role
  namespace: default
rules:
- apiGroups: [""]
  resources: ["secrets"]
  verbs: ["get", "list", "create", "update", "delete"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: credential-manager-binding
  namespace: default
subjects:
- kind: ServiceAccount
  name: credential-manager
  namespace: default
roleRef:
  kind: Role
  name: credential-manager-role
  apiGroup: rbac.authorization.k8s.io
```

```bash
# Apply
kubectl apply -f secret-access-role.yaml

# Verify
kubectl auth can-i get secrets --as=system:serviceaccount:default:credential-manager
# Should output: yes
```

---

### 4.2 Connection Refused

**Error**: `ConnectionFailed("Connection refused")`

**Diagnosis**:

```bash
# Check kubeconfig
kubectl config view

# Test API server connectivity
kubectl cluster-info

# Check if running in-cluster
ls /var/run/secrets/kubernetes.io/serviceaccount/
```

**Solutions**:

1. **Out-of-cluster**: Ensure kubeconfig is configured
   ```bash
   export KUBECONFIG=~/.kube/config
   kubectl get nodes
   ```

2. **In-cluster**: Use in-cluster configuration
   ```rust
   use nebula_credential::prelude::*;
   use k8s_openapi::api::core::v1::Secret;
   use kube::Client;
   
   // In-cluster configuration (automatic in pods)
   let client = Client::try_default().await?;
   ```

---

## 5. Local Storage (SQLite)

### 5.1 Database Locked

**Error**: `WriteFailed("database is locked")`

**Cause**: Concurrent write access

**Solutions**:

1. **Enable WAL mode** (Write-Ahead Logging):
   ```rust
   use nebula_credential::prelude::*;
   use rusqlite::Connection;
   
   let conn = Connection::open("credentials.db")?;
   conn.execute("PRAGMA journal_mode=WAL", [])?;
   conn.execute("PRAGMA busy_timeout=5000", [])?;  // 5 second timeout
   ```

2. **Use connection pool**:
   ```rust
   use nebula_credential::prelude::*;
   use r2d2_sqlite::SqliteConnectionManager;
   
   let manager = SqliteConnectionManager::file("credentials.db");
   let pool = r2d2::Pool::new(manager)?;
   
   // Pool handles connection reuse and locking
   let conn = pool.get()?;
   ```

---

### 5.2 Disk Full

**Error**: `WriteFailed("disk I/O error")`

**Diagnosis**:

```bash
# Check disk space
df -h /path/to/credentials.db

# Check inode usage
df -i /path/to/credentials.db
```

**Solution**: Free disk space or move database

```bash
# Move to larger volume
mv credentials.db /mnt/large-volume/credentials.db
ln -s /mnt/large-volume/credentials.db credentials.db
```

---

## 6. Network Troubleshooting

### 6.1 Diagnostic Commands

```bash
# Test DNS resolution
nslookup secretsmanager.us-east-1.amazonaws.com
nslookup vault.azure.net
nslookup vault.example.com

# Test connectivity
curl -v https://secretsmanager.us-east-1.amazonaws.com
telnet vault.example.com 8200

# Check SSL/TLS
openssl s_client -connect vault.example.com:8200 -showcerts

# Trace route
traceroute secretsmanager.us-east-1.amazonaws.com
```

---

### 6.2 Proxy Configuration

**For providers behind proxy**:

```bash
# Set proxy environment variables
export HTTP_PROXY=http://proxy.example.com:8080
export HTTPS_PROXY=http://proxy.example.com:8080
export NO_PROXY=localhost,127.0.0.1

# Test with curl
curl -v https://secretsmanager.us-east-1.amazonaws.com
```

```rust
// Configure in Rust client
use reqwest::Client;

let client = Client::builder()
    .proxy(reqwest::Proxy::all("http://proxy.example.com:8080")?)
    .build()?;
```

---

## 7. Provider Migration

**Scenario**: Migrating between providers

**Steps**:

1. **Test new provider connectivity**:
   ```rust
   diagnose_aws_connection(&new_config).await?;
   ```

2. **Migrate credentials** (see [[../Integrations/Migration-Guide]]):
   ```rust
   migrate_credentials(
       old_provider,
       new_provider,
       &migration_config
   ).await?;
   ```

3. **Validate migration**:
   ```rust
   verify_all_credentials_accessible(new_provider).await?;
   ```

4. **Cutover**:
   ```rust
   update_application_config(new_provider);
   ```

---

## Related Documentation

- [[Common-Errors]] - All error types
- [[../Integrations/AWS-Secrets-Manager]] - AWS setup guide
- [[../Integrations/Azure-Key-Vault]] - Azure setup guide
- [[../Integrations/HashiCorp-Vault]] - Vault setup guide
- [[../Integrations/Kubernetes-Secrets]] - Kubernetes setup guide
- [[../Integrations/Local-Storage]] - Local storage guide
- [[../Integrations/Migration-Guide]] - Provider migration
- [[Debugging-Checklist]] - Systematic debugging

---

## Summary

This guide covers:

✅ **AWS Secrets Manager** - IAM permissions, VPC endpoints  
✅ **Azure Key Vault** - Managed Identity, access policies  
✅ **HashiCorp Vault** - Unsealing, policies, tokens  
✅ **Kubernetes Secrets** - RBAC, ServiceAccounts  
✅ **Local Storage** - SQLite locking, disk space  
✅ **Network troubleshooting** - DNS, TLS, proxies  
✅ **Provider migration** - Safe migration procedures  

Always test connectivity with diagnostic tools before deploying to production.
