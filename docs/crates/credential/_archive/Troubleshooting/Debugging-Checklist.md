---
title: Debugging Checklist
description: Systematic debugging approach for nebula-credential issues with step-by-step diagnostic procedures
tags: [troubleshooting, debugging, diagnostics, checklist, workflow]
related:
  - "[[Common-Errors]]"
  - "[[Decryption-Failures]]"
  - "[[OAuth2-Issues]]"
  - "[[Rotation-Failures]]"
  - "[[Scope-Violations]]"
  - "[[Provider-Connectivity]]"
  - "[[../Advanced/Observability-Guide]]"
status: published
version: 1.0.0
---

# Debugging Checklist

Systematic debugging approach for diagnosing and resolving `nebula-credential` issues.

---

## Debugging Workflow

```
1. Identify Error
   ↓
2. Gather Context
   ↓
3. Reproduce Issue
   ↓
4. Isolate Root Cause
   ↓
5. Apply Solution
   ↓
6. Verify Fix
   ↓
7. Document Resolution
```

---

## 1. Initial Triage

### 1.1 Error Identification

**Checklist**:

- [ ] What is the exact error message?
- [ ] What is the error type? (`CredentialError::*`, `StorageError::*`, etc.)
- [ ] When did the error first occur?
- [ ] Is the error consistent or intermittent?
- [ ] Can you reproduce the error?

**Quick Reference**:

```rust
match result {
    Err(e) => {
        eprintln!("Error type: {:?}", std::any::type_name_of_val(&e));
        eprintln!("Error message: {e}");
        eprintln!("Error debug: {e:?}");
        
        // Categorize error
        match e {
            CredentialError::AuthenticationFailed(_) => eprintln!("→ See [[OAuth2-Issues]]"),
            CredentialError::Encryption(_) => eprintln!("→ See [[Decryption-Failures]]"),
            CredentialError::Storage(_) => eprintln!("→ See [[Provider-Connectivity]]"),
            CredentialError::PermissionDenied(_) => eprintln!("→ See [[Scope-Violations]]"),
            CredentialError::Expired(_) => eprintln!("→ See [[Rotation-Failures]]"),
            _ => eprintln!("→ See [[Common-Errors]]"),
        }
    }
    Ok(_) => {}
}
```

---

### 1.2 Context Gathering

**Checklist**:

- [ ] What operation was being performed?
- [ ] What is the credential type? (OAuth2, API Key, Database, etc.)
- [ ] What is the storage provider? (AWS, Azure, Vault, K8s, local)
- [ ] What is the deployment environment? (dev, staging, prod)
- [ ] What changed recently? (code, config, infrastructure)

**Diagnostic Commands**:

```rust
use nebula_credential::prelude::*;

// Gather credential metadata
let metadata = manager.get_metadata(&id).await?;
eprintln!("Credential Info:");
eprintln!("  ID: {}", metadata.id);
eprintln!("  Type: {}", metadata.credential_type);
eprintln!("  Owner: {}", metadata.owner_id);
eprintln!("  Scope: {:?}", metadata.scope_id);
eprintln!("  Created: {}", metadata.created_at);
eprintln!("  Updated: {}", metadata.updated_at);
eprintln!("  Expires: {:?}", metadata.expires_at);

// Gather state info
let state = manager.get_state(&id).await?;
eprintln!("\nState Info:");
eprintln!("  Current: {:?}", state.current_state);
eprintln!("  History length: {}", state.history.len());
if let Some(last) = state.history.last() {
    eprintln!("  Last transition: {} → {} at {}",
        last.from, last.to, last.timestamp);
}

// Gather system info
eprintln!("\nSystem Info:");
eprintln!("  nebula-credential version: {}", env!("CARGO_PKG_VERSION"));
eprintln!("  OS: {}", std::env::consts::OS);
eprintln!("  Arch: {}", std::env::consts::ARCH);
```

---

## 2. Reproduction

### 2.1 Minimal Reproduction

**Goal**: Create smallest possible test case that reproduces the error

**Template**:

