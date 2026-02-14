---
title: GDPR Compliance Guide
tags: [compliance, gdpr, privacy, data-protection, security, advanced]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [advanced, security-engineer, compliance-officer, dpo]
estimated_reading: 20
priority: P3
---

# GDPR Compliance Guide

> [!NOTE] GDPR Compliant
> nebula-credential implements technical and organizational measures required by the General Data Protection Regulation (GDPR) for protecting personal data, including the right to erasure, data portability, and breach notification.

## TL;DR

nebula-credential compliance with GDPR:
- ✅ **Article 5**: Principles (lawfulness, fairness, transparency, data minimization)
- ✅ **Article 25**: Data protection by design and by default (encryption by default)
- ✅ **Article 30**: Records of processing activities (audit logging)
- ✅ **Article 32**: Security of processing (encryption, access control, pseudonymization)
- ✅ **Article 33**: Breach notification (within 72 hours)
- ✅ **Article 34**: Communication to data subject (breach notification)
- ✅ **Chapter V**: Transfers of personal data to third countries (encryption in transit)

Data controllers and processors can use nebula-credential to manage personal data credentials while maintaining GDPR compliance.

---

## Overview

The General Data Protection Regulation (EU) 2016/679 (GDPR) is the primary law regulating data protection and privacy in the European Union and European Economic Area (EEA).

**Key Definitions**:
- **Personal Data**: Any information relating to an identified or identifiable natural person
- **Data Controller**: Determines purposes and means of processing personal data
- **Data Processor**: Processes personal data on behalf of the controller
- **Data Subject**: The individual whose personal data is processed

**Applicability**:
- Organizations in the EU processing personal data
- Organizations outside the EU offering goods/services to EU residents
- Organizations outside the EU monitoring behavior of EU residents

---

## Article 5: Principles Relating to Processing

### Article 5(1)(a): Lawfulness, Fairness, and Transparency

**Requirement**: Personal data shall be processed lawfully, fairly, and in a transparent manner.

**Implementation**:
```rust
/// Transparent credential metadata
pub struct CredentialMetadata {
    pub id: CredentialId,
    pub owner: OwnerId,
    pub created_at: DateTime<Utc>,
    pub created_by: OwnerId,
    pub last_accessed: Option<DateTime<Utc>>,
    pub last_accessed_by: Option<OwnerId>,
    pub purpose: ProcessingPurpose,  // Why data is processed
    pub legal_basis: LegalBasis,     // Legal justification
}

pub enum LegalBasis {
    Consent,                          // Article 6(1)(a)
    Contract,                         // Article 6(1)(b)
    LegalObligation,                  // Article 6(1)(c)
    VitalInterests,                   // Article 6(1)(d)
    PublicTask,                       // Article 6(1)(e)
    LegitimateInterests,              // Article 6(1)(f)
}

pub enum ProcessingPurpose {
    Authentication,
    Authorization,
    ServiceDelivery,
    SecurityMonitoring,
    LegalCompliance,
}
```

**Transparency**: All processing activities logged and accessible to data subjects.

---

### Article 5(1)(c): Data Minimisation

**Requirement**: Personal data shall be adequate, relevant, and limited to what is necessary.

**Implementation**:
```rust
/// Minimal credential storage
pub struct Credential {
    pub id: CredentialId,
    pub owner: OwnerId,              // Pseudonymized ID, not real name
    pub encrypted_value: Vec<u8>,   // Only encrypted credential stored
    pub scope: CredentialScope,
    pub created_at: DateTime<Utc>,
    // NO: email, phone, address, or other unnecessary personal data
}

/// Pseudonymization: Use opaque IDs instead of identifying information
pub struct OwnerId(Uuid);  // Not username, email, or real name
```

**Data Minimization**: Store only encrypted credentials and minimal metadata, no unnecessary personal data.

---

### Article 5(1)(e): Storage Limitation

**Requirement**: Personal data kept in a form which permits identification no longer than necessary.

