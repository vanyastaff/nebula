# Rotation Implementation Research

**Phase**: 0 (Research & Technical Discovery)  
**Date**: 2026-02-04  
**Status**: Complete

## Technology Selections

### RT-001: Rotation Scheduling Library

**Decision**: Use `tokio::time::interval` with custom jitter logic (no external cron library)

**Rationale**:
- Tokio's native timers provide sufficient scheduling capability for rotation policies
- Periodic policy: `tokio::time::interval(duration)` with jitter added via `rand::thread_rng()`
- Before-Expiry policy: Calculate next check time from credential TTL, use `tokio::time::sleep_until()`
- Scheduled policy: Single `tokio::time::sleep_until(scheduled_at)`
- Simpler dependency footprint: Avoids cron parsing complexity (don't need cron syntax, just durations)

**Alternatives considered**:
- `tokio-cron-scheduler`: Adds cron parsing overhead we don't need (policies use Duration, not cron expressions)
- `apscheduler` equivalent: Too heavyweight for our use case

**Integration approach**:
```rust
// Periodic with jitter
let base_interval = policy.interval;
let jitter = base_interval.mul_f32(0.1); // ±10%
let actual_wait = base_interval + random_duration_in_range(-jitter, +jitter);
tokio::time::sleep(actual_wait).await;

// Before-Expiry
let ttl_remaining = credential.expires_at - Utc::now();
let threshold_duration = ttl_remaining.mul_f32(1.0 - policy.threshold_percentage);
tokio::time::sleep(threshold_duration).await;
```

---

### RT-002: Credential-Specific Validation Patterns

**Decision**: Trait-based validation with type-specific implementations

**Pattern Design**:
```rust
#[async_trait]
pub trait CredentialValidator: Send + Sync {
    async fn validate(&self, credential: &dyn Credential, context: &ValidationContext) -> Result<ValidationResult, ValidationError>;
    fn timeout(&self) -> Duration { Duration::from_secs(30) }
}

pub struct DatabaseValidator {
    test_query: String, // "SELECT 1"
}

#[async_trait]
impl CredentialValidator for DatabaseValidator {
    async fn validate(&self, credential: &dyn Credential, context: &ValidationContext) -> Result<ValidationResult, ValidationError> {
        let db_cred = credential.as_database()?;
        let client = create_client(db_cred).await?;
        let result = timeout(self.timeout(), client.query(&self.test_query, &[])).await??;
        Ok(ValidationResult::Success)
    }
}

// Similarly for OAuth2Validator (test /userinfo), ApiKeyValidator (test /account/status), etc.
```

**Test endpoint configuration**: Hardcoded defaults per credential type (can be overridden in CredentialMetadata)

**Mock validation**: Use trait objects with test doubles implementing `CredentialValidator`

---

### RT-003: Two-Phase Commit State Persistence

**Decision**: Store transaction state in same StorageProvider as credentials, using separate key namespace

**State storage approach**:
- Transaction state stored at key: `rotation:transactions:{rotation_id}`
- Credential backups stored at key: `rotation:backups:{backup_id}`
- Separate from credentials (stored at `credentials:{credential_id}`)

**Recovery on restart**:
```rust
async fn recover_incomplete_transactions(&self) -> Result<(), Error> {
    let incomplete = self.storage.list_prefix("rotation:transactions:").await?;
    
    for transaction_key in incomplete {
        let transaction: RotationTransaction = self.storage.get(&transaction_key).await?;
        
        match transaction.state {
            Pending | Creating => {
                // Not started or very early: safe to cancel
                self.rollback_transaction(transaction.id).await?;
            }
            Validating => {
                // Validation may have completed but state not updated: retry validation
                self.retry_validation(transaction.id).await?;
            }
            Committing => {
                // Storage write may have succeeded but state not updated: check credential exists
                if self.new_credential_exists(&transaction).await? {
                    self.complete_transaction(transaction.id).await?;
                } else {
                    self.rollback_transaction(transaction.id).await?;
                }
            }
            Committed | RolledBack => {
                // Terminal states: cleanup transaction record
                self.cleanup_transaction(transaction.id).await?;
            }
        }
    }
    
    Ok(())
}
```

**Idempotency**: Transaction IDs are UUIDs, operations check current state before executing

---

### RT-004: Grace Period Implementation

**Decision**: Credential versioning with `CredentialManager` supporting version parameter

**API changes to CredentialManager**:
```rust
impl CredentialManager {
    // Existing method (unchanged - returns latest version)
    pub async fn retrieve(&self, id: &CredentialId) -> Result<Credential, Error>;
    
    // NEW method - explicit version retrieval
    pub async fn retrieve_version(&self, id: &CredentialId, version: u32) -> Result<Credential, Error>;
    
    // NEW method - list all versions during grace period
    pub async fn list_versions(&self, id: &CredentialId) -> Result<Vec<(u32, Credential, VersionStatus)>, Error>;
}

pub enum VersionStatus {
    Active,          // Current version
    GracePeriod(DateTime<Utc>), // Old version, expires at DateTime
    Revoked,         // Expired or manually revoked
}
```

**Storage schema**: Multiple entries with version suffix
- Old credential: `credentials:{credential_id}:v1` (marked for deletion at grace_period_end)
- New credential: `credentials:{credential_id}:v2` (active)
- Current pointer: `credentials:{credential_id}` → points to v2

**Backward compatibility**: Existing `retrieve()` calls get latest version transparently

---

### RT-005: Blue-Green Database Rotation

**Decision**: Database-specific privilege introspection using SQL queries, DDL generation per database type

**Supported databases**:

**PostgreSQL**:
```sql
-- Introspect privileges
SELECT grantee, privilege_type, table_schema, table_name
FROM information_schema.role_table_grants
WHERE grantee = 'old_user';

-- Replicate for new user
GRANT {privilege_type} ON {schema}.{table} TO new_user;
```

**MySQL**:
```sql
-- Introspect privileges
SHOW GRANTS FOR 'old_user'@'%';

-- Parse and replicate
GRANT SELECT, INSERT, UPDATE ON database.* TO 'new_user'@'%';
```

**Abstraction design**:
```rust
#[async_trait]
pub trait DatabasePrivilegeManager {
    async fn list_privileges(&self, username: &str) -> Result<Vec<Privilege>, Error>;
    async fn grant_privilege(&self, privilege: &Privilege, username: &str) -> Result<(), Error>;
}

pub struct PostgresPrivilegeManager { /* impl */ }
pub struct MySqlPrivilegeManager { /* impl */ }
```

**Atomic transfer**: Wrapped in database transaction where supported (PostgreSQL), best-effort for MySQL

---

### RT-006: Certificate Renewal CA Integration

**Decision**: Trait abstraction `CertificateAuthority` with provider-specific implementations

**CA abstraction**:
```rust
#[async_trait]
pub trait CertificateAuthority: Send + Sync {
    async fn issue_certificate(&self, request: CertificateRequest) -> Result<X509Certificate, CaError>;
    async fn revoke_certificate(&self, serial: &str) -> Result<(), CaError>;
    fn supports_client_auth(&self) -> bool; // June 2026 policy: private CAs only
}

pub struct AwsPrivateCA {
    client: aws_sdk_acmpca::Client,
    ca_arn: String,
}

pub struct VaultPKI {
    client: vaultrs::client::VaultClient,
    pki_mount: String,
    role_name: String,
}

pub struct SelfSignedCA {
    ca_cert: rcgen::Certificate,
    ca_key: rcgen::KeyPair,
}
```

**Provider implementations**:
- AWS: Use `aws-sdk-acmpca` crate
- Vault: Use `vaultrs` crate
- Self-signed: Use `rcgen` crate for certificate generation

**Rate limiting**: Implement retry with exponential backoff (5 attempts, 2x multiplier, max 32s backoff)

---

### RT-007: Rotation Metrics and Observability

**Decision**: Use `metrics` crate (vendor-agnostic facade) with Prometheus exporter

**Chosen library**: `metrics` crate + `metrics-exporter-prometheus`

**Rationale**:
- Vendor-agnostic: Can switch from Prometheus to StatsD/Datadog without code changes
- Rust-idiomatic: Macros like `counter!()`, `histogram!()`, `gauge!()`
- Low overhead: No-op in release builds if metrics disabled

**Metrics specification**:
```rust
// Duration histogram
histogram!("rotation.duration_seconds", rotation_duration.as_secs_f64(), 
    "policy" => policy_type, "credential_type" => cred_type);

// Success/failure counters
counter!("rotation.total", 1, "status" => "success", "policy" => policy_type);
counter!("rotation.total", 1, "status" => "failure", "reason" => error_type);

// Active grace periods gauge
gauge!("rotation.active_grace_periods", active_count as f64);

// Credentials near expiry
gauge!("rotation.credentials_expiring_soon", expiring_count as f64, 
    "threshold_days" => days.to_string());

// Validation failures
counter!("rotation.validation_failures", 1, "credential_type" => cred_type);

// Rollback events
counter!("rotation.rollbacks", 1, "reason" => rollback_reason);
```

**Integration**: Initialize Prometheus exporter on HTTP endpoint `/metrics` for scraping

---

### RT-008: Distributed Locking

**Decision**: Storage-provider-based optimistic locking with version numbers (compare-and-swap)

**Rationale**:
- Works with ALL storage providers (no external dependency like Redis)
- Leverages existing storage operations
- Fault-tolerant: No lock holder crash issues (locks are stateless version checks)

**Implementation**:
```rust
pub struct RotationLock {
    credential_id: CredentialId,
    lock_version: u64,
}

impl RotationScheduler {
    async fn acquire_rotation_lock(&self, credential_id: &CredentialId) -> Result<Option<RotationLock>, Error> {
        let lock_key = format!("rotation:locks:{}", credential_id);
        
        // Try to create lock with version 1
        match self.storage.create_if_not_exists(&lock_key, &1u64).await {
            Ok(_) => Ok(Some(RotationLock { credential_id: credential_id.clone(), lock_version: 1 })),
            Err(AlreadyExists) => {
                // Lock held by another instance
                Ok(None)
            }
        }
    }
    
    async fn release_rotation_lock(&self, lock: RotationLock) -> Result<(), Error> {
        let lock_key = format!("rotation:locks:{}", lock.credential_id);
        self.storage.delete(&lock_key).await?;
        Ok(())
    }
}
```

**Lock timeout**: Lock records include timestamp, cleanup task removes locks older than 5 minutes (handles crashes)

**Fallback strategy**: If lock acquisition fails, skip rotation and retry at next scheduler interval (fail-safe, not fail-fast)

---

## Summary of Key Decisions

| Research Area | Decision | Key Benefit |
|---------------|----------|-------------|
| Scheduling | Tokio native timers + custom jitter | Simpler, no cron parsing overhead |
| Validation | Trait-based with type-specific impls | Extensible for new credential types |
| State Persistence | Same StorageProvider, separate namespace | No new dependencies, crash recovery |
| Grace Period | Credential versioning with version parameter | Backward compatible API |
| Blue-Green DB | Database-specific SQL introspection | Accurate privilege replication |
| CA Integration | Trait abstraction with 3 initial providers | Extensible for new CAs |
| Metrics | `metrics` crate (vendor-agnostic) | Prometheus + future flexibility |
| Distributed Lock | Optimistic locking with version numbers | Works with all storage providers |

---

## Open Questions (for implementation phase)

1. **Grace period cleanup**: Should expired grace period credentials be hard-deleted or soft-deleted (marked revoked)?
   - **Recommendation**: Soft-delete initially, hard-delete after 7 days (forensics window)

2. **Rotation failure notification**: Email, webhook, or both?
   - **Recommendation**: Both, configurable per rotation policy

3. **Backup encryption key**: Same as credentials or separate?
   - **Recommendation**: Same key (simplifies key management), backups inherit credential encryption

4. **Scheduler crash detection**: How quickly should system detect scheduler is down?
   - **Recommendation**: Heartbeat every 60 seconds, alert if no heartbeat for 5 minutes

---

**Status**: Research complete, ready for Phase 1 (Data Model & Contracts)
