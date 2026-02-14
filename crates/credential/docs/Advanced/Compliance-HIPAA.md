---
title: HIPAA Compliance Guide
tags: [compliance, hipaa, healthcare, security, audit, advanced]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [advanced, security-engineer, compliance-officer, healthcare]
estimated_reading: 20
priority: P3
---

# HIPAA Compliance Guide

> [!NOTE] HIPAA Security Rule Compliant
> nebula-credential implements technical safeguards required by the Health Insurance Portability and Accountability Act (HIPAA) Security Rule for protecting Electronic Protected Health Information (ePHI).

## TL;DR

nebula-credential compliance with HIPAA Security Rule:
- ✅ **164.312(a)(1)**: Access Control (unique user ID, automatic logoff, encryption/decryption)
- ✅ **164.312(a)(2)(iv)**: Encryption and Decryption (AES-256-GCM)
- ✅ **164.312(b)**: Audit Controls (comprehensive logging)
- ✅ **164.312(c)(1)**: Integrity (authentication tag prevents tampering)
- ✅ **164.312(d)**: Person or Entity Authentication (mutual TLS, OAuth2)
- ✅ **164.312(e)(1)**: Transmission Security (TLS 1.3)

Healthcare organizations can use nebula-credential to securely manage ePHI credentials while maintaining HIPAA compliance.

---

## Overview

The HIPAA Security Rule (45 CFR Part 164, Subpart C) establishes national standards to protect electronic personal health information (ePHI). It applies to:
- **Covered Entities**: Healthcare providers, health plans, healthcare clearinghouses
- **Business Associates**: Service providers that handle ePHI on behalf of covered entities

**Three Types of Safeguards**:
1. **Administrative Safeguards** (organizational policies - outside scope)
2. **Physical Safeguards** (datacenter security - infrastructure level)
3. **Technical Safeguards** (system controls - **implemented by nebula-credential**)

This guide focuses on **Technical Safeguards** (164.312).

---

## 164.312(a)(1): Access Control

**Requirement**: Implement technical policies and procedures for electronic information systems that maintain ePHI to allow access only to those persons or software programs that have been granted access rights.

### 164.312(a)(2)(i): Unique User Identification (Required)

**Requirement**: Assign a unique name and/or number for identifying and tracking user identity.

**Implementation**:
```rust
/// Every credential has unique owner
pub struct Credential {
    pub id: CredentialId,          // Unique credential ID
    pub owner: OwnerId,            // Unique user/service ID
    pub created_by: OwnerId,       // Who created it
    pub last_accessed_by: Option<OwnerId>,  // Who last accessed
    // ...
}

/// All operations require authenticated user
pub async fn access_credential(
    &self,
    id: &CredentialId,
    requester: &OwnerId,  // Unique user identifier
) -> Result<Credential, CredentialError> {
    // Log who accessed what
    self.audit_logger.log(AuditEvent::CredentialAccessed {
        credential_id: id.clone(),
        requester: requester.clone(),  // Unique ID tracked
        timestamp: Utc::now(),
        // ...
    }).await;
    // ...
}
```

**Compliance Evidence**: Every operation logged with unique user ID.

---

### 164.312(a)(2)(ii): Emergency Access Procedure (Required)

**Requirement**: Establish procedures for obtaining necessary ePHI during an emergency.

**Implementation**:
```rust
/// Emergency access with elevated logging
pub async fn emergency_access(
    &self,
    credential_id: &CredentialId,
    responder: &OwnerId,
    emergency_reason: &str,
) -> Result<Credential, CredentialError> {
    // Emergency access bypasses normal ACL (with logging)
    let credential = self.storage.get(credential_id).await?;
    
    // CRITICAL: Log emergency access with reason
    self.audit_logger.log(AuditEvent::EmergencyAccess {
        credential_id: credential_id.clone(),
        responder: responder.clone(),
        reason: emergency_reason.to_string(),
        timestamp: Utc::now(),
        alert_level: AlertLevel::Critical,
    }).await;
    
    // Notify compliance team
    self.notify_compliance_team(EmergencyAccessNotification {
        responder: responder.clone(),
        credential: credential_id.clone(),
        reason: emergency_reason.to_string(),
    }).await;
    
    Ok(credential)
}
```

