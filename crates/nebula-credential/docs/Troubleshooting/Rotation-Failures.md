---
title: Rotation Failures
description: Troubleshooting guide for credential rotation failures with rollback procedures
tags: [troubleshooting, rotation, rollback, credentials, recovery]
related:
  - "[[Common-Errors]]"
  - "[[../How-To/Rotate-Credentials]]"
  - "[[../Advanced/Rotation-Policies]]"
  - "[[../Examples/Database-Rotation]]"
  - "[[../Examples/API-Key-Rotation]]"
  - "[[../Examples/Certificate-Rotation]]"
  - "[[../Examples/OAuth2-Token-Refresh]]"
  - "[[Debugging-Checklist]]"
status: published
version: 1.0.0
---

# Rotation Failures

Comprehensive troubleshooting for credential rotation failures, including rollback procedures and recovery strategies.

---

## Overview

**Rotation Strategies**:
1. **Periodic Rotation**: Rotate at fixed intervals (e.g., every 90 days)
2. **Before-Expiry Rotation**: Rotate before credential expires
3. **Scheduled Rotation**: Rotate at specific times
4. **Manual Rotation**: On-demand rotation

**Failure Modes**:
- New credential creation fails
- Old credential revocation fails
- Grace period not honored
- Rollback fails
- Zero-downtime requirement violated

---

## Quick Diagnosis

```
Rotation Failed
├─ New credential failed? → Check remote service, retry
├─ Old credential revoked prematurely? → Activate grace period, rollback
├─ Grace period expired? → Extend grace period, re-rotate
└─ Rollback failed? → Manual recovery required
```

---

## 1. New Credential Creation Failures

### 1.1 Remote Service Rejected Rotation

**Symptom**: New credential creation returns error

**Common Causes**:
- Rate limit exceeded
- Insufficient permissions
- Service outage
- Invalid rotation request

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

match credential.rotate(&current, &policy, &ctx).await {
    Err(CredentialError::AuthenticationFailed(msg)) if msg.contains("rate limit") => {
        eprintln!("⚠️  Rate limit exceeded");
        eprintln!("   Recommendation: Retry with exponential backoff");
    }
    Err(CredentialError::PermissionDenied(msg)) => {
        eprintln!("⚠️  Insufficient permissions for rotation");
        eprintln!("   Required: rotate_credentials permission");
        eprintln!("   Error: {msg}");
    }
    Err(CredentialError::AuthenticationFailed(msg)) if msg.contains("503") || msg.contains("unavailable") => {
        eprintln!("⚠️  Service temporarily unavailable");
        eprintln!("   Recommendation: Retry later");
    }
    Err(e) => eprintln!("Rotation error: {e}"),
    Ok(new_cred) => println!("Rotation successful"),
}
```

**Solutions**:

1. **Retry with exponential backoff**:

```rust
use nebula_credential::prelude::*;
use tokio::time::{sleep, Duration};

pub async fn rotate_with_retry(
    credential: &impl RotatableCredential,
    current: &<impl RotatableCredential as Credential>::Output,
    policy: &<impl RotatableCredential as RotatableCredential>::Policy,
    ctx: &CredentialContext,
    max_retries: u32,
) -> Result<<impl RotatableCredential as Credential>::Output, CredentialError> {
    let mut attempts = 0;
    
    loop {
        match credential.rotate(current, policy, ctx).await {
            Ok(new_cred) => {
                eprintln!("✓ Rotation successful on attempt {}", attempts + 1);
                return Ok(new_cred);
            }
            Err(e) if is_retriable(&e) && attempts < max_retries => {
                attempts += 1;
                let backoff = Duration::from_secs(2u64.pow(attempts));
                eprintln!("Attempt {attempts} failed: {e}");
                eprintln!("Retrying in {backoff:?}...");
                sleep(backoff).await;
            }
            Err(e) => {
                eprintln!("✗ Rotation failed after {attempts} attempts");
                return Err(e);
            }
        }
    }
}

