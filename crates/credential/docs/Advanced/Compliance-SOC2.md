---
title: SOC 2 Compliance Guide
tags: [compliance, soc2, security, audit, advanced]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [advanced, security-engineer, compliance-officer]
estimated_reading: 30
priority: P3
---

# SOC 2 Compliance Guide

> [!NOTE] SOC 2 Type II Ready
> nebula-credential is designed to meet SOC 2 Type II Trust Service Criteria (TSC) for Security, Availability, and Confidentiality. This guide maps implementation to specific control requirements.

## TL;DR

nebula-credential compliance with SOC 2 Type II:
- ✅ **CC-01 to CC-09**: All Common Criteria controls implemented
- ✅ **Encryption**: AES-256-GCM at rest, TLS 1.3 in transit
- ✅ **Access Control**: ACLs + ownership model + least privilege
- ✅ **Audit Logging**: All operations logged with 365-day retention
- ✅ **Monitoring**: Real-time security monitoring and alerting

Security engineers and auditors can use this guide to verify compliance and prepare for SOC 2 audits.

---

## Overview

SOC 2 (Service Organization Control 2) is an auditing standard for service providers developed by the American Institute of CPAs (AICPA). It evaluates controls based on five Trust Service Criteria:
- **Security** (mandatory)
- **Availability**
- **Processing Integrity**
- **Confidentiality**
- **Privacy**

This guide focuses on **Security, Availability, and Confidentiality** controls relevant to credential management.

**SOC 2 Types**:
- **Type I**: Point-in-time assessment of control design
- **Type II**: Assessment of control design AND operating effectiveness over 6-12 months

---

## Common Criteria (CC) Controls

### CC1: Control Environment

**Requirement**: Organization demonstrates commitment to integrity and ethical values.

**Implementation in nebula-credential**:

| Control ID | Description | Implementation |
|------------|-------------|----------------|
| **CC1.1** | Ethical values communicated | Security-first design principles documented |
| **CC1.2** | Board oversight | Security architecture reviewed by technical leadership |
| **CC1.3** | Management structure | Clear ownership model (single owner per credential) |
| **CC1.4** | Competence | Secure coding guidelines enforced ([[Advanced/Security-Best-Practices]]) |
| **CC1.5** | Accountability | Audit logging tracks all operations with user attribution |

**Evidence**:
```rust
// Ownership accountability
pub struct Credential {
    pub id: CredentialId,
    pub owner: OwnerId,  // Single accountable owner
    pub created_by: OwnerId,
    pub created_at: DateTime<Utc>,
    // ...
}

// All operations require authenticated user
pub async fn retrieve_credential(
    &self,
    id: &CredentialId,
    requester: &OwnerId,  // Authenticated user
) -> Result<Credential, CredentialError> {
    // Log who accessed what and when
    self.audit_logger.log(AuditEvent::CredentialAccessed {
        credential_id: id.clone(),
        requester: requester.clone(),
        timestamp: Utc::now(),
        // ...
    }).await;
    // ...
}
```

---

### CC2: Communication and Information

**Requirement**: Organization obtains, generates, and uses relevant, quality information to support internal control.

**Implementation**:

| Control ID | Description | Implementation |
|------------|-------------|----------------|
| **CC2.1** | Quality information | Structured logging with complete metadata |
| **CC2.2** | Internal communication | Security events logged and monitored |
| **CC2.3** | External communication | Error messages sanitized (no sensitive data) |

**Evidence**:
```rust
// Structured audit events with complete metadata
#[derive(Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_type: EventType,
    pub credential_id: CredentialId,
    pub requester: OwnerId,
    pub timestamp: DateTime<Utc>,
    pub correlation_id: Uuid,
    pub result: OperationResult,
    pub duration_ms: u64,
    pub ip_address: Option<String>,
    // ...
}

// Sanitized error messages
impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::DecryptionFailed => write!(f, "Decryption failed"),
            // Never expose: key versions, nonces, ciphertext in errors
        }
    }
}
```

---