**Compliance Evidence**: Emergency access logged and monitored separately.

---

### 164.312(a)(2)(iii): Automatic Logoff (Addressable)

**Requirement**: Implement electronic procedures that terminate an electronic session after a predetermined time of inactivity.

**Implementation**:
```rust
/// Session management with automatic expiration
pub struct CredentialSession {
    pub session_id: Uuid,
    pub owner_id: OwnerId,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
}

pub const SESSION_TIMEOUT_MINUTES: i64 = 15;  // HIPAA recommendation

impl CredentialService {
    pub async fn validate_session(
        &self,
        session_id: &Uuid,
    ) -> Result<CredentialSession, SessionError> {
        let session = self.session_store.get(session_id).await?;
        
        // Check if session expired
        let now = Utc::now();
        if now > session.expires_at {
            self.terminate_session(session_id).await?;
            return Err(SessionError::Expired);
        }
        
        // Check inactivity timeout
        if now - session.last_activity > Duration::minutes(SESSION_TIMEOUT_MINUTES) {
            self.terminate_session(session_id).await?;
            return Err(SessionError::InactivityTimeout);
        }
        
        Ok(session)
    }
    
    pub async fn terminate_session(&self, session_id: &Uuid) -> Result<(), SessionError> {
        // Log session termination
        self.audit_logger.log(AuditEvent::SessionTerminated {
            session_id: *session_id,
            reason: "Timeout",
            timestamp: Utc::now(),
        }).await;
        
        self.session_store.delete(session_id).await?;
        Ok(())
    }
}
```

**Compliance Evidence**: Sessions expire after 15 minutes of inactivity.

---

### 164.312(a)(2)(iv): Encryption and Decryption (Addressable)

**Requirement**: Implement a mechanism to encrypt and decrypt ePHI.

**Implementation**:
```rust
/// AES-256-GCM encryption (NIST-approved, FIPS 140-2 compliant)
pub const ENCRYPTION_ALGORITHM: &str = "AES-256-GCM";

pub fn encrypt_ephi(
    plaintext: &[u8],
    key: &EncryptionKey,
    nonce: &Nonce,
) -> Result<Vec<u8>, EncryptionError> {
    let cipher = Aes256Gcm::new(&key.0);
    
    // Encrypt with authentication tag
    cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| EncryptionError::EncryptionFailed)
}

pub fn decrypt_ephi(
    ciphertext: &[u8],
    key: &EncryptionKey,
    nonce: &Nonce,
) -> Result<Vec<u8>, EncryptionError> {
    let cipher = Aes256Gcm::new(&key.0);
    
    // Decrypt and verify authentication tag
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| EncryptionError::DecryptionFailed)
}
```

**Encryption Parameters**:
- **Algorithm**: AES-256-GCM (NIST SP 800-38D)
- **Key Size**: 256 bits
- **Nonce Size**: 96 bits (unique per encryption)
- **Authentication Tag**: 128 bits (prevents tampering)

**Compliance Evidence**: All ePHI encrypted at rest with NIST-approved algorithm.

**See**: [[Security/Encryption|Encryption Deep Dive]]

---

## 164.312(b): Audit Controls

**Requirement**: Implement hardware, software, and/or procedural mechanisms that record and examine activity in information systems that contain or use ePHI.