fn is_retriable(error: &CredentialError) -> bool {
    matches!(
        error,
        CredentialError::AuthenticationFailed(msg) if msg.contains("rate limit") ||
            msg.contains("503") ||
            msg.contains("unavailable") ||
            msg.contains("timeout")
    )
}
```

2. **Check permissions before rotation**:

```rust
use nebula_credential::prelude::*;

pub async fn check_rotation_permissions(
    credential: &impl RotatableCredential,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    // Verify ACL allows rotation
    let acl = get_credential_acl(&credential_id).await?;
    
    if !acl.has_permission(&ctx.owner_id.as_str(), Permission::Rotate) {
        return Err(CredentialError::PermissionDenied(
            format!("User {} lacks rotate permission", ctx.owner_id)
        ));
    }
    
    eprintln!("✓ Rotation permission verified");
    Ok(())
}
```

---

### 1.2 Grace Period Configuration Missing

**Symptom**: Old credential immediately revoked, causing downtime

**Diagnosis**:

```rust
use nebula_credential::prelude::*;

let config = RotationConfig {
    grace_period: Duration::from_secs(0), // ⚠️  No grace period!
    // ...
};

eprintln!("Rotation config:");
eprintln!("  Grace period: {:?}", config.grace_period);

if config.grace_period.as_secs() == 0 {
    eprintln!("⚠️  WARNING: No grace period configured");
    eprintln!("   Old credential will be revoked immediately");
    eprintln!("   This may cause service disruption");
    eprintln!("   Recommendation: Set grace period to 5-15 minutes");
}
```

**Solution**: Configure grace period

```rust
use nebula_credential::prelude::*;

pub struct RotationConfig {
    pub grace_period: Duration,
    pub auto_rollback_on_failure: bool,
    pub validate_new_credential: bool,
}

impl Default for RotationConfig {
    fn default() -> Self {
        Self {
            grace_period: Duration::from_secs(300), // 5 minutes
            auto_rollback_on_failure: true,
            validate_new_credential: true,
        }
    }
}

pub async fn rotate_with_grace_period(
    manager: &CredentialManager,
    id: &CredentialId,
    policy: &RotationPolicy,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    let config = RotationConfig::default();
    
    // 1. Create new credential
    eprintln!("Creating new credential...");
    let current = manager.retrieve_credential(id, ctx).await?;
    let new_cred = current.rotate(&current, policy, ctx).await?;
    
    // 2. Validate new credential
    if config.validate_new_credential {
        eprintln!("Validating new credential...");
        match new_cred.test(ctx).await {
            Ok(test_result) if test_result.success => {
                eprintln!("✓ New credential validated");
            }
            Ok(test_result) => {
                eprintln!("✗ New credential validation failed: {}", test_result.message);
                return Err(CredentialError::TestFailed(test_result.message));
            }
            Err(e) => {
                eprintln!("✗ Validation error: {e}");
                return Err(e);
            }
        }
    }
    
    // 3. Store new credential
    manager.store_credential(id, &new_cred, ctx).await?;
    eprintln!("✓ New credential stored");
    
    // 4. Grace period - keep old credential valid
    eprintln!("Grace period: {:?}", config.grace_period);
    eprintln!("Old credential remains valid until: {}",
        Utc::now() + chrono::Duration::from_std(config.grace_period).unwrap());
    
    // Mark credential in grace period state
    let mut state = manager.get_state(id).await?;
    state.transition(
        CredentialState::GracePeriod,
        "Rotation grace period".to_string(),
        ctx.owner_id.as_str().to_string(),
    )?;
    
    // 5. Schedule old credential revocation
    tokio::spawn(async move {
        sleep(config.grace_period).await;
        
        eprintln!("Grace period expired, revoking old credential");
        // Revoke old credential
        // ...
    });
    
    Ok(())
}
```

---

## 2. Rollback Procedures

### 2.1 Automatic Rollback on Failure

**Implementation**:

```rust
use nebula_credential::prelude::*;

pub struct RotationTransaction {
    credential_id: CredentialId,
    old_credential: Option<Box<dyn Credential<Output = (), Error = CredentialError>>>,
    new_credential: Option<Box<dyn Credential<Output = (), Error = CredentialError>>>,
    state: TransactionState,
}