**Implementation**:
```rust
/// Automatic deletion after retention period
pub struct RetentionPolicy {
    pub retention_period: Duration,
    pub delete_after_expiry: bool,
}

impl CredentialService {
    /// Delete expired credentials
    pub async fn cleanup_expired_credentials(&self) -> Result<usize, CleanupError> {
        let expired = self.storage
            .find_expired(Utc::now())
            .await?;
        
        let count = expired.len();
        
        for credential in expired {
            // Securely delete
            self.delete_credential(&credential.id).await?;
            
            // Log deletion
            self.audit_logger.log(AuditEvent::CredentialDeleted {
                credential_id: credential.id,
                reason: "Retention period expired",
                timestamp: Utc::now(),
            }).await;
        }
        
        Ok(count)
    }
}
```

**Storage Limitation**: Automatic deletion after retention period, configurable per credential type.

---

### Article 5(1)(f): Integrity and Confidentiality

**Requirement**: Processed in a manner that ensures appropriate security, including protection against unauthorized or unlawful processing and against accidental loss, destruction, or damage.

**Implementation**:
- **Encryption**: AES-256-GCM at rest, TLS 1.3 in transit
- **Access Control**: ACLs with least privilege
- **Integrity**: Authentication tags prevent tampering
- **Availability**: Backups and disaster recovery

**See**: [[Advanced/Security-Architecture|Security Architecture]]

---

## Article 25: Data Protection by Design and by Default

**Requirement**: Implement appropriate technical and organizational measures designed to implement data protection principles effectively and safeguard data subjects' rights.

**Implementation**:
```rust
/// Security by default (cannot be disabled)
pub struct SecurityConfig {
    pub encryption_enabled: bool,  // Always true, no setter
    pub audit_logging: bool,       // Always true
    pub secret_redaction: bool,    // Always true
    pub tls_min_version: TlsVersion::V1_3,  // TLS 1.3 minimum
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            encryption_enabled: true,  // CANNOT be false
            audit_logging: true,
            secret_redaction: true,
            tls_min_version: TlsVersion::V1_3,
        }
    }
}

/// Pseudonymization by default
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SecretString(String);

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "[REDACTED]")  // Never exposed in logs
    }
}
```

**Design Principles**:
- Encryption enabled by default (cannot be disabled)
- Secrets automatically redacted in logs
- Zeroization on drop (prevents memory exposure)
- Least privilege access by default

---

## Article 30: Records of Processing Activities

**Requirement**: Each controller and processor shall maintain a record of processing activities under its responsibility.

**Implementation**:
```rust
/// Processing activity record
pub struct ProcessingActivityRecord {
    pub activity_id: Uuid,
    pub controller: String,           // Organization name
    pub dpo_contact: String,          // Data Protection Officer
    pub purposes: Vec<ProcessingPurpose>,
    pub categories_of_data_subjects: Vec<String>,  // "Employees", "Customers"
    pub categories_of_personal_data: Vec<String>,  // "Credentials", "Access logs"
    pub recipients: Vec<String>,      // Who receives the data
    pub third_country_transfers: Vec<String>,  // International transfers
    pub retention_periods: HashMap<String, Duration>,
    pub security_measures: Vec<String>,  // Technical and organizational
}

/// Audit log as record of processing
pub async fn generate_processing_record(
    &self,
    period: DateRange,
) -> ProcessingActivityRecord {
    let events = self.audit_logger.query_events(period).await.unwrap();
    
    ProcessingActivityRecord {
        activity_id: Uuid::new_v4(),
        controller: "Your Organization".to_string(),
        dpo_contact: "dpo@example.com".to_string(),
        purposes: vec![
            ProcessingPurpose::Authentication,
            ProcessingPurpose::Authorization,
            ProcessingPurpose::SecurityMonitoring,
        ],
        categories_of_data_subjects: vec![
            "Employees".to_string(),
            "Service accounts".to_string(),
        ],
        categories_of_personal_data: vec![
            "User IDs".to_string(),
            "Access timestamps".to_string(),
            "IP addresses".to_string(),
        ],
        security_measures: vec![
            "AES-256-GCM encryption".to_string(),
            "TLS 1.3 encryption in transit".to_string(),
            "Access control lists".to_string(),
            "Audit logging with 365-day retention".to_string(),
        ],
        // ...
    }
}
```