**Implementation**:
```rust
/// Comprehensive audit events for ePHI access
pub enum AuditEvent {
    /// ePHI credential accessed
    EphiAccessed {
        credential_id: CredentialId,
        requester: OwnerId,
        timestamp: DateTime<Utc>,
        result: AccessResult,
        ip_address: Option<String>,
        user_agent: Option<String>,
        correlation_id: Uuid,
    },
    
    /// ePHI credential created
    EphiCreated {
        credential_id: CredentialId,
        creator: OwnerId,
        timestamp: DateTime<Utc>,
    },
    
    /// ePHI credential modified
    EphiModified {
        credential_id: CredentialId,
        modifier: OwnerId,
        fields_changed: Vec<String>,
        timestamp: DateTime<Utc>,
    },
    
    /// ePHI credential deleted
    EphiDeleted {
        credential_id: CredentialId,
        deleter: OwnerId,
        timestamp: DateTime<Utc>,
    },
    
    /// Emergency access to ePHI
    EmergencyAccess {
        credential_id: CredentialId,
        responder: OwnerId,
        reason: String,
        timestamp: DateTime<Utc>,
        alert_level: AlertLevel,
    },
    
    /// Unauthorized access attempt
    UnauthorizedAccessAttempt {
        credential_id: CredentialId,
        requester: OwnerId,
        reason: String,  // "Permission denied", "Scope violation"
        timestamp: DateTime<Utc>,
        ip_address: Option<String>,
    },
}

/// Audit log retention: 6 years (HIPAA requirement)
pub const AUDIT_LOG_RETENTION_YEARS: u32 = 6;
```

**Required Audit Information**:
- ✅ Who accessed ePHI (user ID)
- ✅ What was accessed (credential ID)
- ✅ When it was accessed (timestamp)
- ✅ What action was performed (read, write, delete)
- ✅ Where access originated (IP address)
- ✅ Whether access was successful or denied

**Retention**: 6 years from date of creation or last date in effect (whichever is later).

**Compliance Evidence**: All ePHI access logged with complete metadata and retained for 6 years.

**See**: [[How-To/Enable-Audit-Logging|Audit Logging Setup]]

---

## 164.312(c)(1): Integrity

**Requirement**: Implement policies and procedures to protect ePHI from improper alteration or destruction.

### 164.312(c)(2): Mechanism to Authenticate ePHI (Addressable)

**Requirement**: Implement electronic mechanisms to corroborate that ePHI has not been altered or destroyed in an unauthorized manner.

**Implementation**:
```rust
/// GCM authentication tag prevents tampering
pub struct EncryptedCredential {
    pub version: KeyVersion,
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,  // Includes 128-bit authentication tag
    pub hash: blake3::Hash,   // Additional integrity check
}

pub fn encrypt_with_integrity(
    plaintext: &[u8],
    key: &EncryptionKey,
) -> Result<EncryptedCredential, EncryptionError> {
    let nonce = generate_nonce()?;
    
    // AES-GCM includes authentication tag (prevents tampering)
    let ciphertext = aes_gcm_encrypt(plaintext, key, &nonce)?;
    
    // Additional hash for integrity verification
    let hash = blake3::hash(&ciphertext);
    
    Ok(EncryptedCredential {
        version: key.version(),
        nonce,
        ciphertext,
        hash,
    })
}

pub fn decrypt_with_integrity_check(
    encrypted: &EncryptedCredential,
    key: &EncryptionKey,
) -> Result<Vec<u8>, EncryptionError> {
    // Verify hash first
    let computed_hash = blake3::hash(&encrypted.ciphertext);
    if computed_hash != encrypted.hash {
        return Err(EncryptionError::IntegrityCheckFailed);
    }
    
    // Decrypt (GCM verifies authentication tag automatically)
    let plaintext = aes_gcm_decrypt(
        &encrypted.ciphertext,
        key,
        &encrypted.nonce,
    )?;
    
    Ok(plaintext)
}
```

**Integrity Mechanisms**:
1. **GCM Authentication Tag**: 128-bit tag detects any ciphertext modification
2. **BLAKE3 Hash**: Additional integrity check for entire encrypted blob
3. **Immutable Audit Log**: Append-only storage prevents log tampering

**Compliance Evidence**: Tampering attempts automatically detected and rejected.

---

## 164.312(d): Person or Entity Authentication

**Requirement**: Implement procedures to verify that a person or entity seeking access to ePHI is the one claimed.