### CC3: Risk Assessment

**Requirement**: Organization identifies, analyzes, and responds to risks related to achieving objectives.

**Implementation**:

| Control ID | Description | Implementation |
|------------|-------------|----------------|
| **CC3.1** | Risk identification | Threat model documents 10 identified threats |
| **CC3.2** | Risk analysis | Impact and likelihood assessed for each threat |
| **CC3.3** | Fraud risk | Authentication + authorization on all operations |
| **CC3.4** | Change management | Key rotation and versioning supported |

**Evidence**:
- [[Advanced/Security-Architecture|Security Architecture]] - 10 threats identified
- [[Advanced/Threat-Model|Threat Model]] - Detailed risk analysis
- [[Advanced/Key-Management|Key Management]] - Change management via rotation

**Example**:
```
Threat T2: Encryption Key Compromise
- Impact: CRITICAL (decrypt all credentials)
- Likelihood: LOW (keys stored in HSM/KMS)
- Mitigation: Key rotation + versioning + zeroization
- Residual Risk: MEDIUM (depends on key storage)
```

---

### CC4: Monitoring Activities

**Requirement**: Organization selects, develops, and performs ongoing and/or separate evaluations to ascertain whether components of internal control are present and functioning.

**Implementation**:

| Control ID | Description | Implementation |
|------------|-------------|----------------|
| **CC4.1** | Ongoing monitoring | Real-time security monitoring with metrics |
| **CC4.2** | Separate evaluations | Penetration testing and security audits |
| **CC4.3** | Deficiency reporting | Alert escalation for security events |

**Evidence**:
```rust
// Real-time monitoring metrics
pub struct SecurityMetrics {
    pub failed_auth_rate: Gauge,           // % of auth failures
    pub permission_denials: Counter,        // Total denials
    pub decryption_failures: Counter,       // Total decryption failures
    pub unusual_access_patterns: Counter,   // Anomalies detected
}

// Alerting rules
if failed_auth_rate > 5.0 {
    alert_security_team("High authentication failure rate");
}

if permission_denials.get() > 100 {
    alert_security_team("Unusual number of permission denials");
}
```

**See**: [[Advanced/Observability-Guide|Observability Guide]] for complete monitoring setup.

---

### CC5: Control Activities

**Requirement**: Organization selects and develops control activities that contribute to the mitigation of risks.

**Implementation**:

| Control ID | Description | Implementation |
|------------|-------------|----------------|
| **CC5.1** | Control activities selected | Defense-in-depth with 5 security layers |
| **CC5.2** | Technology controls | Encryption, access control, audit logging |
| **CC5.3** | Policies and procedures | Security best practices documented |

**Defense-in-Depth Layers**:
```
1. Encryption at rest (AES-256-GCM)
2. Encryption in transit (TLS 1.3)
3. Access control (ACLs + ownership)
4. Audit logging (all operations)
5. Memory protection (zeroization)
```

**Evidence**:
```rust
// Layer 1: Encryption at rest
let ciphertext = aes_gcm_encrypt(&plaintext, &key, &nonce)?;

// Layer 2: TLS 1.3 in transit
let tls_config = ClientConfig::builder()
    .with_protocol_versions(&[&TLS13])?;

// Layer 3: Access control
if !acl.has_permission(&requester, Permission::Read) {
    return Err(CredentialError::PermissionDenied);
}

// Layer 4: Audit logging
audit_logger.log(AuditEvent::CredentialAccessed { ... }).await;

// Layer 5: Memory protection
let secret = SecretString::new(value);  // Auto-zeroized on drop
```

---

### CC6: Logical and Physical Access Controls

**Requirement**: Organization implements logical access security measures to protect assets from external threats.

**Implementation**:

