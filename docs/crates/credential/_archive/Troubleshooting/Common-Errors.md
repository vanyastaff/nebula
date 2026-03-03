---
title: Common Errors
description: Comprehensive catalog of nebula-credential error types with quick diagnosis and solutions
tags: [troubleshooting, errors, diagnostics, reference]
related:
  - "[[Decryption-Failures]]"
  - "[[OAuth2-Issues]]"
  - "[[Rotation-Failures]]"
  - "[[Scope-Violations]]"
  - "[[Provider-Connectivity]]"
  - "[[Debugging-Checklist]]"
  - "[[../../Reference/API-Reference]]"
status: published
version: 1.0.0
---

# Common Errors

This guide catalogs all error types in `nebula-credential` with quick diagnosis tables, solutions, and links to detailed troubleshooting pages.

---

## Quick Diagnosis Table

| Error Pattern | Likely Cause | First Steps | Detailed Guide |
|--------------|--------------|-------------|----------------|
| `DecryptionFailed` | Wrong key, corrupted data, version mismatch | Check key rotation, verify data integrity | [[Decryption-Failures]] |
| `PermissionDenied` | Missing ACL entry, wrong owner | Verify owner_id, check ACL | [[Scope-Violations]] |
| `NotFound(CredentialId)` | Wrong ID, deleted credential | Verify ID, check storage | [[Provider-Connectivity]] |
| `Expired(DateTime)` | Credential past expiry | Refresh or rotate credential | [[Rotation-Failures]] |
| `OAuth2Error::TokenExchangeFailed` | Wrong code, expired state | Check OAuth2 flow state | [[OAuth2-Issues]] |
| `OAuth2Error::StateMismatch` | CSRF attack or state corruption | Regenerate flow, verify state | [[OAuth2-Issues]] |
| `StorageError::ConnectionFailed` | Storage provider unreachable | Check network, provider status | [[Provider-Connectivity]] |
| `CryptoError::NoValidKey` | All keys tried, none decrypt | Check key rotation history | [[Decryption-Failures]] |
| `TestFailed` | Credential invalid or revoked | Re-authenticate, check remote status | [[Debugging-Checklist]] |
| `StateTransition::InvalidTransition` | Invalid state machine transition | Review credential state | [[../Advanced/Credential-Lifecycle]] |
| `SamlError::SignatureVerificationFailed` | Wrong certificate, tampered XML | Verify IdP certificate | [[OAuth2-Issues]] |
| `LdapError::BindFailed` | Wrong password, user not found | Check credentials, DN template | [[Provider-Connectivity]] |
| `MtlsError::CertificateExpired` | Certificate past validity | Rotate certificate | [[Rotation-Failures]] |
| `JwtError::TokenValidationFailed` | Wrong signature, expired token | Check signing key, expiry | [[OAuth2-Issues]] |
| `ApiKeyError::KeyExpired` | Key past expiration | Rotate API key | [[Rotation-Failures]] |

---

## Error Hierarchy Reference

### Top-Level: `CredentialError`

```rust
pub enum CredentialError {
    AuthenticationFailed(String),
    NotFound(CredentialId),
    Expired(DateTime<Utc>),
    InvalidFormat(String),
    TestFailed(String),
    PermissionDenied(String),
    StateTransition(StateTransitionError),
    Storage(StorageError),
    Encryption(CryptoError),
    OAuth2(OAuth2Error),
    Saml(SamlError),
    Ldap(LdapError),
    Mtls(MtlsError),
    Jwt(JwtError),
    ApiKey(ApiKeyError),
    Kerberos(KerberosError),
}
```

---

## 1. Authentication Errors

### 1.1 `AuthenticationFailed(String)`

**Description**: Generic authentication failure.

**Common Causes**:
- Invalid credentials (wrong password, API key, token)
- Remote service rejected authentication
- Network timeout during authentication
- Account locked or disabled

**Diagnosis**:

```bash
# Check error message for specifics
Error: AuthenticationFailed("Invalid API key")
Error: AuthenticationFailed("Account locked")
Error: AuthenticationFailed("Service unavailable: 503")
```