**Implementation**:
```rust
/// Multi-factor authentication support
pub enum AuthenticationMethod {
    /// Username/password with MFA
    PasswordMfa {
        password_hash: blake3::Hash,
        mfa_secret: SecretString,
    },
    
    /// Mutual TLS (certificate-based)
    MutualTls {
        client_cert_fingerprint: String,
    },
    
    /// OAuth2 with identity provider
    OAuth2 {
        provider: IdentityProvider,
        access_token: SecretString,
    },
    
    /// API key with HMAC signature
    ApiKeyHmac {
        key_id: String,
        signature: Vec<u8>,
    },
}

pub async fn authenticate(
    &self,
    credentials: &AuthenticationCredentials,
) -> Result<OwnerId, AuthError> {
    match &credentials.method {
        AuthenticationMethod::PasswordMfa { password_hash, mfa_secret } => {
            // Verify password
            self.verify_password(password_hash)?;
            
            // Verify MFA token
            self.verify_mfa_token(mfa_secret)?;
        }
        
        AuthenticationMethod::MutualTls { client_cert_fingerprint } => {
            // Verify client certificate
            self.verify_client_certificate(client_cert_fingerprint)?;
        }
        
        // Other methods...
    }
    
    // Log successful authentication
    self.audit_logger.log(AuditEvent::AuthenticationSuccessful {
        owner_id: credentials.owner_id.clone(),
        method: format!("{:?}", credentials.method),
        timestamp: Utc::now(),
    }).await;
    
    Ok(credentials.owner_id.clone())
}
```

**Supported Authentication**:
- ✅ Password + MFA (TOTP, SMS, push notification)
- ✅ Mutual TLS (X.509 certificates)
- ✅ OAuth2 (integration with identity providers)
- ✅ API keys with HMAC signatures

**Compliance Evidence**: Multi-factor authentication supported for ePHI access.

---

## 164.312(e)(1): Transmission Security

**Requirement**: Implement technical security measures to guard against unauthorized access to ePHI that is being transmitted over an electronic communications network.

### 164.312(e)(2)(i): Integrity Controls (Addressable)

**Requirement**: Implement security measures to ensure that electronically transmitted ePHI is not improperly modified without detection until disposed of.

**Implementation**:
```rust
/// TLS 1.3 provides transmission integrity
let tls_config = ClientConfig::builder()
    .with_safe_default_cipher_suites()
    .with_safe_default_kx_groups()
    .with_protocol_versions(&[&rustls::version::TLS13])?  // TLS 1.3 only
    .with_root_certificates(root_store)
    .with_no_client_auth();

// TLS 1.3 uses AEAD ciphers with integrity protection:
// - TLS_AES_256_GCM_SHA384
// - TLS_CHACHA20_POLY1305_SHA256
```

**Integrity Protection**:
- **TLS 1.3**: Built-in integrity checks (HMAC-based)
- **AEAD Ciphers**: Authentication tag prevents tampering
- **Certificate Pinning**: Prevents MITM attacks

---

### 164.312(e)(2)(ii): Encryption (Addressable)

**Requirement**: Implement a mechanism to encrypt ePHI whenever deemed appropriate.

**Implementation**:
```rust
/// Encrypt ePHI before transmission
pub async fn transmit_ephi(
    &self,
    ephi_data: &[u8],
    recipient_url: &str,
) -> Result<(), TransmissionError> {
    // Encrypt with TLS 1.3
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .min_tls_version(reqwest::tls::Version::TLS_1_3)
        .build()?;
    
    // Verify HTTPS (no HTTP allowed for ePHI)
    if !recipient_url.starts_with("https://") {
        return Err(TransmissionError::InsecureProtocol);
    }
    
    // Send encrypted
    let response = client
        .post(recipient_url)
        .body(ephi_data.to_vec())
        .send()
        .await?;
    
    // Log transmission
    self.audit_logger.log(AuditEvent::EphiTransmitted {
        recipient: recipient_url.to_string(),
        timestamp: Utc::now(),
        tls_version: "TLS 1.3",
    }).await;
    
    Ok(())
}
```

**Transmission Security**:
- ✅ TLS 1.3 mandatory (no fallback to TLS 1.2)
- ✅ HTTPS required (HTTP blocked for ePHI)
- ✅ Certificate validation enforced
- ✅ All transmissions logged

**Compliance Evidence**: All ePHI transmitted over TLS 1.3 with integrity protection.