#[derive(Debug, Clone, Copy)]
enum TransactionState {
    Started,
    NewCredentialCreated,
    NewCredentialValidated,
    OldCredentialBackedUp,
    Committed,
    RolledBack,
}

impl RotationTransaction {
    pub async fn execute<C>(
        &mut self,
        manager: &CredentialManager,
        credential: &C,
        policy: &C::Policy,
        ctx: &CredentialContext,
    ) -> Result<C::Output, CredentialError>
    where
        C: RotatableCredential,
    {
        self.state = TransactionState::Started;
        eprintln!("=== Rotation Transaction Started ===");
        
        // Step 1: Retrieve current credential
        let current = manager.retrieve_credential(&self.credential_id, ctx).await?;
        self.old_credential = Some(Box::new(current.clone()));
        self.state = TransactionState::OldCredentialBackedUp;
        eprintln!("✓ Old credential backed up");
        
        // Step 2: Create new credential
        let new_cred = match credential.rotate(&current, policy, ctx).await {
            Ok(cred) => cred,
            Err(e) => {
                eprintln!("✗ New credential creation failed: {e}");
                self.rollback(manager, ctx).await?;
                return Err(e);
            }
        };
        self.new_credential = Some(Box::new(new_cred.clone()));
        self.state = TransactionState::NewCredentialCreated;
        eprintln!("✓ New credential created");
        
        // Step 3: Validate new credential
        match new_cred.test(ctx).await {
            Ok(test_result) if test_result.success => {
                eprintln!("✓ New credential validated");
            }
            Ok(test_result) => {
                eprintln!("✗ Validation failed: {}", test_result.message);
                self.rollback(manager, ctx).await?;
                return Err(CredentialError::TestFailed(test_result.message));
            }
            Err(e) => {
                eprintln!("✗ Validation error: {e}");
                self.rollback(manager, ctx).await?;
                return Err(e);
            }
        }
        self.state = TransactionState::NewCredentialValidated;
        
        // Step 4: Store new credential
        match manager.store_credential(&self.credential_id, &new_cred, ctx).await {
            Ok(_) => {
                eprintln!("✓ New credential stored");
            }
            Err(e) => {
                eprintln!("✗ Storage failed: {e}");
                self.rollback(manager, ctx).await?;
                return Err(e.into());
            }
        }
        
        // Step 5: Commit
        self.state = TransactionState::Committed;
        eprintln!("✓ Rotation committed");
        eprintln!("=== Rotation Transaction Complete ===");
        
        Ok(new_cred)
    }
    
    async fn rollback(
        &mut self,
        manager: &CredentialManager,
        ctx: &CredentialContext,
    ) -> Result<(), CredentialError> {
        eprintln!("=== Rollback Started ===");
        eprintln!("Transaction state: {:?}", self.state);
        
        match self.state {
            TransactionState::Started => {
                eprintln!("No changes to rollback");
            }
            TransactionState::OldCredentialBackedUp => {
                eprintln!("Old credential still active, no rollback needed");
            }
            TransactionState::NewCredentialCreated |
            TransactionState::NewCredentialValidated => {
                // Restore old credential
                if let Some(old_cred) = &self.old_credential {
                    eprintln!("Restoring old credential...");
                    manager.store_credential(&self.credential_id, old_cred.as_ref(), ctx).await?;
                    eprintln!("✓ Old credential restored");
                }
                
                // Revoke new credential if created remotely
                if let Some(new_cred) = &self.new_credential {
                    eprintln!("Revoking new credential...");
                    // Implementation depends on credential type
                    // revoke_credential(new_cred).await?;
                    eprintln!("✓ New credential revoked");
                }
            }
            TransactionState::Committed => {
                eprintln!("⚠️  Cannot rollback committed transaction");
                return Err(CredentialError::InvalidFormat(
                    "Transaction already committed".to_string()
                ));
            }
            TransactionState::RolledBack => {
                eprintln!("Transaction already rolled back");
            }
        }
        
        self.state = TransactionState::RolledBack;
        eprintln!("=== Rollback Complete ===");
        
        Ok(())
    }
}
```

---

### 2.2 Manual Rollback

**Scenario**: Rotation completed but new credential doesn't work in production

**Procedure**:

```rust
use nebula_credential::prelude::*;