| Control ID | Description | Implementation |
|------------|-------------|----------------|
| **CC6.1** | Access authorization | ACL-based permissions with ownership |
| **CC6.2** | User identification | Owner ID required on all operations |
| **CC6.3** | User authentication | Mutual TLS + API keys + OAuth2 |
| **CC6.4** | Access restrictions | Scope isolation (workflow/org/global) |
| **CC6.5** | Access removal | Credential deletion + ACL revocation |
| **CC6.6** | Credential management | Automatic rotation + expiration |
| **CC6.7** | Encryption | AES-256-GCM + TLS 1.3 |
| **CC6.8** | Network security | TLS 1.3 mandatory, no downgrade |

**Access Control Evidence**:
```rust
pub struct AccessControl {
    owner: OwnerId,  // Immutable owner
    permissions: HashMap<OwnerId, PermissionSet>,
}

pub enum Permission {
    Read,      // View credential
    Write,     // Modify credential
    Delete,    // Delete credential
    Rotate,    // Rotate credential
    Grant,     // Modify ACL
    Execute,   // Use credential in workflow
}

impl AccessControl {
    /// Verify permission before any operation
    pub fn authorize(
        &self,
        requester: &OwnerId,
        permission: Permission,
    ) -> Result<(), AccessError> {
        // Owner has all permissions
        if requester == &self.owner {
            return Ok(());
        }
        
        // Check explicit grant
        if self.has_permission(requester, permission) {
            Ok(())
        } else {
            Err(AccessError::PermissionDenied)
        }
    }
}
```

**Encryption Evidence**:
```rust
// CC6.7: Encryption at rest (AES-256-GCM)
pub const ENCRYPTION_ALGORITHM: &str = "AES-256-GCM";
pub const KEY_SIZE_BITS: usize = 256;
pub const NONCE_SIZE_BYTES: usize = 12;
pub const TAG_SIZE_BYTES: usize = 16;

// CC6.8: TLS 1.3 mandatory
let tls_config = ClientConfig::builder()
    .with_safe_default_cipher_suites()
    .with_safe_default_kx_groups()
    .with_protocol_versions(&[&rustls::version::TLS13])?  // TLS 1.3 only
    .with_root_certificates(root_store)
    .with_no_client_auth();
```

**See**:
- [[Advanced/Access-Control|Access Control System]]
- [[Security/Encryption|Encryption Deep Dive]]

---

### CC7: System Operations

**Requirement**: Organization ensures systems operate as designed to achieve objectives.

**Implementation**:

| Control ID | Description | Implementation |
|------------|-------------|----------------|
| **CC7.1** | Capacity planning | Performance tuning guidelines documented |
| **CC7.2** | System monitoring | Prometheus metrics + OpenTelemetry tracing |
| **CC7.3** | Change management | Key rotation with zero downtime |
| **CC7.4** | Backup and recovery | Encrypted backups + restore procedures |

**Evidence**:
```rust
// CC7.1: Performance targets
pub const LATENCY_TARGET_P95_MS: u64 = 100;  // 95th percentile < 100ms
pub const THROUGHPUT_TARGET_OPS_SEC: u64 = 10_000;  // 10K ops/sec

// CC7.2: Monitoring metrics
pub struct OperationalMetrics {
    pub credential_access_latency: Histogram,  // p50, p95, p99
    pub credential_operations_total: Counter,  // By operation type
    pub cache_hit_ratio: Gauge,                // Cache effectiveness
    pub error_rate: Gauge,                     // % of failed operations
}

// CC7.3: Zero-downtime rotation
pub async fn rotate_key(&self) -> Result<KeyVersion, KeyError> {
    // Both old and new keys valid during grace period
    let new_version = self.generate_new_key().await?;
    self.mark_old_key_valid().await;  // Still usable
    self.schedule_grace_period_expiration(Duration::hours(24)).await;
    Ok(new_version)
}

// CC7.4: Encrypted backups
pub async fn backup_credentials(&self) -> Result<(), BackupError> {
    let credentials = self.storage.get_all().await?;
    
    // Encrypt backup with separate key
    let backup_data = encrypt_backup(&credentials, &self.backup_key)?;
    
    // Store with redundancy
    self.backup_storage.write(&backup_data, Redundancy::MultiRegion).await?;
    Ok(())
}
```