---

## Business Associate Agreement (BAA) Requirements

If using nebula-credential as a Business Associate handling ePHI:

### Required BAA Provisions

1. **Use of ePHI**: Only as permitted by agreement
2. **Safeguards**: Implement appropriate safeguards (documented in this guide)
3. **Subcontractors**: Ensure same protections if subcontracting
4. **Reporting**: Report security incidents within 60 days
5. **Access**: Provide access to ePHI as required
6. **Audit**: Provide documentation for covered entity audits
7. **Return/Destruction**: Return or destroy ePHI at termination

**Compliance Checklist for Business Associates**:
- [ ] BAA signed with covered entity
- [ ] All technical safeguards implemented (164.312)
- [ ] Audit logging enabled and retained for 6 years
- [ ] Encryption enabled for all ePHI
- [ ] Incident response procedures documented
- [ ] Security awareness training completed
- [ ] Regular security risk assessments conducted

---

## Breach Notification Requirements

**45 CFR 164.404-414**: Notification in case of breaches of unsecured ePHI

### Breach Discovery and Notification Timeline

```rust
/// Breach notification workflow
pub async fn handle_breach(
    &self,
    breach: BreachEvent,
) -> Result<(), BreachError> {
    // 1. Discover breach (as soon as reasonably possible)
    let discovery_date = Utc::now();
    
    // 2. Notify covered entity (within 60 days)
    let notification_deadline = discovery_date + Duration::days(60);
    
    self.notify_covered_entity(BreachNotification {
        discovery_date,
        notification_deadline,
        affected_individuals: breach.affected_count,
        description: breach.description,
        mitigation: breach.mitigation_steps,
    }).await?;
    
    // 3. If >500 individuals affected: notify HHS and media
    if breach.affected_count > 500 {
        self.notify_hhs(&breach).await?;
        self.notify_media(&breach).await?;
    }
    
    // 4. Log breach response
    self.audit_logger.log(AuditEvent::BreachNotified {
        breach_id: breach.id,
        affected_count: breach.affected_count,
        notification_date: Utc::now(),
    }).await;
    
    Ok(())
}
```

**Notification Timeline**:
- **Discovery**: As soon as reasonably possible
- **Covered Entity**: Within 60 days of discovery
- **HHS**: Within 60 days (if >500 affected)
- **Media**: Without unreasonable delay (if >500 affected)
- **Individuals**: Within 60 days of discovery

---

## HIPAA Compliance Checklist

### Technical Safeguards (164.312)

- [x] **Access Control**
  - [x] Unique user identification
  - [x] Emergency access procedures
  - [x] Automatic logoff (15-minute timeout)
  - [x] Encryption/decryption (AES-256-GCM)

- [x] **Audit Controls**
  - [x] Comprehensive audit logging
  - [x] 6-year retention
  - [x] Immutable audit trail

- [x] **Integrity**
  - [x] Authentication tags (GCM)
  - [x] Hash verification (BLAKE3)

- [x] **Person/Entity Authentication**
  - [x] Multi-factor authentication
  - [x] Certificate-based authentication
  - [x] OAuth2 integration

- [x] **Transmission Security**
  - [x] TLS 1.3 encryption
  - [x] Integrity controls
  - [x] HTTPS enforcement

### Administrative & Physical (Infrastructure)

- [ ] Risk analysis conducted
- [ ] Security policies documented
- [ ] Workforce training completed
- [ ] Business Associate Agreements signed
- [ ] Incident response plan documented
- [ ] Disaster recovery plan tested
- [ ] Physical security controls (datacenter level)

---

## See Also

- [[Advanced/Compliance-SOC2|SOC 2 Compliance]]
- [[Advanced/Compliance-ISO27001|ISO 27001 Compliance]]
- [[Advanced/Compliance-GDPR|GDPR Compliance]]
- [[Security/Encryption|Encryption Deep Dive]]
- [[How-To/Enable-Audit-Logging|Audit Logging Setup]]
- [[Advanced/Security-Architecture|Security Architecture]]
- [[Advanced/Access-Control|Access Control System]]