pub async fn manual_rollback(
    manager: &CredentialManager,
    id: &CredentialId,
    ctx: &CredentialContext,
) -> Result<(), CredentialError> {
    eprintln!("=== Manual Rollback Procedure ===\n");
    
    // Step 1: Verify old credential still in grace period
    eprintln!("Step 1: Checking grace period status...");
    let state = manager.get_state(id).await?;
    
    if state.current_state != CredentialState::GracePeriod {
        eprintln!("⚠️  Credential not in grace period (state: {:?})", state.current_state);
        eprintln!("   Old credential may already be revoked");
        eprintln!("   Recovery may require re-authentication");
    } else {
        eprintln!("✓ Credential in grace period");
    }
    
    // Step 2: Retrieve rotation history
    eprintln!("\nStep 2: Retrieving rotation history...");
    let history = manager.get_rotation_history(id).await?;
    
    if history.is_empty() {
        eprintln!("✗ No rotation history found");
        return Err(CredentialError::NotFound(id.clone()));
    }
    
    let last_rotation = history.last().unwrap();
    eprintln!("Last rotation:");
    eprintln!("  Timestamp: {}", last_rotation.rotated_at);
    eprintln!("  New version: {}", last_rotation.new_version);
    eprintln!("  Old version: {}", last_rotation.old_version);
    
    // Step 3: Restore old credential
    eprintln!("\nStep 3: Restoring old credential...");
    
    if let Some(old_cred_encrypted) = manager.get_previous_version(id, last_rotation.old_version).await? {
        let old_cred = manager.decrypt_credential(&old_cred_encrypted).await?;
        
        // Validate old credential still works
        match old_cred.test(ctx).await {
            Ok(test_result) if test_result.success => {
                eprintln!("✓ Old credential still valid");
            }
            Ok(test_result) => {
                eprintln!("⚠️  Old credential validation failed: {}", test_result.message);
                eprintln!("   Proceeding anyway (may require re-authentication later)");
            }
            Err(e) => {
                eprintln!("⚠️  Old credential test error: {e}");
            }
        }
        
        // Store old credential as current
        manager.store_credential(id, &old_cred, ctx).await?;
        eprintln!("✓ Old credential restored as current");
        
    } else {
        eprintln!("✗ Old credential not found in history");
        return Err(CredentialError::NotFound(id.clone()));
    }
    
    // Step 4: Revoke new credential
    eprintln!("\nStep 4: Revoking new (failed) credential...");
    // Implementation depends on credential type
    eprintln!("✓ New credential marked for revocation");
    
    // Step 5: Update state
    eprintln!("\nStep 5: Updating credential state...");
    let mut state = manager.get_state(id).await?;
    state.transition(
        CredentialState::Active,
        "Manual rollback completed".to_string(),
        ctx.owner_id.as_str().to_string(),
    )?;
    eprintln!("✓ State updated to Active");
    
    eprintln!("\n=== Manual Rollback Complete ===");
    eprintln!("Old credential restored and active");
    eprintln!("Monitor application logs to verify functionality");
    
    Ok(())
}
```

**Usage**:

```bash
# Trigger manual rollback
cargo run --example manual-rollback -- \
    --credential-id "cred-12345" \
    --owner-id "alice"
```

---

## 3. Zero-Downtime Rotation Failures

### 3.1 Blue-Green Rotation Pattern

**Issue**: Application uses new credential before it's fully propagated

**Solution**: Staggered deployment

```rust
use nebula_credential::prelude::*;

pub struct BlueGreenRotation {
    pub blue_credential: DatabaseCredential,  // Current/old
    pub green_credential: Option<DatabaseCredential>,  // New
    pub active: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Blue,
    Green,
}

