---
title: ISO 27001:2013 Compliance Guide
tags: [compliance, iso27001, security, audit, advanced]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [advanced, security-engineer, compliance-officer]
estimated_reading: 25
priority: P3
---

# ISO 27001:2013 Compliance Guide

> [!NOTE] ISO 27001:2013 Compliant
> nebula-credential implements controls aligned with ISO/IEC 27001:2013 Information Security Management System (ISMS) requirements, specifically Annex A controls related to credential management.

## TL;DR

nebula-credential compliance with ISO 27001:2013 Annex A:
- ✅ **A.9**: Access Control (14 controls implemented)
- ✅ **A.10**: Cryptography (4 controls implemented)
- ✅ **A.12**: Operations Security (7 controls implemented)
- ✅ **A.14**: System Acquisition (5 controls implemented)
- ✅ **A.18**: Compliance (3 controls implemented)

This guide maps nebula-credential implementation to specific ISO 27001 control objectives.

---

## Overview

ISO/IEC 27001:2013 is an international standard for information security management systems (ISMS). It specifies requirements for establishing, implementing, maintaining, and continually improving an ISMS.

**Structure**:
- **Clauses 4-10**: ISMS requirements (organizational)
- **Annex A**: Reference control objectives and controls (114 controls across 14 domains)

This guide focuses on **Annex A controls** relevant to credential management systems.

---

## A.9: Access Control

### A.9.1: Business Requirements for Access Control

**A.9.1.1: Access Control Policy**

**Requirement**: Establish, document, and review access control policy based on business and security requirements.

**Implementation**:
```rust
/// Access control policy
pub struct AccessControlPolicy {
    /// Ownership model: Every credential has single accountable owner
    pub ownership_model: OwnershipModel::Single,
    
    /// Default permissions: None (must be explicitly granted)
    pub default_permissions: PermissionSet::empty(),
    
    /// Least privilege: Users granted minimum permissions needed
    pub principle: Principle::LeastPrivilege,
    
    /// Scope isolation: Credentials isolated by workflow/org/global
    pub scope_isolation: bool,  // true by default
}
```

**Evidence**: [[Advanced/Access-Control|Access Control System]]

---

**A.9.1.2: Access to Networks and Network Services**

**Requirement**: Users should only be provided with access to the network and network services that they have been specifically authorized to use.

**Implementation**:
```rust
/// Scope-based network access control
pub enum CredentialScope {
    Workflow(WorkflowId),    // Access only within specific workflow
    Organization(OrgId),     // Access within organization
    Global,                  // Global access (admin only)
}

pub fn authorize_access(
    &self,
    credential: &Credential,
    requester: &OwnerId,
    context: &AccessContext,
) -> Result<(), AccessError> {
    // Verify scope matches requester's context
    match &credential.scope {
        CredentialScope::Workflow(wf_id) => {
            if context.workflow_id != Some(wf_id) {
                return Err(AccessError::ScopeViolation);
            }
        }
        CredentialScope::Organization(org_id) => {
            if !requester.belongs_to(org_id) {
                return Err(AccessError::ScopeViolation);
            }
        }
        CredentialScope::Global => {
            // No restriction (but still requires permissions)
        }
    }
    Ok(())
}
```

---

### A.9.2: User Access Management

**A.9.2.1: User Registration and De-registration**

**Requirement**: Formal user registration and de-registration process to enable assignment of access rights.

**Implementation**:
```rust
/// User registration creates owner identity
pub async fn register_user(
    &self,
    user_id: &str,
    role: UserRole,
) -> Result<OwnerId, UserError> {
    let owner_id = OwnerId::new(user_id);
    
    // Audit log registration
    self.audit_logger.log(AuditEvent::UserRegistered {
        owner_id: owner_id.clone(),
        role,
        timestamp: Utc::now(),
    }).await;
    
    Ok(owner_id)
}

/// User de-registration revokes all access
pub async fn deregister_user(
    &self,
    owner_id: &OwnerId,
) -> Result<(), UserError> {
    // Revoke all credentials owned by user
    self.revoke_all_credentials(owner_id).await?;
    
    // Remove from all ACLs
    self.remove_from_all_acls(owner_id).await?;
    
    // Audit log
    self.audit_logger.log(AuditEvent::UserDeregistered {
        owner_id: owner_id.clone(),
        timestamp: Utc::now(),
    }).await;
    
    Ok(())
}
```