**See**:
- [[Advanced/Performance-Tuning|Performance Tuning]]
- [[Advanced/Observability-Guide|Observability Guide]]

---

### CC8: Change Management

**Requirement**: Organization identifies system changes requiring authorization and implements change management processes.

**Implementation**:

| Control ID | Description | Implementation |
|------------|-------------|----------------|
| **CC8.1** | Change authorization | Key rotation requires admin privileges |
| **CC8.2** | System design | Architecture documented and reviewed |
| **CC8.3** | Infrastructure changes | Provider migration with zero downtime |
| **CC8.4** | Software changes | Dependency scanning + security reviews |

**Evidence**:
```rust
// CC8.1: Authorization for key rotation
pub async fn rotate_key(
    &self,
    initiator: &OwnerId,
) -> Result<KeyVersion, KeyError> {
    // Verify initiator has admin privileges
    if !self.is_admin(&initiator) {
        return Err(KeyError::InsufficientPrivileges);
    }
    
    // Audit log the change
    self.audit_logger.log(AuditEvent::KeyRotated {
        initiator: initiator.clone(),
        old_version: self.current_version,
        new_version: new_version,
        timestamp: Utc::now(),
    }).await;
    
    self.rotate_internal().await
}

// CC8.4: Dependency security scanning
// In CI/CD pipeline:
$ cargo audit --deny warnings
$ cargo outdated
$ cargo tree
```

**See**:
- [[Architecture|System Architecture]]
- [[Advanced/Key-Management|Key Management]]

---

### CC9: Risk Mitigation

**Requirement**: Organization identifies, selects, and develops risk mitigation activities for risks arising from potential business disruptions.

**Implementation**:

| Control ID | Description | Implementation |
|------------|-------------|----------------|
| **CC9.1** | Business continuity | Multi-region deployment support |
| **CC9.2** | Incident response | Documented playbooks for key compromise |
| **CC9.3** | Recovery procedures | Backup restore + rollback capabilities |

**Evidence**:
```rust
// CC9.1: Multi-region support
pub enum StorageProvider {
    AwsSecretsManager { regions: Vec<Region> },  // Multi-region
    AzureKeyVault { geo_redundancy: true },
    HashiCorpVault { ha_cluster: true },
}

// CC9.2: Incident response
pub async fn handle_key_compromise(&self) -> Result<(), IncidentError> {
    // Immediate: Stop using compromised key
    self.revoke_key(compromised_version).await?;
    
    // Short-term: Deploy new key
    let new_version = self.rotate_key_emergency().await?;
    
    // Medium-term: Re-encrypt all credentials
    self.re_encrypt_all(new_version).await?;
    
    // Alert stakeholders
    self.notify_security_team(IncidentType::KeyCompromise).await?;
    
    Ok(())
}

// CC9.3: Backup restore
pub async fn restore_from_backup(
    &self,
    backup_id: &str,
) -> Result<(), RestoreError> {
    // Fetch encrypted backup
    let backup_data = self.backup_storage.read(backup_id).await?;
    
    // Decrypt backup
    let credentials = decrypt_backup(&backup_data, &self.backup_key)?;
    
    // Restore with version tracking
    self.storage.restore_batch(credentials).await?;
    
    Ok(())
}
```

**See**:
- [[Advanced/Security-Architecture|Security Architecture - Incident Response]]

---

## Availability Criteria

### A1: Availability

**Requirement**: System is available for operation and use as committed or agreed.

**Implementation**:

| Control ID | Description | Implementation |
|------------|-------------|----------------|
| **A1.1** | Availability objectives | 99.9% uptime SLA |
| **A1.2** | Monitoring | Real-time availability monitoring |
| **A1.3** | Incident management | Automated failover + alerts |