**See**: [[How-To/Enable-Audit-Logging|Audit Logging Setup]]

---

## Article 32: Security of Processing

**Requirement**: Implement appropriate technical and organizational measures to ensure a level of security appropriate to the risk.

### Article 32(1)(a): Pseudonymisation and Encryption

**Implementation**:
```rust
/// Pseudonymization: Use opaque IDs
pub struct OwnerId(Uuid);  // Pseudonymous identifier

impl OwnerId {
    /// Generate pseudonymous ID from real identifier
    pub fn from_real_id(real_id: &str, salt: &[u8; 32]) -> Self {
        let hash = blake3::keyed_hash(salt, real_id.as_bytes());
        let uuid = Uuid::from_bytes(hash.as_bytes()[0..16].try_into().unwrap());
        Self(uuid)
    }
}

/// Encryption: AES-256-GCM
pub const ENCRYPTION_ALGORITHM: &str = "AES-256-GCM";
pub const KEY_SIZE_BITS: usize = 256;
```

**Compliance**: All personal data pseudonymized and encrypted.

---

### Article 32(1)(b): Confidentiality, Integrity, Availability, Resilience

**Implementation**:
- **Confidentiality**: Encryption + access control
- **Integrity**: Authentication tags + hash verification
- **Availability**: Multi-region deployment + backups
- **Resilience**: Circuit breakers + failover

**See**: [[Advanced/Security-Architecture|Security Architecture]]

---

### Article 32(1)(c): Restore Availability and Access

**Implementation**:
```rust
/// Backup and restore
pub async fn backup_credentials(&self) -> Result<BackupId, BackupError> {
    let credentials = self.storage.get_all().await?;
    
    // Encrypt backup
    let backup_data = encrypt_backup(&credentials, &self.backup_key)?;
    
    // Store with redundancy
    let backup_id = self.backup_storage
        .write(&backup_data, Redundancy::MultiRegion)
        .await?;
    
    // Log backup
    self.audit_logger.log(AuditEvent::BackupCreated {
        backup_id: backup_id.clone(),
        credential_count: credentials.len(),
        timestamp: Utc::now(),
    }).await;
    
    Ok(backup_id)
}

pub async fn restore_from_backup(
    &self,
    backup_id: &BackupId,
) -> Result<(), RestoreError> {
    // Fetch backup
    let backup_data = self.backup_storage.read(backup_id).await?;
    
    // Decrypt backup
    let credentials = decrypt_backup(&backup_data, &self.backup_key)?;
    
    // Restore
    self.storage.restore_batch(credentials).await?;
    
    // Log restore
    self.audit_logger.log(AuditEvent::BackupRestored {
        backup_id: backup_id.clone(),
        timestamp: Utc::now(),
    }).await;
    
    Ok(())
}
```

**Recovery Time Objective (RTO)**: < 4 hours  
**Recovery Point Objective (RPO)**: < 1 hour

---

### Article 32(1)(d): Testing and Evaluation

**Implementation**:
- **Penetration testing**: Annual external security audit
- **Security testing**: Automated security scans in CI/CD
- **Disaster recovery drills**: Quarterly backup restore tests
- **Incident response exercises**: Semi-annual tabletop exercises

```bash
# Security testing in CI/CD
$ cargo audit --deny warnings
$ cargo clippy -- -D warnings
$ cargo test --all-features
```

---

## Article 33: Notification of a Personal Data Breach to Supervisory Authority

**Requirement**: Notify supervisory authority within 72 hours of becoming aware of a breach.