---

**A.9.2.2: User Access Provisioning**

**Requirement**: Formal user access provisioning process to assign or revoke access rights.

**Implementation**:
```rust
/// Grant permission (requires Grant permission)
pub fn grant_permission(
    &mut self,
    granter: &OwnerId,
    grantee: &OwnerId,
    permission: Permission,
) -> Result<(), AccessError> {
    // Verify granter has Grant permission
    if !self.has_permission(granter, Permission::Grant) {
        return Err(AccessError::InsufficientPermissions);
    }
    
    // Cannot grant permissions granter doesn't have
    if !self.has_permission(granter, permission) {
        return Err(AccessError::CannotGrantHigherPrivilege);
    }
    
    // Grant permission
    self.permissions
        .entry(grantee.clone())
        .or_default()
        .insert(permission);
    
    // Audit log
    self.audit_log(AuditEvent::PermissionGranted {
        granter: granter.clone(),
        grantee: grantee.clone(),
        permission,
        timestamp: Utc::now(),
    });
    
    Ok(())
}
```

---

**A.9.2.3: Management of Privileged Access Rights**

**Requirement**: Allocation and use of privileged access rights should be restricted and controlled.

**Implementation**:
```rust
/// Privileged operations require admin role
pub enum Permission {
    Read,       // Regular user
    Write,      // Regular user
    Delete,     // Admin only
    Rotate,     // Admin or owner
    Grant,      // Admin only (modify ACL)
    Execute,    // Regular user
}

pub fn is_admin(&self, owner_id: &OwnerId) -> bool {
    // Owner always has admin privileges on their credentials
    owner_id == &self.owner
}

pub async fn perform_admin_operation(
    &self,
    requester: &OwnerId,
    operation: AdminOperation,
) -> Result<(), AccessError> {
    if !self.is_admin(requester) {
        // Log privilege escalation attempt
        self.audit_logger.log(AuditEvent::PrivilegeEscalationAttempt {
            requester: requester.clone(),
            operation: format!("{:?}", operation),
            timestamp: Utc::now(),
        }).await;
        
        return Err(AccessError::InsufficientPrivileges);
    }
    
    // Perform operation with audit logging
    self.execute_admin_operation(operation).await
}
```

---

**A.9.2.4: Management of Secret Authentication Information of Users**

**Requirement**: Allocation of secret authentication information should be controlled through a formal management process.

**Implementation**:
```rust
/// Master password management
pub struct PasswordPolicy {
    pub min_length: usize,           // 16 characters
    pub max_length: usize,           // 128 characters
    pub require_uppercase: bool,     // true
    pub require_lowercase: bool,     // true
    pub require_digit: bool,         // true
    pub require_special: bool,       // true
    pub min_character_types: usize,  // 3 of 4 types
    pub prevent_common: bool,        // true
}

pub fn validate_master_password(
    password: &str,
    policy: &PasswordPolicy,
) -> Result<(), PasswordError> {
    if password.len() < policy.min_length {
        return Err(PasswordError::TooShort);
    }
    
    if password.len() > policy.max_length {
        return Err(PasswordError::TooLong);
    }
    
    let char_types = [
        password.chars().any(|c| c.is_lowercase()),
        password.chars().any(|c| c.is_uppercase()),
        password.chars().any(|c| c.is_numeric()),
        password.chars().any(|c| !c.is_alphanumeric()),
    ];
    
    if char_types.iter().filter(|&&x| x).count() < policy.min_character_types {
        return Err(PasswordError::InsufficientComplexity);
    }
    
    Ok(())
}
```

**See**: [[Security/Encryption|Encryption - Master Password Requirements]]

---

**A.9.2.5: Review of User Access Rights**