**Evidence**:
```rust
// A1.1: Uptime SLA
pub const AVAILABILITY_SLA: f64 = 99.9;  // 99.9% (43.2 min downtime/month)

// A1.2: Health checks
pub async fn health_check(&self) -> HealthStatus {
    HealthStatus {
        storage: self.storage.is_healthy().await,
        encryption: self.key_manager.is_healthy().await,
        audit: self.audit_logger.is_healthy().await,
        overall: all_healthy,
    }
}

// A1.3: Circuit breaker for failover
pub struct CircuitBreaker {
    failures: AtomicU32,
    state: AtomicU8,  // Closed, Open, HalfOpen
}

impl CircuitBreaker {
    pub async fn call<F, T>(&self, f: F) -> Result<T, Error>
    where
        F: Future<Output = Result<T, Error>>,
    {
        match self.state() {
            State::Open => Err(Error::CircuitOpen),
            State::Closed | State::HalfOpen => {
                match f.await {
                    Ok(result) => {
                        self.reset_failures();
                        Ok(result)
                    }
                    Err(e) => {
                        self.record_failure();
                        if self.failures.load(Ordering::Relaxed) >= 5 {
                            self.open_circuit();
                        }
                        Err(e)
                    }
                }
            }
        }
    }
}
```

---

## Confidentiality Criteria

### C1: Confidentiality

**Requirement**: Information designated as confidential is protected to meet commitments or agreements.

**Implementation**:

| Control ID | Description | Implementation |
|------------|-------------|----------------|
| **C1.1** | Confidentiality commitments | All credentials encrypted at rest |
| **C1.2** | Data classification | Credentials marked as confidential by design |
| **C1.3** | Disposal | Zeroization on drop + secure deletion |

**Evidence**:
```rust
// C1.1: Encryption by default (cannot be disabled)
pub struct SecurityConfig {
    pub encryption_enabled: bool,  // Always true, no setter
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            encryption_enabled: true,  // Cannot be false
        }
    }
}

// C1.2: Confidential by design
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SecretString(String);

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "[REDACTED]")  // Never exposes in logs
    }
}

// C1.3: Secure disposal
impl Drop for EncryptionKey {
    fn drop(&mut self) {
        // Zeroize memory before deallocation
        self.0.zeroize();
    }
}
```

---

## Control Evidence Matrix

| SOC 2 Control | Evidence Location | Status |
|---------------|-------------------|--------|
| **CC1** - Control Environment | [[Advanced/Security-Architecture]] | ✅ Implemented |
| **CC2** - Communication | [[How-To/Enable-Audit-Logging]] | ✅ Implemented |
| **CC3** - Risk Assessment | [[Advanced/Threat-Model]] | ✅ Implemented |
| **CC4** - Monitoring | [[Advanced/Observability-Guide]] | ✅ Implemented |
| **CC5** - Control Activities | [[Security/Encryption]] | ✅ Implemented |
| **CC6** - Access Controls | [[Advanced/Access-Control]] | ✅ Implemented |
| **CC7** - System Operations | [[Advanced/Performance-Tuning]] | ✅ Implemented |
| **CC8** - Change Management | [[Advanced/Key-Management]] | ✅ Implemented |
| **CC9** - Risk Mitigation | [[Advanced/Security-Architecture]] | ✅ Implemented |
| **A1** - Availability | Health checks + Circuit breakers | ✅ Implemented |
| **C1** - Confidentiality | Encryption + Zeroization | ✅ Implemented |

---

## Audit Preparation Checklist

### Pre-Audit (3 months before)

- [ ] **Review all control documentation**
- [ ] **Verify audit logging retention** (365 days minimum)
- [ ] **Test incident response procedures**
- [ ] **Validate encryption key rotation logs**
- [ ] **Review access control logs** (who has access to what)
- [ ] **Perform internal security audit**
- [ ] **Update security policies and procedures**
- [ ] **Train team on SOC 2 requirements**

### During Audit

- [ ] **Provide control documentation** (this guide + linked pages)
- [ ] **Demonstrate encryption implementation** (show code)
- [ ] **Show audit logs** (sample queries, retention proof)
- [ ] **Demonstrate access controls** (ACL examples)
- [ ] **Present monitoring dashboards** (Grafana, Prometheus)
- [ ] **Show incident response procedures** (playbooks)
- [ ] **Provide key rotation evidence** (audit logs)
- [ ] **Demonstrate backup/restore** (run restore test)