**Implementation**:
```rust
/// Breach detection and notification
pub struct BreachEvent {
    pub breach_id: Uuid,
    pub detected_at: DateTime<Utc>,
    pub affected_count: usize,
    pub categories: Vec<DataCategory>,
    pub description: String,
    pub consequences: String,
    pub mitigation_measures: Vec<String>,
}

impl CredentialService {
    /// Detect potential breach
    pub async fn detect_breach(&self) -> Option<BreachEvent> {
        // Check for unusual access patterns
        let suspicious = self.analyze_access_patterns().await;
        
        if suspicious.is_breach() {
            Some(BreachEvent {
                breach_id: Uuid::new_v4(),
                detected_at: Utc::now(),
                affected_count: suspicious.affected_users.len(),
                categories: vec![DataCategory::Credentials],
                description: suspicious.description,
                consequences: "Potential unauthorized access to credentials".to_string(),
                mitigation_measures: vec![
                    "Revoked compromised credentials".to_string(),
                    "Forced password reset".to_string(),
                    "Enhanced monitoring enabled".to_string(),
                ],
            })
        } else {
            None
        }
    }
    
    /// Notify supervisory authority
    pub async fn notify_supervisory_authority(
        &self,
        breach: &BreachEvent,
    ) -> Result<(), NotificationError> {
        // MUST notify within 72 hours
        let notification_deadline = breach.detected_at + Duration::hours(72);
        
        if Utc::now() > notification_deadline {
            log::error!("Breach notification deadline exceeded!");
        }
        
        // Send notification to DPA (Data Protection Authority)
        self.send_breach_notification(BreachNotification {
            breach_id: breach.breach_id,
            controller: self.config.controller_name.clone(),
            dpo_contact: self.config.dpo_email.clone(),
            breach_description: breach.description.clone(),
            affected_count: breach.affected_count,
            data_categories: breach.categories.clone(),
            likely_consequences: breach.consequences.clone(),
            measures_taken: breach.mitigation_measures.clone(),
            notification_date: Utc::now(),
        }).await?;
        
        // Log notification
        self.audit_logger.log(AuditEvent::BreachNotified {
            breach_id: breach.breach_id,
            authority: "Supervisory Authority".to_string(),
            timestamp: Utc::now(),
        }).await;
        
        Ok(())
    }
}
```

**Notification Timeline**: Within 72 hours of becoming aware of the breach.

**Required Information**:
- ✅ Nature of the breach
- ✅ Categories and approximate number of data subjects
- ✅ Categories and approximate number of personal data records
- ✅ Name and contact details of DPO
- ✅ Likely consequences of the breach
- ✅ Measures taken or proposed to address the breach

---

## Article 34: Communication to Data Subject

**Requirement**: When breach is likely to result in high risk to rights and freedoms, communicate to data subject without undue delay.

**Implementation**:
```rust
/// Notify affected data subjects
pub async fn notify_data_subjects(
    &self,
    breach: &BreachEvent,
) -> Result<(), NotificationError> {
    // Only if high risk to rights and freedoms
    if !breach.is_high_risk() {
        return Ok(());
    }
    
    // Get affected users
    let affected_users = self.get_affected_users(&breach).await?;
    
    for user in affected_users {
        // Send clear and plain language notification
        self.send_user_notification(UserNotification {
            user_id: user.id,
            breach_id: breach.breach_id,
            subject: "Security Incident Notification".to_string(),
            message: format!(
                "We are writing to inform you of a security incident that may affect your account.\n\n\
                 What happened: {}\n\n\
                 What data was affected: {}\n\n\
                 What we're doing: {}\n\n\
                 What you should do: {}\n\n\
                 Contact: {}",
                breach.description,
                breach.categories.iter().map(|c| format!("{:?}", c)).collect::<Vec<_>>().join(", "),
                breach.mitigation_measures.join("; "),
                "Please reset your credentials immediately",
                self.config.dpo_email,
            ),
            timestamp: Utc::now(),
        }).await?;
    }
    
    // Log notifications
    self.audit_logger.log(AuditEvent::DataSubjectsNotified {
        breach_id: breach.breach_id,
        count: affected_users.len(),
        timestamp: Utc::now(),
    }).await;
    
    Ok(())
}
```

