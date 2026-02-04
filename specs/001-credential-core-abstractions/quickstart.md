# Quick Start Guide: Core Credential Abstractions

**Feature**: 001-credential-core-abstractions  
**Date**: 2026-02-03  
**Phase**: 1 (Architecture & Contracts)

## Overview

This guide shows developers how to use the core credential abstractions in under 10 lines of code. Phase 1 provides the foundational types and traits - Phase 2 adds concrete storage providers.

## Prerequisites

Add to `Cargo.toml`:

```toml
[dependencies]
nebula-credential = { path = "../../crates/nebula-credential" }
tokio = { version = "1.49", features = ["full"] }
```

## Basic Example (10 Lines)

```rust
use nebula_credential::{
    CredentialId, SecretString, EncryptionKey,
    CredentialContext, CredentialMetadata,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create credential ID
    let id = CredentialId::new("github_token")?;
    
    // 2. Create secret (automatically zeros on drop)
    let secret = SecretString::new("ghp_xxxxxxxxxxxx");
    
    // 3. Derive encryption key from master password
    let salt = [0u8; 16]; // Load from secure storage in production
    let key = EncryptionKey::derive_from_password("master-pwd", &salt)?;
    
    // 4. Create request context
    let context = CredentialContext::new("user_123");
    
    // 5. Create metadata
    let metadata = CredentialMetadata::new();
    
    println!("Credential '{}' ready for storage", id);
    // Phase 2 adds: provider.store(&id, encrypted, metadata, &context).await?
    
    Ok(())
}
```

## Step-by-Step Explanation

### 1. Create Credential ID

```rust
let id = CredentialId::new("github_token")?;
```

**What it does**: Creates a validated credential identifier.

**Validation**: Only allows alphanumeric characters, hyphens, underscores.

**Error**: Returns `ValidationError` if ID is empty or contains invalid characters.

### 2. Create Secret String

```rust
let secret = SecretString::new("ghp_xxxxxxxxxxxx");
```

**What it does**: Wraps sensitive string data with automatic memory zeroization.

**Security**: Memory is securely erased when `secret` goes out of scope.

**Display**: Prints as `[REDACTED]` in logs/debug output.

### 3. Derive Encryption Key

```rust
let salt = [0u8; 16]; // Load from secure storage
let key = EncryptionKey::derive_from_password("master-pwd", &salt)?;
```

**What it does**: Derives 256-bit AES key from password using Argon2id.

**Performance**: Takes 100-200ms (security requirement to prevent brute force).

**Salt**: Must be stored alongside encrypted data (not secret, but must be unique).

**Error**: Returns `CryptoError::KeyDerivation` if derivation fails.

### 4. Create Request Context

```rust
let context = CredentialContext::new("user_123");
```

**What it does**: Creates request context with owner, trace ID, timestamp.

**Purpose**: Enables observability and audit logging in future phases.

**Optional scope**: Add with `.with_scope("workflow_456")` for isolation.

### 5. Create Metadata

```rust
let metadata = CredentialMetadata::new();
```

**What it does**: Creates metadata with current timestamp.

**Contents**: Creation time, last accessed (None), last modified, tags.

**Mutation**: Use `.mark_accessed()` and `.mark_modified()` to update timestamps.

## Common Patterns

### Accessing Secret Value

```rust
let secret = SecretString::new("my-secret");

// ❌ WRONG - no direct access
// let value: &str = secret.???;

// ✅ CORRECT - use closure scope
secret.expose_secret(|value| {
    println!("Secret length: {}", value.len());
    // Use value here - cannot escape closure scope
});
```

### Validating Credential IDs

```rust
// ✅ Valid IDs
CredentialId::new("github_token")?;          // OK
CredentialId::new("aws-access-key-123")?;    // OK  
CredentialId::new("db_password_prod")?;      // OK

// ❌ Invalid IDs
CredentialId::new("")?;                      // Error: EmptyCredentialId
CredentialId::new("../etc/passwd")?;         // Error: InvalidCredentialId
CredentialId::new("token with spaces")?;     // Error: InvalidCredentialId
```

### Adding Metadata Tags

```rust
let mut metadata = CredentialMetadata::new();

// Add custom tags for organization
metadata.tags.insert("environment".to_string(), "production".to_string());
metadata.tags.insert("service".to_string(), "api-gateway".to_string());
metadata.tags.insert("owner".to_string(), "platform-team".to_string());
```

### Context with Scope

```rust
// No scope (global)
let context = CredentialContext::new("user_123");

// Workflow-specific scope
let context = CredentialContext::new("user_123")
    .with_scope("workflow_456");

// Custom trace ID for distributed tracing
use uuid::Uuid;
let trace_id = Uuid::new_v4();
let context = CredentialContext::new("user_123")
    .with_trace_id(trace_id);
```

## Error Handling

### Pattern: Match on Error Type