impl BlueGreenRotation {
    pub async fn rotate(
        &mut self,
        pool: &ConnectionPool,
    ) -> Result<(), CredentialError> {
        eprintln!("=== Blue-Green Rotation ===");
        
        // Step 1: Create green credential
        eprintln!("Step 1: Creating green credential...");
        let green = self.blue_credential.rotate(&self.blue_credential, &policy, &ctx).await?;
        self.green_credential = Some(green.clone());
        eprintln!("✓ Green credential created");
        
        // Step 2: Test green credential
        eprintln!("\nStep 2: Testing green credential...");
        match green.test(&ctx).await {
            Ok(test_result) if test_result.success => {
                eprintln!("✓ Green credential validated");
            }
            Ok(test_result) => {
                eprintln!("✗ Green validation failed: {}", test_result.message);
                self.green_credential = None;
                return Err(CredentialError::TestFailed(test_result.message));
            }
            Err(e) => {
                eprintln!("✗ Green test error: {e}");
                self.green_credential = None;
                return Err(e);
            }
        }
        
        // Step 3: Gradual traffic shift
        eprintln!("\nStep 3: Gradual traffic shift...");
        let shift_percentages = vec![10, 25, 50, 75, 100];
        
        for percentage in shift_percentages {
            eprintln!("\n  Shifting {}% traffic to green...", percentage);
            
            // Update load balancer/connection pool weights
            pool.set_weights(Color::Blue, 100 - percentage).await?;
            pool.set_weights(Color::Green, percentage).await?;
            
            // Monitor for 30 seconds
            eprintln!("  Monitoring for 30 seconds...");
            tokio::time::sleep(Duration::from_secs(30)).await;
            
            // Check error rates
            let error_rate = pool.get_error_rate(Color::Green).await?;
            eprintln!("  Green error rate: {:.2}%", error_rate * 100.0);
            
            if error_rate > 0.05 {  // 5% threshold
                eprintln!("✗ High error rate detected, rolling back!");
                pool.set_weights(Color::Blue, 100).await?;
                pool.set_weights(Color::Green, 0).await?;
                self.green_credential = None;
                return Err(CredentialError::TestFailed(
                    format!("High error rate: {:.2}%", error_rate * 100.0)
                ));
            }
            
            eprintln!("  ✓ Traffic shift successful");
        }
        
        // Step 4: Deactivate blue
        eprintln!("\nStep 4: Deactivating blue credential...");
        self.active = Color::Green;
        self.blue_credential = self.green_credential.clone().unwrap();
        self.green_credential = None;
        eprintln!("✓ Blue-green swap complete");
        
        // Step 5: Grace period before revoking old blue
        eprintln!("\nStep 5: Grace period (5 minutes)...");
        tokio::time::sleep(Duration::from_secs(300)).await;
        
        eprintln!("✓ Rotation complete, old credential can be revoked");
        
        Ok(())
    }
}
```

---

### 3.2 Connection Pool Draining

**Issue**: Existing connections still use old credential

**Solution**: Graceful connection draining

```rust
use nebula_credential::prelude::*;