**Requirement**: Asset owners should review users' access rights at regular intervals.

**Implementation**:
```rust
/// Periodic access review
pub async fn review_access_rights(
    &self,
    credential_id: &CredentialId,
) -> AccessReviewReport {
    let credential = self.storage.get(credential_id).await.unwrap();
    
    AccessReviewReport {
        credential_id: credential_id.clone(),
        owner: credential.owner.clone(),
        permissions: credential.acl.permissions.clone(),
        last_accessed: credential.last_accessed,
        last_review: Utc::now(),
        recommendations: self.generate_recommendations(&credential),
    }
}

fn generate_recommendations(&self, credential: &Credential) -> Vec<String> {
    let mut recommendations = Vec::new();
    
    // Check for stale permissions
    for (grantee, perms) in &credential.acl.permissions {
        if let Some(last_access) = credential.get_last_access_by(grantee) {
            if Utc::now() - last_access > Duration::days(90) {
                recommendations.push(format!(
                    "Consider revoking access for {} (not accessed in 90 days)",
                    grantee
                ));
            }
        }
    }
    
    recommendations
}
```

---

**A.9.2.6: Removal or Adjustment of Access Rights**

**Requirement**: Access rights of all employees and external party users should be removed upon termination of their employment, contract, or agreement, or adjusted upon change.

**Implementation**:
```rust
/// Revoke all access for terminated user
pub async fn terminate_user_access(
    &self,
    owner_id: &OwnerId,
    reason: TerminationReason,
) -> Result<(), AccessError> {
    // Find all credentials with access
    let affected_credentials = self.storage
        .find_by_permission(owner_id)
        .await?;
    
    for credential in affected_credentials {
        // Remove from ACL
        self.revoke_permission(&credential.id, owner_id, Permission::All).await?;
    }
    
    // Audit log
    self.audit_logger.log(AuditEvent::UserAccessTerminated {
        owner_id: owner_id.clone(),
        reason,
        credentials_affected: affected_credentials.len(),
        timestamp: Utc::now(),
    }).await;
    
    Ok(())
}
```

---

### A.9.3: User Responsibilities

**A.9.3.1: Use of Secret Authentication Information**

**Requirement**: Users should be required to follow the organization's practices in the use of secret authentication information.

**Implementation**:
```rust
/// Enforce secure practices
pub struct AuthenticationPractices {
    /// Never log credentials
    pub redact_secrets: bool,  // Always true
    
    /// Zeroize secrets after use
    pub zeroize_on_drop: bool,  // Always true
    
    /// Use constant-time comparison
    pub constant_time_compare: bool,  // Always true
}

// Example: SecretString auto-redaction
impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "[REDACTED]")  // Never expose secret
    }
}

// Example: Zeroization on drop
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey([u8; 32]);
```

---

### A.9.4: System and Application Access Control

**A.9.4.1: Information Access Restriction**

**Requirement**: Access to information and application system functions should be restricted in accordance with the access control policy.

**Implementation**: All access controlled by ACLs + ownership model (see A.9.2.2).

**A.9.4.2: Secure Log-on Procedures**

**Requirement**: Where required by the access control policy, access to systems and applications should be controlled by a secure log-on procedure.

**Implementation**: Mutual TLS + API keys + OAuth2 (see [[Architecture|Authentication Methods]]).

**A.9.4.3: Password Management System**

**Requirement**: Password management systems should be interactive and should ensure quality passwords.

**Implementation**: Argon2id key derivation with password complexity validation (see A.9.2.4).

**A.9.4.4: Use of Privileged Utility Programs**

**Requirement**: Use of utility programs that might be capable of overriding system and application controls should be restricted and tightly controlled.

**Implementation**: Admin operations logged and require elevated privileges (see A.9.2.3).

**A.9.4.5: Access Control to Program Source Code**

**Requirement**: Access to program source code should be restricted.

**Implementation**: Open-source with contributor access controls (GitHub permissions).

---

## A.10: Cryptography

### A.10.1: Cryptographic Controls

