# Credential Rotation Quickstart

Get started with credential rotation in 5-10 minutes.

## Prerequisites

- Completed Phase 3: Credential Manager implementation
- At least one storage provider configured (Local, AWS, Azure, Vault, or K8s)
- Rust 2024 Edition (MSRV 1.92)

## Basic Periodic Rotation (5 minutes)

Configure a database credential to rotate every 90 days:

```rust
use nebula_credential::rotation::*;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Initialize credential manager (from Phase 3)
    let manager = CredentialManager::new(storage_provider).await?;
    
    // 2. Store a database credential
    let credential_id = manager.store(DatabaseCredential {
        host: "db.example.com".to_string(),
        port: 5432,
        username: "app_user".to_string(),
        password: SecretString::new("current_password"),
        database: "production".to_string(),
    }).await?;
    
    // 3. Configure periodic rotation (every 90 days with 24-hour grace period)
    let policy = RotationPolicy::Periodic(PeriodicConfig {
        interval: Duration::from_secs(90 * 24 * 60 * 60), // 90 days
        grace_period: Duration::from_secs(24 * 60 * 60),  // 24 hours
        enable_jitter: true,                              // ±10% randomization
    });
    
    manager.set_rotation_policy(credential_id, policy).await?;
    
    // 4. Start background scheduler
    let scheduler = RotationScheduler::new(manager.clone());
    tokio::spawn(async move {
        scheduler.run().await.expect("Scheduler failed");
    });
    
    println!("✓ Rotation configured for credential {}", credential_id);
    println!("  Next rotation: ~90 days (±9 days jitter)");
    println!("  Grace period: 24 hours");
    
    // Application continues...
    Ok(())
}
```

**What happens**:
1. Every 90 days (±9 days with jitter), scheduler triggers rotation
2. New database user created with identical privileges
3. New and old credentials both valid for 24 hours (grace period)
4. After 24 hours, old credential automatically revoked

---

## Manual Emergency Rotation (1 minute)

Respond to security incident by immediately rotating compromised credential:

```rust
use nebula_credential::rotation::*;

// Discovered API key in public GitHub repository
let result = manager.rotate_now(
    credential_id,
    RotationReason::SecurityIncident {
        incident_id: "INC-2026-042".to_string(),
        description: "API key found in public repo".to_string(),
    },
    true, // no_grace_period - revoke old key immediately
).await?;

println!("✓ Emergency rotation complete");
println!("  Old credential: REVOKED immediately");
println!("  New credential: {} (active)", result.new_credential_id);
println!("  Audit log: {}", result.audit_event_id);
```

**When to use**:
- Credential compromised (leaked in logs, public repository, etc.)
- Employee departure requiring immediate access revocation
- Compliance audit finding requiring immediate remediation

---

## OAuth2 Token Refresh (Before-Expiry)

Automatically refresh OAuth2 tokens before expiration:

```rust
use nebula_credential::rotation::*;

// Configure before-expiry rotation for 1-hour OAuth2 tokens
let policy = RotationPolicy::BeforeExpiry(BeforeExpiryConfig {
    threshold_percentage: 0.80,                     // Rotate at 80% of TTL
    minimum_time_before_expiry: Duration::from_secs(5 * 60), // Safety buffer: 5 min
    grace_period: Duration::from_secs(10 * 60),     // Overlap: 10 min
});

manager.set_rotation_policy(oauth2_credential_id, policy).await?;

// System automatically refreshes when token reaches 48 minutes (80% of 1-hour TTL)
// Both old and new tokens valid for 10 minutes during rotation
```

**Prevents**:
- Token expiration causing authentication failures
- Service disruptions from expired credentials
- Manual token refresh operations

---

## Monitoring Rotation

Check rotation status and history:

```rust
// Get current rotation status
let status = manager.get_rotation_status(credential_id).await?;
match status.state {
    RotationState::Committed => {
        println!("Last rotation: {} ago", Utc::now() - status.completed_at);
        println!("Grace period ends: {}", status.grace_period_end);
    }
    RotationState::Pending => {
        println!("Rotation scheduled, not yet started");
    }
    _ => {
        println!("Rotation in progress: {:?}", status.state);
    }
}

// Get rotation history (audit trail)
let events = manager.get_rotation_history(credential_id).await?;
for event in events {
    println!("[{}] {:?} - {}", event.timestamp, event.event_type, event.metadata.get("message").unwrap_or(&String::new()));
}
```

**Output example**:
```
[2026-01-05 10:00:00 UTC] RotationStarted - Periodic policy triggered
[2026-01-05 10:00:15 UTC] NewCredentialCreated - Database user created
[2026-01-05 10:00:20 UTC] ValidationSucceeded - Connection test passed
[2026-01-05 10:00:25 UTC] RotationCommitted - Grace period active until 2026-01-06 10:00
[2026-01-06 10:00:00 UTC] GracePeriodExpired - Old credential revoked
```

---

## Scheduled Rotation (Maintenance Window)

Coordinate rotation with planned maintenance:

```rust
use chrono::{DateTime, Utc, NaiveDate, NaiveTime};

// Schedule rotation for first Saturday of March at 2 AM UTC
let scheduled_time = DateTime::<Utc>::from_utc(
    NaiveDate::from_ymd_opt(2026, 3, 7).unwrap()
        .and_time(NaiveTime::from_hms_opt(2, 0, 0).unwrap()),
    Utc,
);

let policy = RotationPolicy::Scheduled(ScheduledConfig {
    scheduled_at: scheduled_time,
    grace_period: Duration::from_secs(4 * 60 * 60), // 4 hours
    notify_before: Some(Duration::from_secs(24 * 60 * 60)), // 24-hour warning
});

manager.set_rotation_policy(credential_id, policy).await?;

// System sends notification on 2026-03-06 at 2 AM (24 hours before)
// Rotation executes on 2026-03-07 at 2 AM exactly
// Grace period: 4 hours for staggered application restarts
```

---

## Rollback on Failure

Automatic rollback if new credential fails validation:

```rust
// Configure validation test for database credentials
let validation_test = ValidationTest {
    test_method: TestMethod::DatabaseQuery {
        query: "SELECT 1".to_string(),
    },
    endpoint: "connection_string".to_string(),
    expected_criteria: SuccessCriteria::QueryResult,
    timeout: Duration::from_secs(10),
    retry_policy: RetryPolicy::default(),
};

// Rotation automatically tests new credential before committing
// If validation fails (connection error, auth failure, etc.):
//   1. New credential discarded
//   2. Old credential restored to active state
//   3. Rollback event logged for debugging
//   4. Administrators alerted via notifications

// Manual rollback also supported:
manager.rollback_rotation(credential_id).await?;
```

---

## Next Steps

- **Examples**: See `crates/nebula-credential/examples/` for complete runnable examples
  - `database_rotation.rs` - Blue-green database rotation
  - `oauth2_refresh.rs` - OAuth2 token refresh
  - `certificate_renewal.rs` - X.509 certificate rotation
  
- **Documentation**: Read `crates/nebula-credential/docs/How-To/Rotate-Credentials.md` for detailed guide

- **Troubleshooting**: See `docs/Troubleshooting/Rotation-Failures.md` for common issues

- **Testing**: Run `cargo test -p nebula-credential --test rotation_tests` to verify rotation works

---

**Estimated Time**: 5-10 minutes to configure first rotation policy
**Prerequisites**: Phase 3 (Credential Manager) must be complete
**Next**: Configure rotation policies for all production credentials