**Solutions**:

1. **Invalid credentials**: Re-enter correct credentials
2. **Account locked**: Contact service administrator
3. **Service unavailable**: Retry with exponential backoff
4. **Network timeout**: Check network connectivity

**Example**:

```rust
use nebula_credential::prelude::*;

match credential.authenticate(&ctx).await {
    Err(CredentialError::AuthenticationFailed(msg)) => {
        if msg.contains("Invalid") {
            // Re-prompt user for credentials
            eprintln!("Invalid credentials, please re-enter");
        } else if msg.contains("locked") {
            // Account issue
            eprintln!("Account locked, contact administrator");
        } else if msg.contains("503") || msg.contains("unavailable") {
            // Retry with backoff
            tokio::time::sleep(Duration::from_secs(5)).await;
            // retry_authentication().await;
        }
    }
    Ok(output) => println!("Authentication successful"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

---

### 1.2 `TestFailed(String)`

**Description**: Credential test operation failed.

**Common Causes**:
- Credential revoked remotely
- Insufficient permissions
- Test endpoint unreachable
- Credential format invalid

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

let result = credential.test(&ctx).await;
match result {
    Ok(test_result) if !test_result.success => {
        eprintln!("Test failed: {}", test_result.message);
        if let Some(details) = &test_result.details {
            eprintln!("Endpoint: {}", details.endpoint_tested);
            eprintln!("Latency: {}ms", details.latency_ms);
        }
    }
    Err(CredentialError::TestFailed(msg)) => {
        eprintln!("Test error: {msg}");
    }
    _ => {}
}
```

**Solutions**:

1. **Revoked credential**: Re-authenticate and obtain new credential
2. **Insufficient permissions**: Request elevated access
3. **Endpoint unreachable**: Check network, retry later
4. **Invalid format**: Validate credential structure