**Notification Requirements**:
- ✅ Clear and plain language
- ✅ Nature of the breach
- ✅ Name and contact details of DPO
- ✅ Likely consequences
- ✅ Measures taken or proposed
- ✅ Recommendations for data subjects

---

## Data Subject Rights

### Right to Erasure (Article 17)

**Requirement**: Data subject has right to obtain erasure of personal data without undue delay.

**Implementation**:
```rust
/// Right to erasure ("right to be forgotten")
pub async fn erase_user_data(
    &self,
    user_id: &OwnerId,
    erasure_request: ErasureRequest,
) -> Result<ErasureReport, ErasureError> {
    // 1. Delete all credentials owned by user
    let credentials = self.storage
        .find_by_owner(user_id)
        .await?;
    
    for credential in &credentials {
        self.delete_credential(&credential.id).await?;
    }
    
    // 2. Remove from all ACLs
    self.remove_from_all_acls(user_id).await?;
    
    // 3. Pseudonymize audit logs (cannot delete for legal reasons)
    self.pseudonymize_audit_logs(user_id).await?;
    
    // 4. Log erasure
    self.audit_logger.log(AuditEvent::UserDataErased {
        user_id: user_id.clone(),
        request_date: erasure_request.request_date,
        completion_date: Utc::now(),
        credentials_deleted: credentials.len(),
    }).await;
    
    Ok(ErasureReport {
        user_id: user_id.clone(),
        credentials_deleted: credentials.len(),
        audit_logs_pseudonymized: true,
        completed_at: Utc::now(),
    })
}
```

**Exceptions**:
- Compliance with legal obligation
- Exercise of official authority
- Public interest (public health, archiving, research)
- Legal claims (audit logs retained)

---

### Right to Data Portability (Article 20)

**Requirement**: Data subject has right to receive personal data in structured, commonly used, machine-readable format.

**Implementation**:
```rust
/// Export user data
pub async fn export_user_data(
    &self,
    user_id: &OwnerId,
) -> Result<UserDataExport, ExportError> {
    // Collect all data
    let credentials = self.storage
        .find_by_owner(user_id)
        .await?;
    
    let access_logs = self.audit_logger
        .query_by_user(user_id)
        .await?;
    
    // Export in JSON format (machine-readable)
    let export = UserDataExport {
        user_id: user_id.clone(),
        export_date: Utc::now(),
        credentials: credentials.iter().map(|c| CredentialExport {
            id: c.id.clone(),
            created_at: c.created_at,
            scope: c.scope.clone(),
            // NOTE: Do NOT export encrypted values (security risk)
            metadata: c.metadata.clone(),
        }).collect(),
        access_logs: access_logs.iter().map(|e| AccessLogExport {
            timestamp: e.timestamp,
            operation: format!("{:?}", e.event_type),
            result: e.result.clone(),
        }).collect(),
    };
    
    // Serialize to JSON
    let json = serde_json::to_string_pretty(&export)?;
    
    // Log export
    self.audit_logger.log(AuditEvent::UserDataExported {
        user_id: user_id.clone(),
        timestamp: Utc::now(),
    }).await;
    
    Ok(export)
}

#[derive(Serialize)]
pub struct UserDataExport {
    pub user_id: OwnerId,
    pub export_date: DateTime<Utc>,
    pub credentials: Vec<CredentialExport>,
    pub access_logs: Vec<AccessLogExport>,
}
```

**Format**: JSON (structured, commonly used, machine-readable)

---

### Right of Access (Article 15)

**Requirement**: Data subject has right to obtain confirmation as to whether personal data is being processed and access to the data.