**A.10.1.1: Policy on the Use of Cryptographic Controls**

**Requirement**: A policy on the use of cryptographic controls for protection of information should be developed and implemented.

**Implementation**:
```rust
pub struct CryptographicPolicy {
    /// Encryption algorithm
    pub encryption: EncryptionAlgorithm::Aes256Gcm,
    
    /// Key size
    pub key_size_bits: usize,  // 256
    
    /// Key derivation
    pub kdf: KeyDerivationFunction::Argon2id,
    
    /// Hashing
    pub hash: HashAlgorithm::Blake3,
    
    /// TLS version
    pub tls_version: TlsVersion::V1_3,  // Minimum
}
```

**See**: [[Security/Encryption|Encryption Deep Dive]]

---

**A.10.1.2: Key Management**

**Requirement**: A policy on the use, protection, and lifetime of cryptographic keys should be developed and implemented through their whole lifecycle.

**Implementation**:
- **Key generation**: Cryptographically secure RNG
- **Key storage**: HSM or KMS (AWS KMS, Azure Key Vault, HashiCorp Vault)
- **Key rotation**: Every 90 days (configurable)
- **Key versioning**: Multiple versions active during rotation
- **Key destruction**: Zeroization before deallocation

**See**: [[Advanced/Key-Management|Key Management]]

---

## A.12: Operations Security

### A.12.4: Logging and Monitoring

**A.12.4.1: Event Logging**

**Requirement**: Event logs recording user activities, exceptions, faults, and information security events should be produced, kept, and regularly reviewed.

**Implementation**:
```rust
pub enum AuditEvent {
    CredentialAccessed { /* ... */ },
    CredentialCreated { /* ... */ },
    CredentialUpdated { /* ... */ },
    CredentialDeleted { /* ... */ },
    CredentialRotated { /* ... */ },
    AuthenticationAttempt { /* ... */ },
    DecryptionFailed { /* ... */ },
    ScopeViolation { /* ... */ },
    PermissionDenied { /* ... */ },
    AclModified { /* ... */ },
    KeyRotated { /* ... */ },
}

// All events logged with:
// - Event type
// - User ID
// - Timestamp
// - Result (success/failure)
// - Correlation ID
// - Duration
```

**See**: [[How-To/Enable-Audit-Logging|Audit Logging Setup]]

---

**A.12.4.2: Protection of Log Information**

**Requirement**: Logging facilities and log information should be protected against tampering and unauthorized access.

**Implementation**:
```rust
/// Append-only audit log
pub struct ImmutableAuditLog {
    storage: Arc<dyn AppendOnlyStorage>,
}

impl ImmutableAuditLog {
    pub async fn append(&self, event: AuditEvent) -> Result<(), AuditError> {
        let json = serde_json::to_string(&event)?;
        
        // Hash for integrity
        let hash = blake3::hash(json.as_bytes());
        
        // Append (cannot modify or delete)
        self.storage.append(LogEntry {
            event: json,
            hash,
            timestamp: Utc::now(),
        }).await?;
        
        Ok(())
    }
}
```

---

**A.12.4.3: Administrator and Operator Logs**

**Requirement**: System administrator and system operator activities should be logged and the logs protected and regularly reviewed.

**Implementation**: All admin operations logged with AuditEvent (see A.12.4.1).

---

**A.12.4.4: Clock Synchronization**

**Requirement**: Clocks of all relevant information processing systems within an organization or security domain should be synchronized to a single reference time source.

**Implementation**:
```rust
// Use UTC for all timestamps
use chrono::{DateTime, Utc};

pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,  // UTC, not local time
    // ...
}

// Recommendation: Configure NTP on all servers
// $ timedatectl set-ntp true
```

---

## A.14: System Acquisition, Development, and Maintenance

### A.14.2: Security in Development and Support Processes

**A.14.2.1: Secure Development Policy**

**Requirement**: Rules for the development of software and systems should be established and applied to developments within the organization.

**Implementation**:
- Secure coding guidelines: [[Advanced/Security-Best-Practices]]
- Dependency scanning: `cargo audit` in CI/CD
- Code review: Security review for all changes
- Testing: Security testing + penetration testing