```rust
use nebula_credential::prelude::*;

#[tokio::test]
async fn reproduce_error() -> Result<(), CredentialError> {
    // 1. Setup minimal environment
    let manager = CredentialManager::new_local("test.db").await?;
    
    // 2. Create minimal credential
    let api_key = ApiKeyCredential::new("test-key-123");
    let ctx = CredentialContext::new(OwnerId::new("test-user"));
    let id = CredentialId::new();
    
    // 3. Reproduce error
    let result = manager.store_credential(&id, &api_key, &ctx).await;
    
    // 4. Verify error occurs
    assert!(result.is_err(), "Expected error but got Ok");
    
    // 5. Inspect error
    match result {
        Err(e) => {
            eprintln!("Successfully reproduced error: {e}");
            eprintln!("Error type: {e:?}");
        }
        Ok(_) => panic!("Error not reproduced"),
    }
    
    Ok(())
}
```

---

### 2.2 Isolation Testing

**Checklist**:

- [ ] Does error occur with different credential types?
- [ ] Does error occur with different storage providers?
- [ ] Does error occur in different environments?
- [ ] Does error occur with different users/scopes?
- [ ] Does error occur with minimal configuration?

**Example**:

```rust
use nebula_credential::preopen::*;

// Test 1: Different credential type
#[tokio::test]
async fn test_with_oauth2() {
    // ... test with OAuth2 ...
}

#[tokio::test]
async fn test_with_api_key() {
    // ... test with API key ...
}

// Test 2: Different storage provider
#[tokio::test]
async fn test_with_local_storage() {
    let manager = CredentialManager::new_local("test.db").await?;
    // ...
}

#[tokio::test]
async fn test_with_aws() {
    let manager = CredentialManager::new_aws(&aws_config).await?;
    // ...
}

// Test 3: Minimal vs full configuration
#[tokio::test]
async fn test_minimal_config() {
    let config = CredentialSystemConfig::default();
    // ...
}

#[tokio::test]
async fn test_full_config() {
    let config = CredentialSystemConfig {
        encryption: EncryptionConfig { /* custom */ },
        storage: StorageConfig { /* custom */ },
        // ...
    };
    // ...
}
```

---

## 3. Root Cause Analysis

### 3.1 Debugging Tools

**Enable Debug Logging**:

```rust
use tracing_subscriber;

// Enable all debug logs
tracing_subscriber::fmt()
    .with_max_level(tracing::Level::DEBUG)
    .with_target(true)
    .with_thread_ids(true)
    .with_file(true)
    .with_line_number(true)
    .init();

// Or specific module
tracing_subscriber::fmt()
    .with_env_filter("nebula_credential=debug")
    .init();
```

**Instrument Functions**:

```rust
use tracing::{debug, error, info, instrument};

#[instrument(skip(manager, credential))]
pub async fn store_with_logging(
    manager: &CredentialManager,
    id: &CredentialId,
    credential: &impl Credential,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    info!("Storing credential {id}");
    debug!("Credential type: {}", credential.credential_type());
    debug!("Owner: {}", ctx.owner_id);
    
    match manager.store_credential(id, credential, ctx).await {
        Ok(_) => {
            info!("Successfully stored credential {id}");
            Ok(())
        }
        Err(e) => {
            error!("Failed to store credential {id}: {e}");
            Err(e)
        }
    }
}
```

---

### 3.2 State Inspection

**Checklist**:

- [ ] What is the credential state?
- [ ] What is the state history?
- [ ] Are there any invalid state transitions?
- [ ] Is the credential in grace period?
- [ ] Is the credential expired?

**Tool**:

```rust
use nebula_credential::prelude::*;

pub async fn inspect_credential_state(
    manager: &CredentialManager,
    id: &CredentialId,
) -> Result<(), CredentialError> {
    eprintln!("=== Credential State Inspection ===\n");
    
    let state = manager.get_state(id).await?;
    
    eprintln!("Current State: {:?}", state.current_state);
    eprintln!("Is Usable: {}", state.is_usable());
    
    eprintln!("\nState History ({} transitions):", state.history.len());
    for (i, transition) in state.history.iter().enumerate() {
        eprintln!("  {}. {} → {} at {}",
            i + 1,
            transition.from,
            transition.to,
            transition.timestamp
        );
        eprintln!("     Reason: {}", transition.reason);
        eprintln!("     By: {}", transition.triggered_by);
    }
    
    // Check for issues
    if state.current_state == CredentialState::Expired {
        eprintln!("\n⚠️  Credential is expired");
        let metadata = manager.get_metadata(id).await?;
        if let Some(expires_at) = metadata.expires_at {
            eprintln!("   Expired at: {expires_at}");
            let duration = Utc::now().signed_duration_since(expires_at);
            eprintln!("   Expired {} ago", format_duration(duration));
        }
    }
    
    if state.current_state == CredentialState::Invalid {
        eprintln!("\n⚠️  Credential is invalid");
        eprintln!("   Last test failed");
        eprintln!("   Re-authentication may be required");
    }
    
    Ok(())
}
```