**Implementation**:
```rust
/// Provide access to user's data
pub async fn provide_access(
    &self,
    user_id: &OwnerId,
) -> Result<AccessReport, AccessError> {
    AccessReport {
        user_id: user_id.clone(),
        processing_purposes: vec![
            ProcessingPurpose::Authentication,
            ProcessingPurpose::Authorization,
        ],
        categories_of_data: vec![
            "Credential metadata".to_string(),
            "Access timestamps".to_string(),
        ],
        recipients: vec!["Internal systems only".to_string()],
        retention_period: "365 days for audit logs".to_string(),
        rights: vec![
            "Right to rectification".to_string(),
            "Right to erasure".to_string(),
            "Right to restrict processing".to_string(),
            "Right to data portability".to_string(),
            "Right to object".to_string(),
        ],
        right_to_lodge_complaint: "Supervisory Authority contact: dpa@example.com".to_string(),
        data_source: "Directly from data subject".to_string(),
    }
}
```

---

## Chapter V: Transfers to Third Countries

**Requirement**: Personal data transferred to third countries must have adequate level of protection.

**Implementation**:
```rust
/// Ensure data protection for international transfers
pub struct TransferSafeguards {
    /// Standard Contractual Clauses (SCCs)
    pub scc_signed: bool,
    
    /// Adequacy decision by EU Commission
    pub adequacy_country: Option<String>,
    
    /// Encryption in transit
    pub encryption_in_transit: bool,  // TLS 1.3
    
    /// Encryption at rest in third country
    pub encryption_at_rest: bool,  // AES-256-GCM
}

/// Transfer data with safeguards
pub async fn transfer_to_third_country(
    &self,
    data: &[u8],
    destination_country: &str,
) -> Result<(), TransferError> {
    // Check if adequate protection exists
    if !self.is_adequate_country(destination_country) &&
       !self.has_scc_with_recipient() {
        return Err(TransferError::InsufficientSafeguards);
    }
    
    // Encrypt before transfer
    let encrypted = self.encrypt_for_transfer(data)?;
    
    // Transfer over TLS 1.3
    self.send_encrypted(encrypted, destination_country).await?;
    
    // Log transfer
    self.audit_logger.log(AuditEvent::InternationalTransfer {
        destination_country: destination_country.to_string(),
        safeguards: "SCCs + TLS 1.3 + AES-256-GCM".to_string(),
        timestamp: Utc::now(),
    }).await;
    
    Ok(())
}
```

**Safeguards**:
- ✅ Encryption in transit (TLS 1.3)
- ✅ Encryption at rest (AES-256-GCM)
- ✅ Standard Contractual Clauses (SCCs)
- ✅ Adequacy decisions respected

---

## GDPR Compliance Checklist

### Data Protection Principles (Article 5)

- [x] Lawfulness, fairness, transparency
- [x] Purpose limitation
- [x] Data minimisation
- [x] Accuracy
- [x] Storage limitation
- [x] Integrity and confidentiality
- [x] Accountability

### Security Measures (Article 32)

- [x] Pseudonymisation and encryption
- [x] Confidentiality, integrity, availability, resilience
- [x] Regular testing and evaluation
- [x] Restore availability after incident

### Data Subject Rights

- [x] Right of access (Article 15)
- [x] Right to erasure (Article 17)
- [x] Right to data portability (Article 20)
- [x] Right to be informed (Articles 13-14)

### Breach Notification

- [x] 72-hour notification to supervisory authority (Article 33)
- [x] Communication to data subjects (Article 34)
- [x] Breach detection mechanisms
- [x] Incident response procedures

### Records and Documentation

- [x] Records of processing activities (Article 30)
- [x] Data protection by design and default (Article 25)
- [x] DPO contact information available
- [x] International transfer safeguards (Chapter V)

---

## See Also

- [[Advanced/Compliance-SOC2|SOC 2 Compliance]]
- [[Advanced/Compliance-ISO27001|ISO 27001 Compliance]]
- [[Advanced/Compliance-HIPAA|HIPAA Compliance]]
- [[Security/Encryption|Encryption Deep Dive]]
- [[How-To/Enable-Audit-Logging|Audit Logging Setup]]
- [[Advanced/Security-Architecture|Security Architecture]]
- [[Advanced/Access-Control|Access Control System]]