```rust
use nebula_credential::{CredentialError, StorageError, CryptoError};

match result {
    Ok(data) => println!("Success: {:?}", data),
    Err(CredentialError::Storage { id, source }) => {
        match source {
            StorageError::NotFound { .. } => {
                eprintln!("Credential '{}' not found", id);
            }
            StorageError::PermissionDenied { .. } => {
                eprintln!("Permission denied for '{}'", id);
            }
            _ => eprintln!("Storage error: {}", source),
        }
    }
    Err(CredentialError::Crypto { source }) => {
        eprintln!("Encryption error: {}", source);
    }
    Err(e) => eprintln!("Error: {}", e),
}
```

### Pattern: Propagate with Context

```rust
use nebula_credential::{CredentialError, ValidationError};

fn validate_and_store(id_str: &str) -> Result<(), CredentialError> {
    let id = CredentialId::new(id_str)
        .map_err(|e| CredentialError::Validation { source: e })?;
    
    // Continue with storage...
    Ok(())
}
```

## Testing

### Unit Test Example

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_credential_id_validation() {
        // Valid ID
        assert!(CredentialId::new("valid_id").is_ok());
        
        // Invalid IDs
        assert!(CredentialId::new("").is_err());
        assert!(CredentialId::new("../path").is_err());
        assert!(CredentialId::new("with spaces").is_err());
    }
    
    #[test]
    fn test_secret_string_redacted() {
        let secret = SecretString::new("sensitive");
        let debug_str = format!("{:?}", secret);
        assert_eq!(debug_str, "[REDACTED]");
    }
    
    #[tokio::test]
    async fn test_key_derivation_deterministic() {
        let salt = [0u8; 16];
        let key1 = EncryptionKey::derive_from_password("pwd", &salt).unwrap();
        let key2 = EncryptionKey::derive_from_password("pwd", &salt).unwrap();
        // Keys should be identical for same password + salt
        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }
}
```

### Integration Test with Mock Provider (Phase 2)

```rust
#[tokio::test]
async fn test_store_and_retrieve() {
    let provider = MockStorageProvider::new();
    let id = CredentialId::new("test_cred").unwrap();
    let context = CredentialContext::new("test_user");
    
    // Store
    provider.store(&id, encrypted_data, metadata, &context).await.unwrap();
    
    // Retrieve
    let (data, meta) = provider.retrieve(&id, &context).await.unwrap();
    assert_eq!(data.version, 1);
}
```

## Performance Tips

### Key Derivation is Expensive

```rust
// ❌ WRONG - derive key per operation
for credential in credentials {
    let key = EncryptionKey::derive_from_password("pwd", &salt)?; // 100ms each!
    encrypt(&key, &credential)?;
}

// ✅ CORRECT - derive once, reuse
let key = EncryptionKey::derive_from_password("pwd", &salt)?; // 100ms once
for credential in credentials {
    encrypt(&key, &credential)?; // <1ms each
}
```

### Minimize Secret Exposure

```rust
// ❌ WRONG - copying secret
let secret = SecretString::new("value");
let leaked = secret.expose_secret(|s| s.to_string()); // Copies! Not zeroized!

// ✅ CORRECT - process in place
let secret = SecretString::new("value");
secret.expose_secret(|s| {
    // Do work with s here
    validate_secret_format(s)
}); // Secret never leaves closure
```

## Next Steps

After understanding Phase 1 basics:

1. **Phase 2**: Use `LocalStorageProvider` to actually persist credentials
2. **Phase 2**: Switch to cloud providers (AWS, Azure, Vault) with 1 config change
3. **Phase 3**: Add caching layer for sub-millisecond retrieval
4. **Phase 4**: Enable automatic credential rotation
5. **Phase 5**: Use protocol-specific credentials (OAuth2, SAML, etc.)

## Common Issues

### Issue: Key derivation too slow

**Problem**: Argon2id takes 100-200ms per derivation.

**Solution**: Derive key once at initialization, store in memory for the session. Do NOT derive per-operation.

### Issue: Secret appears in logs

**Problem**: Using `format!("{:?}", secret)` or `println!` with secret.

**Solution**: SecretString automatically redacts. Verify you're using SecretString, not raw String.

### Issue: Credential ID validation fails

**Problem**: ID contains spaces, slashes, or other special characters.

**Solution**: Use only alphanumeric characters, hyphens, underscores. Transform user input before validation.

### Issue: Salt reuse across credentials

**Problem**: Using same salt for multiple credentials.

**Solution**: Generate unique salt per credential. Salt is not secret but MUST be unique.

## References

- Feature Specification: [spec.md](./spec.md)
- Data Model: [data-model.md](./data-model.md)
- StorageProvider Contract: [contracts/storage-provider-trait.md](./contracts/storage-provider-trait.md)
- Full Documentation: `crates/nebula-credential/docs/`