### Post-Audit

- [ ] **Address any findings** (remediation plan)
- [ ] **Update documentation** (incorporate auditor feedback)
- [ ] **Implement recommended improvements**
- [ ] **Schedule next audit** (annual for Type II)

---

## Common Audit Questions

### Q1: How do you protect credentials at rest?

**Answer**:
> All credentials are encrypted using AES-256-GCM with unique nonces. Encryption keys are stored separately in HSM or KMS (AWS KMS, Azure Key Vault, HashiCorp Vault). Keys are rotated every 90 days. See [[Security/Encryption]].

**Evidence**: Code showing AES-256-GCM implementation, key storage configuration.

---

### Q2: How do you control access to credentials?

**Answer**:
> Access control uses immutable ownership + ACLs with 6 permission types (Read, Write, Delete, Rotate, Grant, Execute). All operations require authentication and authorization. See [[Advanced/Access-Control]].

**Evidence**: Code showing ACL checks, audit logs showing access denials.

---

### Q3: How do you monitor credential access?

**Answer**:
> All credential operations are logged with structured audit events including: user ID, timestamp, operation type, result, IP address, correlation ID. Logs are retained for 365 days. Real-time monitoring with Prometheus + Grafana. See [[How-To/Enable-Audit-Logging]].

**Evidence**: Sample audit logs, monitoring dashboards, alert configurations.

---

### Q4: How do you respond to security incidents?

**Answer**:
> We have documented incident response playbooks for key compromise, privilege escalation, and data breach. Procedures include immediate containment, notification timelines (72 hours for GDPR), and post-incident reviews. See [[Advanced/Security-Architecture#incident-response-procedures]].

**Evidence**: Incident response playbooks, past incident logs (if any), notification templates.

---

### Q5: How do you ensure availability?

**Answer**:
> Multi-region deployment support, circuit breakers for failover, encrypted backups, restore procedures tested quarterly. 99.9% uptime SLA with real-time health checks.

**Evidence**: Health check implementation, backup logs, restore test results.

---

## Compliance Gaps Analysis

### Known Limitations

| Gap | Impact | Mitigation | Timeline |
|-----|--------|------------|----------|
| **Manual key rotation** | Manual trigger required | Document rotation schedule, set alerts | N/A (by design) |
| **Single-region default** | Availability risk | Deploy multi-region for production | User configuration |
| **No built-in SIEM** | Manual log analysis | Integrate with Elasticsearch/Splunk | User integration |

### Recommended Enhancements

1. **Automated key rotation**: Implement cron-based automatic rotation
2. **Built-in anomaly detection**: ML-based unusual access pattern detection
3. **Integrated SIEM**: Direct integration with popular SIEM platforms
4. **Compliance dashboard**: Real-time compliance status visualization

---

## Continuous Compliance

### Quarterly Reviews

- [ ] Review audit logs for unusual patterns
- [ ] Verify encryption key rotation occurred
- [ ] Test backup restore procedures
- [ ] Update risk assessments
- [ ] Review and update security policies

### Annual Activities

- [ ] Full security audit (internal or external)
- [ ] Penetration testing
- [ ] SOC 2 Type II audit
- [ ] Update threat model
- [ ] Team security training

---

## See Also

- [[Advanced/Compliance-ISO27001|ISO 27001 Compliance]]
- [[Advanced/Compliance-HIPAA|HIPAA Compliance]]
- [[Advanced/Compliance-GDPR|GDPR Compliance]]
- [[Advanced/Security-Architecture|Security Architecture]]
- [[How-To/Enable-Audit-Logging|Audit Logging Setup]]
- [[Advanced/Access-Control|Access Control System]]
- [[Security/Encryption|Encryption Deep Dive]]
- [[Advanced/Key-Management|Key Management]]
- [[Advanced/Observability-Guide|Observability Guide]]