See: [[Debugging-Checklist#Test-Validation]]

---

## 2. Access Control Errors

### 2.1 `PermissionDenied(String)`

**Description**: User lacks required permission for operation.

**Common Causes**:
- Not the credential owner
- Missing ACL entry
- Insufficient permission level (e.g., read-only trying to write)
- Scope mismatch

**Error Messages**:

```
PermissionDenied("User 'alice' cannot rotate credential owned by 'bob'")
PermissionDenied("Read-only access, cannot delete credential")
PermissionDenied("Scope 'workflow:123' does not match credential scope 'global'")
```

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

match manager.delete_credential(&id, &ctx).await {
    Err(CredentialError::PermissionDenied(msg)) => {
        eprintln!("Permission denied: {msg}");
        
        // Check ownership
        let metadata = manager.get_metadata(&id).await?;
        eprintln!("Owner: {}", metadata.owner_id);
        eprintln!("Requester: {}", ctx.owner_id);
        
        // Check ACL
        let acl = manager.get_acl(&id).await?;
        eprintln!("ACL entries: {:#?}", acl.entries);
    }
    Ok(_) => println!("Deleted successfully"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

**Solutions**:

1. **Wrong owner**: Request access from owner or use correct context
2. **Missing ACL**: Owner must grant permission via `grant_access()`
3. **Insufficient permission**: Request higher permission level
4. **Scope mismatch**: Use correct scope_id in context

**Prevention**:

```rust
// Grant permission before operation
acl.grant_access(
    "alice".to_string(),
    PrincipalType::User,
    PermissionSet {
        can_read: true,
        can_write: true,
        can_delete: true,
        can_rotate: false,
        can_test: true,
        can_share: false,
    },
    "bob".to_string(), // granted_by
);

// Verify permission before operation
if acl.has_permission("alice", Permission::Delete) {
    manager.delete_credential(&id, &ctx).await?;
} else {
    eprintln!("Alice lacks delete permission");
}
```

See: [[Scope-Violations]], [[../Advanced/Access-Control]]

---

### 2.2 `NotFound(CredentialId)`

**Description**: Credential ID does not exist in storage.

**Common Causes**:
- Credential deleted
- Wrong credential ID
- Scope mismatch (credential exists but in different scope)
- Storage provider corruption

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

match manager.retrieve_credential(&id, &ctx).await {
    Err(CredentialError::NotFound(missing_id)) => {
        eprintln!("Credential not found: {missing_id}");
        
        // List all credentials to verify
        let filter = CredentialFilter::new()
            .owner(ctx.owner_id.clone());
        let all_creds = manager.list_credentials(Some(&filter)).await?;
        
        eprintln!("Available credentials:");
        for metadata in all_creds {
            eprintln!("  - {} (type: {})", metadata.id, metadata.credential_type);
        }
    }
    Ok(cred) => println!("Found: {cred:?}"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

**Solutions**:

1. **Deleted credential**: Re-authenticate and create new credential
2. **Wrong ID**: Verify ID from list operation
3. **Scope mismatch**: Check scope_id in context
4. **Corruption**: Check storage provider health

See: [[Provider-Connectivity]]

---

## 3. Expiration Errors

### 3.1 `Expired(DateTime<Utc>)`

**Description**: Credential past its expiration timestamp.

**Common Causes**:
- Token expired (OAuth2, JWT)
- Certificate expired (mTLS)
- API key past expiration
- Cached credential stale

**Diagnosis**:

```rust
use nebula_credential::prelude::*;
use chrono::Utc;

match manager.retrieve_credential(&id, &ctx).await {
    Err(CredentialError::Expired(expired_at)) => {
        let now = Utc::now();
        let duration = now.signed_duration_since(expired_at);
        
        eprintln!("Credential expired at: {expired_at}");
        eprintln!("Current time: {now}");
        eprintln!("Expired {} ago", format_duration(duration));
        
        // Check if refresh is supported
        if credential.supports_refresh() {
            eprintln!("Credential supports refresh, attempting...");
            let refreshed = credential.refresh(&expired_credential).await?;
            manager.store_credential(&id, &refreshed, &ctx).await?;
        } else {
            eprintln!("Credential does not support refresh, re-authentication required");
        }
    }
    Ok(cred) => println!("Credential valid"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

**Solutions**:

1. **Supports refresh**: Call `credential.refresh()` to obtain new token
2. **No refresh**: Re-authenticate from scratch
3. **Rotation**: Use rotation policy to prevent expiration
4. **Cache invalidation**: Clear cache and retrieve fresh credential

**Automatic Refresh Example**:

```rust
use nebula_credential::prelude::*;

pub async fn get_valid_credential(
    manager: &CredentialManager,
    id: &CredentialId,
    ctx: &CredentialContext,
) -> Result<impl Credential, CredentialError> {
    match manager.retrieve_credential(id, ctx).await {
        Ok(cred) => Ok(cred),
        Err(CredentialError::Expired(_)) => {
            eprintln!("Credential expired, refreshing...");
            
            // Retrieve expired credential
            let expired = manager.retrieve_credential_unchecked(id).await?;
            
            // Refresh
            let refreshed = expired.refresh(&expired).await?;
            
            // Store refreshed
            manager.store_credential(id, &refreshed, ctx).await?;
            
            Ok(refreshed)
        }
        Err(e) => Err(e),
    }
}
```

See: [[Rotation-Failures]], [[../How-To/Rotate-Credentials]]

---

## 4. Format Errors

### 4.1 `InvalidFormat(String)`

**Description**: Credential data structure is invalid.

**Common Causes**:
- Corrupted serialization
- Version mismatch
- Manual editing of encrypted data
- Partial write during crash

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

match manager.retrieve_credential(&id, &ctx).await {
    Err(CredentialError::InvalidFormat(msg)) => {
        eprintln!("Invalid format: {msg}");
        
        // Retrieve raw encrypted data for inspection
        let encrypted = storage.retrieve(&id).await?;
        eprintln!("Encrypted data version: {}", encrypted.version);
        eprintln!("Nonce length: {}", encrypted.nonce.len());
        eprintln!("Ciphertext length: {}", encrypted.ciphertext.len());
        
        // Check for corruption
        if encrypted.ciphertext.len() < 16 {
            eprintln!("Ciphertext suspiciously short, likely corrupted");
        }
    }
    Ok(cred) => println!("Format valid"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

**Solutions**:

1. **Corrupted data**: Delete and re-create credential
2. **Version mismatch**: Migrate to current version
3. **Partial write**: Check storage provider consistency
4. **Manual editing**: Never edit encrypted data directly

See: [[Decryption-Failures]]

---

## 5. State Transition Errors

### 5.1 `StateTransition(StateTransitionError)`

**Description**: Invalid state machine transition attempted.

**Common Causes**:
- Attempting operation in wrong state (e.g., rotating uninitialized credential)
- Concurrent state modifications
- Logic error in credential lifecycle

**Valid Transitions**:

```
Uninitialized → PendingInteraction | Authenticating
PendingInteraction → Authenticating | Revoked
Authenticating → Active | Invalid | Revoked
Active → Expired | Rotating | Revoked | Invalid
Expired → Active | Revoked
Rotating → GracePeriod | Active | Revoked
GracePeriod → Active | Revoked
Invalid → Authenticating | Revoked
```

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

let mut tracker = CredentialStateTracker::new();

match tracker.transition(
    CredentialState::Active,
    "User initiated rotation".to_string(),
    "alice".to_string(),
) {
    Err(StateTransitionError::InvalidTransition { from, to }) => {
        eprintln!("Cannot transition from {from} to {to}");
        eprintln!("Current state: {:?}", tracker.current_state);
        eprintln!("History:");
        for transition in &tracker.history {
            eprintln!("  {} → {} at {} ({})",
                transition.from,
                transition.to,
                transition.timestamp,
                transition.reason
            );
        }
    }
    Ok(_) => println!("Transition successful"),
}
```

**Solutions**:

1. **Check current state**: Verify state before operation
2. **Review lifecycle**: Understand valid transitions
3. **Fix logic**: Ensure operations follow state machine
4. **Concurrent modification**: Use locking to prevent race conditions

See: [[../Advanced/Credential-Lifecycle]]

---

## 6. Storage Errors

### 6.1 `StorageError::ConnectionFailed(String)`

**Description**: Cannot connect to storage provider.

**Common Causes**:
- Network unreachable
- Provider down or overloaded
- Wrong credentials/configuration
- Firewall blocking connection

**Diagnosis**:

```bash
# AWS Secrets Manager
Error: ConnectionFailed("Failed to connect to secretsmanager.us-east-1.amazonaws.com")

# HashiCorp Vault
Error: ConnectionFailed("Failed to connect to https://vault.example.com:8200")

# Azure Key Vault
Error: ConnectionFailed("Failed to connect to mykeyvault.vault.azure.net")

# Kubernetes Secrets
Error: ConnectionFailed("Failed to connect to Kubernetes API: connection refused")
```

**Solutions**:

1. **Network**: Verify DNS, ping provider endpoint
2. **Provider status**: Check status page (e.g., status.aws.amazon.com)
3. **Credentials**: Verify IAM role, service principal, kubeconfig
4. **Firewall**: Allow outbound HTTPS (443) to provider

**Troubleshooting Steps**:

```bash
# 1. Check DNS
nslookup secretsmanager.us-east-1.amazonaws.com

# 2. Check network connectivity
curl -v https://vault.example.com:8200/v1/sys/health

# 3. Verify credentials (AWS)
aws sts get-caller-identity

# 4. Check Kubernetes access
kubectl auth can-i get secrets
```

See: [[Provider-Connectivity]], [[../Integrations/Migration-Guide]]

---

### 6.2 `StorageError::WriteFailed(String)`

**Description**: Failed to write credential to storage.

**Common Causes**:
- Storage quota exceeded
- Insufficient permissions
- Validation failure (e.g., AWS secret name invalid)
- Provider error

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

match storage.store(&id, &encrypted, &metadata).await {
    Err(StorageError::WriteFailed(msg)) => {
        eprintln!("Write failed: {msg}");
        
        if msg.contains("quota") || msg.contains("limit") {
            eprintln!("Storage quota exceeded");
            // Check current usage
        } else if msg.contains("permission") || msg.contains("denied") {
            eprintln!("Insufficient permissions");
            // Verify IAM policy, RBAC
        } else if msg.contains("invalid") {
            eprintln!("Validation failed");
            // Check secret name, tags
        }
    }
    Ok(_) => println!("Write successful"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

**Solutions**:

1. **Quota exceeded**: Delete old secrets, request quota increase
2. **Permissions**: Grant `secretsmanager:CreateSecret` or equivalent
3. **Validation**: Follow provider naming rules (e.g., AWS: alphanumeric + `/_+=.@-`)
4. **Provider error**: Retry with exponential backoff

See: [[Provider-Connectivity]]

---

### 6.3 `StorageError::SerializationError(String)`

**Description**: Failed to serialize credential data.

**Common Causes**:
- Non-serializable field (e.g., `X509` certificate)
- Recursive structure
- Type mismatch

**Solutions**:

1. **Use serializable types**: Convert to PEM strings before serialization
2. **Check structure**: Avoid circular references

**Example Fix**:

```rust
use nebula_credential::prelude::*;

// BAD: X509 not serializable
#[derive(Serialize)]
pub struct BadMtlsCredential {
    pub cert: X509, // Error!
}

// GOOD: Use PEM string
#[derive(Serialize)]
pub struct GoodMtlsCredential {
    pub cert_pem: String,
}

impl From<X509> for GoodMtlsCredential {
    fn from(cert: X509) -> Self {
        Self {
            cert_pem: String::from_utf8(cert.to_pem().unwrap()).unwrap(),
        }
    }
}
```

---

## 7. Encryption Errors

### 7.1 `CryptoError::DecryptionFailed(String)`

**Description**: Failed to decrypt credential data.

**Common Causes**:
- Wrong encryption key
- Corrupted ciphertext
- Tampered nonce
- Version mismatch

**Diagnosis**: See [[Decryption-Failures]] for comprehensive guide.

**Quick Fix**:

```rust
use nebula_credential::prelude::*;

match manager.retrieve_credential(&id, &ctx).await {
    Err(CredentialError::Encryption(CryptoError::DecryptionFailed(msg))) => {
        eprintln!("Decryption failed: {msg}");
        
        // Try key rotation manager with all historical keys
        let rotation_manager = KeyRotationManager::new(current_key);
        rotation_manager.add_previous_key(key_id_1, old_key_1);
        rotation_manager.add_previous_key(key_id_2, old_key_2);
        
        // Attempt decryption with all keys
        match rotation_manager.decrypt(&encrypted_data).await {
            Ok((plaintext, key_id)) => {
                eprintln!("Decrypted with key: {key_id}");
                // Re-encrypt with current key
                let re_encrypted = rotation_manager.encrypt(&plaintext).await?;
                storage.store(&id, &re_encrypted, &metadata).await?;
            }
            Err(_) => eprintln!("All keys failed"),
        }
    }
    Ok(cred) => println!("Decryption successful"),
    Err(e) => eprintln!("Other error: {e}"),
}
```

See: [[Decryption-Failures]]

---

### 7.2 `CryptoError::NoValidKey`

**Description**: None of the available keys could decrypt the data.

**Common Causes**:
- All encryption keys lost
- Data encrypted with unknown key
- Key rotation without migration

**Solutions**:

1. **Restore keys**: Retrieve from backup or HSM
2. **Re-authenticate**: Create new credential
3. **Key migration**: Ensure all keys available during rotation

See: [[Decryption-Failures]], [[../Advanced/Key-Management]]

---

### 7.3 `CryptoError::UnsupportedVersion(u8)`

**Description**: Encrypted data uses unsupported algorithm version.

**Example**:

```
UnsupportedVersion(2) // Current version is 1
```

**Solutions**:

1. **Upgrade library**: Update `nebula-credential` to support newer versions
2. **Downgrade data**: If possible, re-encrypt with current version
3. **Migration tool**: Use migration script to convert versions

---

## 8. Protocol-Specific Errors

### 8.1 OAuth2 Errors

See: [[OAuth2-Issues]] for comprehensive OAuth2 troubleshooting.

#### `OAuth2Error::TokenExchangeFailed`

**Quick Diagnosis**:

```rust
Error: OAuth2(TokenExchangeFailed(OAuth2ErrorResponse {
    error: "invalid_grant",
    error_description: Some("Authorization code expired"),
    error_uri: None
}))
```

**Common Errors**:

| `error` | Meaning | Solution |
|---------|---------|----------|
| `invalid_grant` | Code expired or already used | Restart auth flow |
| `invalid_client` | Wrong client_id/secret | Verify credentials |
| `invalid_request` | Missing parameter | Check token request |
| `unauthorized_client` | Client not authorized for grant type | Update OAuth2 app config |
| `unsupported_grant_type` | Grant type not supported | Use supported grant type |

---

#### `OAuth2Error::StateMismatch`

**Description**: CSRF protection detected state parameter mismatch.

**Solutions**:

1. **Security issue**: Possible CSRF attack, abort flow
2. **Storage issue**: Verify state persisted correctly
3. **Concurrent flows**: Use unique state per flow

---

### 8.2 SAML Errors

#### `SamlError::SignatureVerificationFailed`

**Common Causes**:
- Wrong IdP certificate
- Certificate expired
- XML tampering
- Clock skew

**Solutions**:

```rust
// 1. Verify certificate matches IdP metadata
let idp_cert_pem = r#"-----BEGIN CERTIFICATE-----
MIIDXTCCAkWgAwIBAgIJAKZ...
-----END CERTIFICATE-----"#;

// 2. Check certificate validity
let cert = X509::from_pem(idp_cert_pem.as_bytes())?;
let not_before = cert.not_before();
let not_after = cert.not_after();
eprintln!("Cert valid from {} to {}", not_before, not_after);

// 3. Allow clock skew
let config = SamlConfig {
    // ...
    clock_skew_seconds: 300, // 5 minutes
};
```

See: [[OAuth2-Issues#SAML-Troubleshooting]]

---

### 8.3 LDAP Errors

#### `LdapError::BindFailed`

**Common Causes**:
- Wrong password
- Wrong DN format
- User not found
- Account locked

**Diagnosis**:

```bash
# Test LDAP bind manually
ldapwhoami -H ldaps://ldap.example.com -D "cn=alice,ou=users,dc=example,dc=com" -W
```

**Solutions**:

1. **Wrong password**: Verify credentials
2. **DN format**: Check bind_dn_template
3. **User not found**: Verify user exists in directory
4. **Account locked**: Contact LDAP administrator

---

### 8.4 mTLS Errors

#### `MtlsError::CertificateExpired`

**Diagnosis**:

```rust
use openssl::x509::X509;

let cert = X509::from_pem(cert_pem.as_bytes())?;
let not_after = cert.not_after();
let now = Utc::now();

eprintln!("Certificate expires: {}", not_after);
eprintln!("Current time: {}", now);

if now > not_after {
    eprintln!("Certificate expired, rotation required");
}
```

**Solution**: Rotate certificate using [[../Examples/Certificate-Rotation]]

---

## 9. Debugging Workflow

### Step-by-Step Diagnostic Process

1. **Identify error type**:
   ```rust
   match result {
       Err(CredentialError::OAuth2(_)) => // OAuth2 issue
       Err(CredentialError::Storage(_)) => // Storage issue
       Err(CredentialError::Encryption(_)) => // Encryption issue
       // ...
   }
   ```

2. **Enable debug logging**:
   ```rust
   use tracing_subscriber;
   
   tracing_subscriber::fmt()
       .with_max_level(tracing::Level::DEBUG)
       .init();
   ```

3. **Check credential state**:
   ```rust
   let metadata = manager.get_metadata(&id).await?;
   eprintln!("State: {:?}", tracker.current_state);
   eprintln!("Created: {}", metadata.created_at);
   eprintln!("Expires: {:?}", metadata.expires_at);
   ```

4. **Test credential**:
   ```rust
   let test_result = credential.test(&ctx).await?;
   if !test_result.success {
       eprintln!("Test failed: {}", test_result.message);
   }
   ```

5. **Consult detailed guides**:
   - [[Decryption-Failures]] for encryption errors
   - [[OAuth2-Issues]] for OAuth2/SAML/JWT errors
   - [[Provider-Connectivity]] for storage errors
   - [[Rotation-Failures]] for rotation errors
   - [[Scope-Violations]] for permission errors

---

## 10. Error Handling Best Practices

### Proper Error Propagation

```rust
use nebula_credential::prelude::*;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Credential error: {0}")]
    Credential(#[from] CredentialError),
    
    #[error("Application error: {0}")]
    Application(String),
}

pub async fn fetch_api_data(
    manager: &CredentialManager,
    api_key_id: &CredentialId,
) -> Result<ApiResponse, AppError> {
    // Retrieve credential (propagates CredentialError)
    let api_key = manager.retrieve_credential(api_key_id, &ctx).await?;
    
    // Use credential
    let response = reqwest::Client::new()
        .get("https://api.example.com/data")
        .header("X-API-Key", api_key.expose())
        .send()
        .await
        .map_err(|e| AppError::Application(e.to_string()))?;
    
    Ok(response.json().await.map_err(|e| AppError::Application(e.to_string()))?)
}
```

### Retry Logic

```rust
use tokio::time::{sleep, Duration};

pub async fn retrieve_with_retry(
    manager: &CredentialManager,
    id: &CredentialId,
    ctx: &CredentialContext,
    max_retries: u32,
) -> Result<impl Credential, CredentialError> {
    let mut attempts = 0;
    
    loop {
        match manager.retrieve_credential(id, ctx).await {
            Ok(cred) => return Ok(cred),
            Err(e) if is_retriable(&e) && attempts < max_retries => {
                attempts += 1;
                let backoff = Duration::from_secs(2u64.pow(attempts));
                eprintln!("Attempt {attempts} failed: {e}, retrying in {backoff:?}");
                sleep(backoff).await;
            }
            Err(e) => return Err(e),
        }
    }
}

fn is_retriable(error: &CredentialError) -> bool {
    matches!(
        error,
        CredentialError::Storage(StorageError::ConnectionFailed(_)) |
        CredentialError::Storage(StorageError::ReadFailed(_))
    )
}
```

### Graceful Degradation

```rust
pub async fn get_credential_or_default(
    manager: &CredentialManager,
    id: &CredentialId,
    ctx: &CredentialContext,
) -> impl Credential {
    match manager.retrieve_credential(id, ctx).await {
        Ok(cred) => cred,
        Err(CredentialError::NotFound(_)) => {
            eprintln!("Credential not found, using default");
            create_default_credential()
        }
        Err(CredentialError::Expired(_)) => {
            eprintln!("Credential expired, refreshing");
            refresh_or_default(manager, id, ctx).await
        }
        Err(e) => {
            eprintln!("Credential error: {e}, using default");
            create_default_credential()
        }
    }
}
```

---

## Related Documentation

- [[Decryption-Failures]] - Encryption and decryption troubleshooting
- [[OAuth2-Issues]] - OAuth2, SAML, JWT error resolution
- [[Rotation-Failures]] - Credential rotation problems
- [[Scope-Violations]] - Permission and ACL debugging
- [[Provider-Connectivity]] - Storage provider connection issues
- [[Debugging-Checklist]] - Systematic debugging approach
- [[../Advanced/Security-Best-Practices]] - Secure error handling
- [[../Advanced/Observability-Guide]] - Error monitoring and alerting

---

## Summary

This guide catalogs all `nebula-credential` error types with:

✅ **Quick diagnosis table** for rapid triage  
✅ **Error hierarchy reference** showing all error types  
✅ **Detailed explanations** for each error category  
✅ **Diagnostic code examples** for troubleshooting  
✅ **Solutions and workarounds** for common issues  
✅ **Best practices** for error handling  
✅ **Links to specialized guides** for deep dives  

Use the quick diagnosis table to identify your error, then follow the detailed section or linked guide for resolution.