pub async fn rotate_with_connection_draining(
    pool: &mut ConnectionPool,
    old_cred: &DatabaseCredential,
    new_cred: &DatabaseCredential,
) -> Result<(), CredentialError> {
    eprintln!("=== Rotation with Connection Draining ===");
    
    // Step 1: Mark old pool for draining
    eprintln!("Step 1: Marking old pool for draining...");
    pool.start_draining().await;
    eprintln!("  Old pool: no new connections");
    eprintln!("  Existing connections: {} active", pool.active_connections());
    
    // Step 2: Create new pool with new credential
    eprintln!("\nStep 2: Creating new pool...");
    let new_pool = ConnectionPool::new(new_cred.clone()).await?;
    eprintln!("✓ New pool created");
    
    // Step 3: Wait for old connections to drain (with timeout)
    eprintln!("\nStep 3: Draining old connections...");
    let drain_timeout = Duration::from_secs(300);  // 5 minutes
    let start = std::time::Instant::now();
    
    loop {
        let active = pool.active_connections();
        eprintln!("  Active connections: {active}");
        
        if active == 0 {
            eprintln!("✓ All connections drained");
            break;
        }
        
        if start.elapsed() > drain_timeout {
            eprintln!("⚠️  Drain timeout, force-closing {} connections", active);
            pool.force_close_all().await;
            break;
        }
        
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    
    // Step 4: Close old pool
    eprintln!("\nStep 4: Closing old pool...");
    pool.close().await;
    eprintln!("✓ Old pool closed");
    
    // Step 5: Activate new pool
    eprintln!("\nStep 5: Activating new pool...");
    *pool = new_pool;
    eprintln!("✓ New pool active");
    
    eprintln!("\n=== Rotation Complete ===");
    
    Ok(())
}
```

---

## 4. Specific Credential Type Failures

### 4.1 OAuth2 Token Refresh Failure

**Issue**: Refresh token rotation failed

**Rollback**: Use cached access token until expiry

```rust
use nebula_credential::prelude::*;

pub async fn oauth2_refresh_with_fallback(
    credential: &OAuth2Credential,
    ctx: &CredentialContext,
) -> Result<OAuth2Credential, CredentialError> {
    match credential.refresh(&credential).await {
        Ok(refreshed) => {
            eprintln!("✓ Token refreshed successfully");
            Ok(refreshed)
        }
        Err(e) => {
            eprintln!("✗ Refresh failed: {e}");
            
            // Check if current access token still valid
            if let Some(expires_at) = credential.expires_at {
                let now = Utc::now();
                if now < expires_at {
                    let remaining = expires_at.signed_duration_since(now);
                    eprintln!("ℹ️  Current access token valid for {remaining}");
                    eprintln!("   Using current token as fallback");
                    return Ok(credential.clone());
                }
            }
            
            eprintln!("✗ Current token also expired, re-authentication required");
            Err(e)
        }
    }
}
```

---

### 4.2 Database Credential Rotation Failure

**Issue**: New database password rejected

**Rollback**: Restore old password

```rust
use nebula_credential::prelude::*;

pub async fn rotate_database_credential_safe(
    db_client: &DatabaseClient,
    username: &str,
    old_password: &SecretString,
    new_password: &SecretString,
) -> Result<(), CredentialError> {
    eprintln!("=== Database Credential Rotation ===");
    
    // Step 1: Set new password
    eprintln!("Step 1: Setting new password...");
    match db_client.change_password(username, new_password).await {
        Ok(_) => eprintln!("✓ New password set"),
        Err(e) => {
            eprintln!("✗ Password change failed: {e}");
            return Err(CredentialError::AuthenticationFailed(e.to_string()));
        }
    }
    
    // Step 2: Test new password
    eprintln!("\nStep 2: Testing new password...");
    match db_client.test_login(username, new_password).await {
        Ok(_) => eprintln!("✓ New password works"),
        Err(e) => {
            eprintln!("✗ New password test failed: {e}");
            eprintln!("   Rolling back to old password...");
            
            // Rollback
            match db_client.change_password(username, old_password).await {
                Ok(_) => {
                    eprintln!("✓ Old password restored");
                    return Err(CredentialError::TestFailed(
                        "New password invalid, rolled back".to_string()
                    ));
                }
                Err(rollback_err) => {
                    eprintln!("✗ CRITICAL: Rollback failed: {rollback_err}");
                    eprintln!("   Manual recovery required!");
                    eprintln!("   SQL: ALTER USER {username} WITH PASSWORD '<old_password>';");
                    return Err(CredentialError::AuthenticationFailed(
                        format!("Rotation and rollback both failed: {e}, {rollback_err}")
                    ));
                }
            }
        }
    }
    
    eprintln!("\n=== Rotation Complete ===");
    Ok(())
}
```

---

### 4.3 Certificate Rotation Failure

**Issue**: New certificate not trusted by clients

**Rollback**: Restore old certificate, extend validity

```rust
use nebula_credential::prelude::*;
use openssl::x509::X509;

pub async fn rotate_certificate_safe(
    cert_manager: &CertificateManager,
    cert_id: &str,
    old_cert: &X509,
    new_cert: &X509,
) -> Result<(), CredentialError> {
    eprintln!("=== Certificate Rotation ===");
    
    // Step 1: Deploy new certificate
    eprintln!("Step 1: Deploying new certificate...");
    match cert_manager.deploy_certificate(cert_id, new_cert).await {
        Ok(_) => eprintln!("✓ New certificate deployed"),
        Err(e) => {
            eprintln!("✗ Deployment failed: {e}");
            return Err(CredentialError::AuthenticationFailed(e.to_string()));
        }
    }
    
    // Step 2: Test TLS handshake
    eprintln!("\nStep 2: Testing TLS handshake...");
    match test_tls_handshake_with_cert(new_cert).await {
        Ok(_) => eprintln!("✓ TLS handshake successful"),
        Err(e) => {
            eprintln!("✗ TLS handshake failed: {e}");
            eprintln!("   Possible causes:");
            eprintln!("   - Certificate not in client trust store");
            eprintln!("   - Certificate chain incomplete");
            eprintln!("   - Wrong certificate purpose");
            eprintln!("\n   Rolling back to old certificate...");
            
            // Rollback
            match cert_manager.deploy_certificate(cert_id, old_cert).await {
                Ok(_) => {
                    eprintln!("✓ Old certificate restored");
                    
                    // Check if old cert near expiry
                    let expiry = old_cert.not_after();
                    let now = Utc::now();
                    let days_remaining = expiry.signed_duration_since(now).num_days();
                    
                    if days_remaining < 30 {
                        eprintln!("\n⚠️  WARNING: Old certificate expires in {days_remaining} days");
                        eprintln!("   Urgent action required:");
                        eprintln!("   1. Fix new certificate trust issues");
                        eprintln!("   2. Retry rotation before expiry");
                    }
                    
                    return Err(CredentialError::TestFailed(
                        "Certificate trust verification failed".to_string()
                    ));
                }
                Err(rollback_err) => {
                    eprintln!("✗ CRITICAL: Rollback failed: {rollback_err}");
                    return Err(CredentialError::AuthenticationFailed(
                        format!("Certificate rotation and rollback failed: {e}, {rollback_err}")
                    ));
                }
            }
        }
    }
    
    eprintln!("\n=== Rotation Complete ===");
    Ok(())
}
```

---

## 5. Recovery Procedures

### 5.1 Catastrophic Failure Recovery

**Scenario**: Both old and new credentials invalid

**Steps**:

1. **Assess damage**:
   ```rust
   eprintln!("=== Catastrophic Rotation Failure ===");
   eprintln!("Both old and new credentials invalid");
   eprintln!("Immediate actions required:");
   ```

2. **Check for backups**:
   ```rust
   let backups = manager.list_credential_backups(&id).await?;
   for backup in backups {
       eprintln!("Backup found:");
       eprintln!("  Timestamp: {}", backup.created_at);
       eprintln!("  Version: {}", backup.version);
   }
   ```

3. **Restore from backup**:
   ```rust
   if let Some(latest_backup) = backups.last() {
       eprintln!("Restoring from backup: {}", latest_backup.created_at);
       let restored = manager.restore_from_backup(&id, &latest_backup.id).await?;
       
       match restored.test(&ctx).await {
           Ok(test) if test.success => eprintln!("✓ Backup credential valid"),
           _ => eprintln!("✗ Backup also invalid"),
       }
   }
   ```

4. **Last resort - re-authenticate**:
   ```rust
   eprintln!("No valid backups found");
   eprintln!("Re-authentication required");
   let new_cred = authenticate_user(&ctx).await?;
   manager.store_credential(&id, &new_cred, &ctx).await?;
   ```

---

## Related Documentation

- [[Common-Errors]] - All error types
- [[../How-To/Rotate-Credentials]] - Rotation guide
- [[../Advanced/Rotation-Policies]] - Policy configuration
- [[../Examples/Database-Rotation]] - Database rotation example
- [[../Examples/API-Key-Rotation]] - API key rotation
- [[../Examples/Certificate-Rotation]] - Certificate rotation
- [[Debugging-Checklist]] - Systematic debugging

---

## Summary

This guide covers:

✅ **Rotation failure modes** and diagnosis  
✅ **Automatic rollback** transactions  
✅ **Manual rollback** procedures  
✅ **Zero-downtime rotation** patterns  
✅ **Credential-specific** rotation failures  
✅ **Catastrophic failure** recovery  
✅ **Grace period** configuration  
✅ **Blue-green deployment** strategies  

Always configure grace periods, validate new credentials before committing, and maintain rollback capability.