---

### 3.3 Network Debugging

**For storage/authentication errors**:

```bash
# Enable network tracing
export RUST_LOG=reqwest=debug,hyper=debug

# Capture network traffic
tcpdump -i any -w capture.pcap port 443

# Analyze with Wireshark
wireshark capture.pcap
```

**Proxy debugging**:

```bash
# Use proxy for inspection
export HTTP_PROXY=http://localhost:8080
export HTTPS_PROXY=http://localhost:8080

# Run mitmproxy
mitmproxy --mode regular --listen-port 8080
```

---

## 4. Common Issue Checklists

### 4.1 Decryption Failures

- [ ] Is the encryption key correct?
- [ ] Has key rotation occurred?
- [ ] Is the encrypted data corrupted?
- [ ] Is the nonce valid?
- [ ] Is the encryption version supported?

**Diagnostic**: See [[Decryption-Failures#Root-Cause-Analysis]]

---

### 4.2 OAuth2 Failures

- [ ] Is the authorization code expired?
- [ ] Is the state parameter matching?
- [ ] Is the client_id/client_secret correct?
- [ ] Is the redirect_uri whitelisted?
- [ ] Is PKCE configured correctly?

**Diagnostic**: See [[OAuth2-Issues#OAuth2-Errors]]

---

### 4.3 Rotation Failures

- [ ] Does the new credential work?
- [ ] Is there a grace period configured?
- [ ] Was validation performed before commit?
- [ ] Is rollback available?
- [ ] Are connections drained?

**Diagnostic**: See [[Rotation-Failures#New-Credential-Creation-Failures]]

---

### 4.4 Permission Failures

- [ ] Is the user the owner?
- [ ] Does the ACL grant required permission?
- [ ] Is the scope correct?
- [ ] Is the principal active?
- [ ] Was permission recently revoked?

**Diagnostic**: See [[Scope-Violations#Permission-Denied-Errors]]

---

### 4.5 Provider Connectivity

- [ ] Is the provider reachable (network)?
- [ ] Are credentials/tokens valid?
- [ ] Are permissions/policies configured?
- [ ] Is TLS/SSL working?
- [ ] Is the provider healthy?

**Diagnostic**: See [[Provider-Connectivity]]

---

## 5. Resolution Verification

### 5.1 Test Fix

**Checklist**:

- [ ] Does the original error no longer occur?
- [ ] Do all related tests pass?
- [ ] Does the fix work in all environments?
- [ ] Are there any regressions?
- [ ] Is performance acceptable?

**Verification Script**:

```rust
use nebula_credential::prelude::*;

pub async fn verify_fix(
    manager: &CredentialManager,
    id: &CredentialId,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    eprintln!("=== Fix Verification ===\n");
    
    // 1. Test original operation
    eprintln!("Step 1: Testing original operation...");
    match manager.retrieve_credential(id, ctx).await {
        Ok(cred) => eprintln!("✓ Retrieve successful"),
        Err(e) => {
            eprintln!("✗ Retrieve failed: {e}");
            return Err(e);
        }
    }
    
    // 2. Test credential functionality
    eprintln!("\nStep 2: Testing credential...");
    let credential = manager.retrieve_credential(id, ctx).await?;
    match credential.test(ctx).await {
        Ok(result) if result.success => eprintln!("✓ Test successful"),
        Ok(result) => {
            eprintln!("✗ Test failed: {}", result.message);
            return Err(CredentialError::TestFailed(result.message));
        }
        Err(e) => {
            eprintln!("✗ Test error: {e}");
            return Err(e);
        }
    }
    
    // 3. Test related operations
    eprintln!("\nStep 3: Testing related operations...");
    
    // List
    let filter = CredentialFilter::new().owner(ctx.owner_id.clone());
    match manager.list_credentials(Some(&filter)).await {
        Ok(list) => eprintln!("✓ List successful ({} credentials)", list.len()),
        Err(e) => eprintln!("✗ List failed: {e}"),
    }
    
    // Update metadata
    let metadata = manager.get_metadata(id).await?;
    match manager.update_metadata(id, &metadata).await {
        Ok(_) => eprintln!("✓ Update metadata successful"),
        Err(e) => eprintln!("✗ Update metadata failed: {e}"),
    }
    
    eprintln!("\n=== Fix Verified ===");
    
    Ok(())
}
```

---

### 5.2 Regression Testing

**Run full test suite**:

```bash
# Unit tests
cargo test --lib

# Integration tests
cargo test --test '*'

# Specific provider tests
cargo test --features aws
cargo test --features azure
cargo test --features vault
cargo test --features kubernetes

# All features
cargo test --all-features
```

---

## 6. Documentation

### 6.1 Incident Report Template

```markdown
## Incident Summary

**Date**: 2026-02-03
**Reporter**: Alice
**Severity**: High
**Status**: Resolved

## Problem Description

Credential retrieval failing with `DecryptionFailed` error for all credentials in production.

## Root Cause

Key rotation occurred without migrating existing credentials to new key.

## Timeline

- 10:00 - Key rotation deployed
- 10:05 - First decryption failures reported
- 10:10 - Investigation started
- 10:20 - Root cause identified
- 10:25 - Fix deployed (added old key to rotation manager)
- 10:30 - All credentials working
- 10:45 - Migration to new key completed

## Solution

1. Added old encryption key to KeyRotationManager
2. Decrypted credentials with old key
3. Re-encrypted with new key
4. Verified all credentials working

## Prevention

1. Always maintain key rotation history
2. Test decryption before removing old keys
3. Add monitoring for decryption failures

## Related Documentation

- [[Decryption-Failures#Wrong-Encryption-Key]]
- [[../Advanced/Key-Management#Key-Rotation]]
```

---

### 6.2 Knowledge Base Entry

**Template**:

```markdown
## Error: OAuth2 StateMismatch

**Category**: Authentication
**Frequency**: Uncommon
**Severity**: Medium

### Symptoms

- `OAuth2Error::StateMismatch` during callback
- OAuth2 flow fails after redirect

### Root Causes

1. State not persisted correctly
2. State expired
3. Multiple concurrent flows

### Diagnostic Steps

1. Check state storage implementation
2. Verify state expiration time
3. Enable debug logging for OAuth2 flow

### Solutions

- Use persistent state storage (see code example)
- Increase state expiration to 10 minutes
- Use unique state per flow

### Related

- [[OAuth2-Issues#StateMismatch]]
- [[../Examples/OAuth2-Flow]]
```

---

## 7. Quick Reference Commands

### 7.1 Diagnostic Commands

```bash
# Enable all debug logging
export RUST_LOG=debug

# Enable specific module
export RUST_LOG=nebula_credential::storage=debug

# Test credential manager
cargo run --example test-credential -- --id cred-123

# Check provider connectivity
cargo run --example test-provider -- --provider aws

# Inspect database
sqlite3 credentials.db "SELECT * FROM credentials;"

# Check Vault status
vault status

# Check AWS credentials
aws sts get-caller-identity

# Check Azure credentials
az account show

# Check Kubernetes access
kubectl auth can-i get secrets
```

---

### 7.2 Recovery Commands

```bash
# Rollback rotation
cargo run --example rollback-rotation -- --id cred-123

# Re-encrypt with new key
cargo run --example re-encrypt -- --all

# Restore from backup
cargo run --example restore -- --id cred-123 --backup latest

# Clear cache
cargo run --example clear-cache -- --all

# Reset state
cargo run --example reset-state -- --id cred-123
```

---

## Related Documentation

- [[Common-Errors]] - All error types
- [[Decryption-Failures]] - Encryption debugging
- [[OAuth2-Issues]] - Authentication debugging
- [[Rotation-Failures]] - Rotation debugging
- [[Scope-Violations]] - Permission debugging
- [[Provider-Connectivity]] - Storage debugging
- [[../Advanced/Observability-Guide]] - Monitoring and metrics

---

## Summary

This checklist provides:

✅ **Systematic workflow** for debugging  
✅ **Triage procedures** for quick categorization  
✅ **Reproduction techniques** for isolation  
✅ **Root cause analysis** tools  
✅ **Issue-specific checklists** for common problems  
✅ **Verification procedures** for fixes  
✅ **Documentation templates** for knowledge sharing  
✅ **Quick reference commands** for diagnostics  

Follow the workflow systematically: Identify → Gather Context → Reproduce → Isolate → Solve → Verify → Document.