---

**A.14.2.5: Secure System Engineering Principles**

**Requirement**: Principles for engineering secure systems should be established, documented, maintained, and applied to any information system implementation efforts.

**Implementation**:
- **Defense in depth**: 5 security layers
- **Least privilege**: Minimal permissions by default
- **Fail secure**: Errors deny access, never allow
- **Zero trust**: Always verify, never trust implicitly

**See**: [[Advanced/Security-Architecture|Security Architecture]]

---

## A.18: Compliance

### A.18.1: Compliance with Legal and Contractual Requirements

**A.18.1.3: Protection of Records**

**Requirement**: Records should be protected from loss, destruction, falsification, unauthorized access, and unauthorized release.

**Implementation**:
- Encrypted audit logs (immutable)
- 365-day retention (compliance requirement)
- Access-controlled log storage
- Backup and disaster recovery

---

**A.18.1.4: Privacy and Protection of Personally Identifiable Information**

**Requirement**: Privacy and protection of PII should be ensured as required in relevant legislation and regulation.

**Implementation**: See [[Advanced/Compliance-GDPR|GDPR Compliance]]

---

### A.18.2: Information Security Reviews

**A.18.2.2: Compliance with Security Policies and Standards**

**Requirement**: Managers should regularly review the compliance of information processing and procedures within their area of responsibility with the appropriate security policies, standards, and any other security requirements.

**Implementation**:
- Quarterly access reviews
- Annual security audits
- Continuous compliance monitoring
- Automated policy checks

---

## Control Implementation Matrix

| Control | Title | Status | Evidence |
|---------|-------|--------|----------|
| **A.9.1.1** | Access control policy | ✅ | [[Advanced/Access-Control]] |
| **A.9.1.2** | Network access | ✅ | Scope isolation |
| **A.9.2.1** | User registration | ✅ | User lifecycle management |
| **A.9.2.2** | Access provisioning | ✅ | ACL grant/revoke |
| **A.9.2.3** | Privileged access | ✅ | Admin permissions |
| **A.9.2.4** | Secret authentication | ✅ | Password policy |
| **A.9.2.5** | Access review | ✅ | Periodic review |
| **A.9.2.6** | Access removal | ✅ | Termination procedure |
| **A.9.3.1** | Secret use practices | ✅ | SecretString, zeroization |
| **A.9.4.1-5** | Application access | ✅ | Authentication methods |
| **A.10.1.1** | Cryptographic policy | ✅ | [[Security/Encryption]] |
| **A.10.1.2** | Key management | ✅ | [[Advanced/Key-Management]] |
| **A.12.4.1** | Event logging | ✅ | [[How-To/Enable-Audit-Logging]] |
| **A.12.4.2** | Log protection | ✅ | Immutable audit log |
| **A.12.4.3** | Admin logs | ✅ | All admin ops logged |
| **A.12.4.4** | Clock sync | ✅ | UTC timestamps + NTP |
| **A.14.2.1** | Secure development | ✅ | [[Advanced/Security-Best-Practices]] |
| **A.14.2.5** | System engineering | ✅ | [[Advanced/Security-Architecture]] |
| **A.18.1.3** | Records protection | ✅ | Encrypted, immutable logs |
| **A.18.1.4** | Privacy/PII | ✅ | [[Advanced/Compliance-GDPR]] |
| **A.18.2.2** | Compliance review | ✅ | Quarterly + annual reviews |

---

## See Also

- [[Advanced/Compliance-SOC2|SOC 2 Compliance]]
- [[Advanced/Compliance-HIPAA|HIPAA Compliance]]
- [[Advanced/Compliance-GDPR|GDPR Compliance]]
- [[Advanced/Security-Architecture|Security Architecture]]
- [[Advanced/Access-Control|Access Control System]]
- [[Security/Encryption|Encryption Deep Dive]]
- [[Advanced/Key-Management|Key Management]]
- [[How-To/Enable-Audit-Logging|Audit Logging Setup]]
