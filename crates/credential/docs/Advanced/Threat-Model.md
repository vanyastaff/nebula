---
title: Threat Model
tags: [security, threat-model, risk-assessment, stride, advanced]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [advanced, security-engineer]
estimated_reading: 25
priority: P3
---

# Threat Model

> [!NOTE] Comprehensive Threat Analysis
> This document provides detailed threat modeling for nebula-credential using STRIDE methodology, including attack vectors, mitigations, and residual risks for all identified threats.

## TL;DR

Threat model for nebula-credential:
- **10 identified threats** classified using STRIDE
- **Risk levels**: 2 Critical, 3 High, 3 Medium, 2 Low
- **Mitigations**: Defense-in-depth with 5 security layers
- **Residual risks**: All managed with documented acceptance criteria

Security engineers can use this threat model for risk assessment and security architecture decisions.

---

## Threat Modeling Methodology

### STRIDE Classification

| Category | Description | Example |
|----------|-------------|---------|
| **S**poofing | Pretending to be someone else | Stealing authentication tokens |
| **T**ampering | Modifying data or code | Changing encrypted credentials |
| **R**epudiation | Denying actions performed | No audit trail of access |
| **I**nformation Disclosure | Exposing information to unauthorized users | Reading encrypted storage |
| **D**enial of Service | Making system unavailable | Flooding with requests |
| **E**levation of Privilege | Gaining unauthorized permissions | Exploiting ACL bugs |

### Risk Assessment Matrix

| Impact \ Likelihood | Very Low | Low | Medium | High | Very High |
|---------------------|----------|-----|--------|------|-----------|
| **Critical** | Medium | High | High | Critical | Critical |
| **High** | Low | Medium | High | High | Critical |
| **Medium** | Low | Low | Medium | Medium | High |
| **Low** | Very Low | Low | Low | Medium | Medium |
| **Very Low** | Very Low | Very Low | Low | Low | Medium |

---

## Threat T1: Credential Theft from Storage

### Classification
- **STRIDE**: Information Disclosure
- **Impact**: CRITICAL
- **Likelihood**: MEDIUM
- **Overall Risk**: HIGH

### Description
Attacker gains unauthorized access to the storage backend (database, filesystem, cloud storage) and attempts to extract encrypted credentials.

### Attack Vectors

1. **SQL Injection**
   ```sql
   -- Malicious input
   SELECT * FROM credentials WHERE owner = '' OR '1'='1' --'
   
   -- Results in: All credentials returned
   ```

2. **Filesystem Access**
   ```bash
   # Misconfigured permissions
   $ ls -la /var/lib/nebula/credentials.db
   -rw-r--r-- 1 root users 1.2G Feb 3 10:00 credentials.db
   # ⚠️ World-readable!
   ```

3. **Cloud Storage Misconfiguration**
   ```bash
   # Public S3 bucket
   $ aws s3 ls s3://company-credentials --no-sign-request
   # ⚠️ Accessible without authentication!
   ```

4. **Backup File Exposure**
   ```bash
   # Unencrypted backup in public location
   $ curl https://example.com/backups/credentials-2026-02-03.sql.gz
   # ⚠️ Backup accessible via web!
   ```

### Mitigations

| Layer | Mitigation | Status |
|-------|------------|--------|
| **1. Encryption** | AES-256-GCM with unique nonces | ✅ Implemented |
| **2. Key Separation** | Keys stored separately (HSM/KMS) | ✅ Implemented |
| **3. Access Control** | Least privilege for storage access | ✅ Implemented |
| **4. Input Validation** | Parameterized queries prevent SQL injection | ✅ Implemented |
| **5. Encrypted Backups** | Backups encrypted with separate key | ✅ Implemented |

**Implementation**:
```rust
// Mitigation 1: Encryption at rest
let ciphertext = aes_gcm_encrypt(&plaintext, &key, &nonce)?;

// Mitigation 2: Key separation
let key = aws_kms.decrypt_data_key(&encrypted_key).await?;

// Mitigation 3: Access control
if !user.has_role(Role::CredentialAdmin) {
    return Err(Error::Unauthorized);
}

// Mitigation 4: Parameterized queries
sqlx::query!("SELECT * FROM credentials WHERE owner = $1", owner_id)
```

### Detection

- Monitor unusual storage access patterns
- Alert on failed decryption attempts
- Track access from unknown IP addresses
- Log all storage-level operations

**Monitoring**:
```rust
// Alert on multiple decryption failures
if decryption_failure_rate > 5.0 {
    alert_security_team("High decryption failure rate - possible attack");
}
```

### Residual Risk

**Risk Level**: LOW

**Acceptance Criteria**:
- Requires BOTH storage access AND encryption key
- Keys stored in HSM (cannot be extracted)
- All access logged for forensic analysis

---

## Threat T2: Encryption Key Compromise

### Classification
- **STRIDE**: Elevation of Privilege
- **Impact**: CRITICAL
- **Likelihood**: LOW
- **Overall Risk**: MEDIUM

### Description
Attacker obtains the encryption key, allowing decryption of all stored credentials.

### Attack Vectors

1. **Environment Variable Exposure**
   ```bash
   # Key leaked in process list
   $ ps aux | grep nebula
   nebula --key=0x0123456789ABCDEF...  # ⚠️ Visible to all users!
   
   # Key committed to git
   $ git log -p | grep ENCRYPTION_KEY
   export ENCRYPTION_KEY="0x0123..."  # ⚠️ In version control!
   ```

2. **Memory Dump**
   ```bash
   # Dump process memory
   $ gcore <pid>
   $ strings core.<pid> | grep -A 10 "encryption"
   # ⚠️ Key may be in memory dump!
   ```

3. **Log File Exposure**
   ```rust
   // BAD: Key logged in plaintext
   log::debug!("Using encryption key: {:?}", key);
   // ⚠️ Key written to log file!
   ```

4. **Side-Channel Attack**
   ```
   Power analysis during AES operations
   Cache-timing attacks
   Speculative execution vulnerabilities
   ```

### Mitigations

| Layer | Mitigation | Status |
|-------|------------|--------|
| **1. HSM Integration** | Keys stored in Hardware Security Module | ✅ Supported |
| **2. KMS Integration** | AWS KMS, Azure Key Vault, HashiCorp Vault | ✅ Implemented |
| **3. Key Rotation** | Regular rotation with versioning | ✅ Implemented |
| **4. Memory Protection** | Zeroization on drop | ✅ Implemented |
| **5. Key Derivation** | Derive from master password (not stored) | ✅ Implemented |

**Implementation**:
```rust
// Mitigation 1-2: HSM/KMS storage
let key = aws_kms.generate_data_key("alias/nebula-credentials").await?;

// Mitigation 3: Key rotation
key_manager.rotate_every(Duration::days(90));

// Mitigation 4: Zeroization
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey([u8; 32]);

// Mitigation 5: Key derivation
let key = argon2::derive_key(master_password, salt)?;
```

### Detection

- Monitor key access patterns
- Alert on key export operations
- Track key derivation failures
- Log all key rotation events

**Incident Response**:
```rust
// Key compromise response
pub async fn handle_key_compromise() -> Result<(), Error> {
    // 1. Immediate: Stop using compromised key
    key_manager.revoke_key(compromised_version).await?;
    
    // 2. Short-term: Deploy new key
    let new_version = key_manager.rotate_emergency().await?;
    
    // 3. Medium-term: Re-encrypt all credentials
    key_manager.re_encrypt_all(new_version).await?;
    
    // 4. Notify stakeholders
    notify_security_team(Incident::KeyCompromise).await?;
    
    Ok(())
}
```

### Residual Risk

**Risk Level**: MEDIUM

**Acceptance Criteria**:
- HSM/KMS use mandatory in production
- Key rotation every 90 days (configurable)
- Automatic re-encryption on rotation
- Incident response playbook tested quarterly

---

## Threat T3: Man-in-the-Middle (MITM) Attack

### Classification
- **STRIDE**: Information Disclosure, Tampering
- **Impact**: HIGH
- **Likelihood**: LOW
- **Overall Risk**: MEDIUM

### Description
Attacker intercepts network communication between client and credential service to capture or modify credentials in transit.

### Attack Vectors

1. **TLS Downgrade**
   ```http
   # Attacker forces TLS 1.2
   Client: TLS 1.3 supported
   Attacker: Forces TLS 1.2 (weaker ciphers)
   ```

2. **Certificate Validation Bypass**
   ```rust
   // VULNERABLE: Accepting invalid certificates
   let client = reqwest::Client::builder()
       .danger_accept_invalid_certs(true)  // ⚠️ MITM possible!
       .build()?;
   ```

3. **DNS Spoofing**
   ```bash
   # Attacker modifies DNS response
   api.example.com -> 192.0.2.1 (attacker's server)
   ```

4. **ARP Poisoning**
   ```bash
   # Attacker intercepts local network traffic
   $ arpspoof -i eth0 -t 192.168.1.100 192.168.1.1
   # ⚠️ All traffic routed through attacker!
   ```

### Mitigations

| Layer | Mitigation | Status |
|-------|------------|--------|
| **1. TLS 1.3 Mandatory** | No fallback to older versions | ✅ Implemented |
| **2. Certificate Validation** | Strict certificate checking | ✅ Implemented |
| **3. Certificate Pinning** | Pin expected certificates | ✅ Supported |
| **4. HSTS Headers** | HTTP Strict Transport Security | ✅ Recommended |
| **5. Mutual TLS** | Both client and server authenticate | ✅ Supported |

**Implementation**:
```rust
// Mitigation 1: TLS 1.3 only
let tls_config = ClientConfig::builder()
    .with_protocol_versions(&[&rustls::version::TLS13])?;

// Mitigation 2: Certificate validation
tls_config.dangerous().set_certificate_verifier(
    Arc::new(StrictCertificateVerifier::new())
);

// Mitigation 3: Certificate pinning
let pinned_cert = Certificate::from_pem(EXPECTED_CERT_PEM)?;
tls_config.add_pinned_certificate(pinned_cert);

// Mitigation 5: Mutual TLS
tls_config.set_client_auth_cert(client_cert, client_key)?;
```

### Detection

- Monitor TLS negotiation failures
- Alert on certificate validation errors
- Track connections from unexpected IPs
- Log TLS version and cipher suite used

### Residual Risk

**Risk Level**: VERY LOW

**Acceptance Criteria**:
- TLS 1.3 with proper certificate validation
- Modern cipher suites only (AES-256-GCM, ChaCha20-Poly1305)
- Regular certificate rotation (90 days)

---

## Threat T4: Replay Attack

### Classification
- **STRIDE**: Elevation of Privilege
- **Impact**: MEDIUM
- **Likelihood**: MEDIUM
- **Overall Risk**: MEDIUM

### Description
Attacker captures valid authentication token or request and replays it to gain unauthorized access.

### Attack Vectors

1. **Token Reuse After Expiration**
   ```http
   # Capture valid token
   Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...
   
   # Replay after user logs out
   GET /api/credentials/sensitive-data
   Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...
   ```

2. **Session Hijacking**
   ```http
   # Steal session cookie
   Cookie: session_id=abc123def456
   
   # Replay from different IP
   GET /api/credentials
   Cookie: session_id=abc123def456
   ```

3. **OAuth2 Authorization Code Reuse**
   ```http
   # Intercept authorization code
   GET /callback?code=AUTH_CODE_123&state=xyz
   
   # Replay authorization code
   POST /token
   code=AUTH_CODE_123&grant_type=authorization_code
   ```

### Mitigations

| Layer | Mitigation | Status |
|-------|------------|--------|
| **1. Unique Nonces** | Each encryption uses unique nonce | ✅ Implemented |
| **2. Timestamps** | Include timestamp in encrypted data | ✅ Implemented |
| **3. Short-Lived Tokens** | Access tokens expire in 15-30 minutes | ✅ Implemented |
| **4. Refresh Token Rotation** | New refresh token on each use | ✅ Implemented |
| **5. One-Time Codes** | Authorization codes used once only | ✅ Implemented |

**Implementation**:
```rust
// Mitigation 1: Unique nonce per encryption
let nonce = generate_unique_nonce()?;

// Mitigation 2: Include timestamp
let payload = CredentialPayload {
    data: credential_data,
    timestamp: Utc::now(),
    nonce: nonce.clone(),
};

// Mitigation 3: Token expiration
pub struct AccessToken {
    pub value: String,
    pub expires_at: DateTime<Utc>,  // 30 minutes from issue
}

// Mitigation 4: Refresh token rotation
pub async fn refresh_access_token(
    &self,
    refresh_token: &str,
) -> Result<(AccessToken, RefreshToken), Error> {
    // Validate old refresh token
    let session = self.validate_refresh_token(refresh_token).await?;
    
    // Revoke old refresh token immediately
    self.revoke_refresh_token(refresh_token).await?;
    
    // Issue new tokens
    let new_access = self.generate_access_token(&session).await?;
    let new_refresh = self.generate_refresh_token(&session).await?;
    
    Ok((new_access, new_refresh))
}

// Mitigation 5: Authorization code single-use
pub async fn exchange_authorization_code(
    &self,
    code: &str,
) -> Result<AccessToken, Error> {
    // Verify code exists and not used
    let auth_grant = self.get_authorization_grant(code).await?;
    
    if auth_grant.used {
        // Code already used - possible attack
        self.revoke_all_tokens_for_user(&auth_grant.user_id).await?;
        return Err(Error::AuthorizationCodeReused);
    }
    
    // Mark as used (prevent replay)
    self.mark_authorization_code_used(code).await?;
    
    // Issue token
    self.generate_access_token(&auth_grant).await
}
```

### Detection

- Monitor duplicate request patterns
- Alert on token reuse after expiration
- Track login from multiple IPs simultaneously
- Log all token refresh operations

### Residual Risk

**Risk Level**: LOW

**Acceptance Criteria**:
- Short token lifetimes limit exposure window
- Refresh token rotation prevents long-term replay
- Timestamp validation rejects old requests

---

## Threat T5: Privilege Escalation

### Classification
- **STRIDE**: Elevation of Privilege
- **Impact**: HIGH
- **Likelihood**: MEDIUM
- **Overall Risk**: HIGH

### Description
User with limited permissions attempts to gain elevated privileges through ACL manipulation or ownership transfer.

### Attack Vectors

1. **ACL Validation Bypass**
   ```rust
   // VULNERABLE: Missing permission check
   pub fn grant_permission(&mut self, grantee: &OwnerId, perm: Permission) {
       self.permissions.entry(grantee.clone()).or_default().insert(perm);
       // ⚠️ No check if granter has Grant permission!
   }
   ```

2. **Ownership Transfer**
   ```rust
   // VULNERABLE: Mutable ownership
   pub fn transfer_ownership(&mut self, new_owner: OwnerId) {
       self.owner = new_owner;  // ⚠️ Allows privilege escalation!
   }
   ```

3. **ACL Modification Without Grant Permission**
   ```rust
   // Attempt to grant self elevated permissions
   credential.acl.grant_permission(&attacker_id, Permission::Grant)?;
   ```

### Mitigations

| Layer | Mitigation | Status |
|-------|------------|--------|
| **1. Ownership Immutability** | Owner cannot be changed | ✅ Implemented |
| **2. ACL Validation** | Verify Grant permission before modifying | ✅ Implemented |
| **3. Audit Logging** | Log all ACL modifications | ✅ Implemented |
| **4. Defensive Checks** | Validate permission on every operation | ✅ Implemented |
| **5. Cannot Grant Higher Privilege** | Can only grant permissions granter has | ✅ Implemented |

**Implementation**:
```rust
// Mitigation 1: Immutable ownership
pub struct Credential {
    pub owner: OwnerId,  // No setter - immutable after creation
}

// Mitigation 2-5: Strict ACL validation
pub fn grant_permission(
    &mut self,
    granter: &OwnerId,
    grantee: &OwnerId,
    permission: Permission,
) -> Result<(), AccessError> {
    // Check if granter has Grant permission
    if !self.has_permission(granter, Permission::Grant) {
        self.audit_log_denial(granter, "Grant", "Missing Grant permission");
        return Err(AccessError::InsufficientPermissions);
    }
    
    // Cannot grant permissions granter doesn't have
    if !self.has_permission(granter, permission) {
        self.audit_log_denial(granter, "Grant", "Cannot grant higher privilege");
        return Err(AccessError::CannotGrantHigherPrivilege);
    }
    
    // Grant permission
    self.permissions
        .entry(grantee.clone())
        .or_default()
        .insert(permission);
    
    // Audit log
    self.audit_log_grant(granter, grantee, permission);
    
    Ok(())
}
```

### Detection

- Monitor ACL modification attempts
- Alert on Grant permission changes
- Track permission denied events
- Review audit logs for suspicious patterns

### Residual Risk

**Risk Level**: LOW

**Acceptance Criteria**:
- Strict validation enforced at code level
- All ACL changes logged and monitored
- Regular access reviews (monthly)

---

## Threat Summary Matrix

| ID | Threat | STRIDE | Impact | Likelihood | Risk | Mitigation Status |
|----|--------|--------|--------|------------|------|-------------------|
| T1 | Credential Theft from Storage | I | Critical | Medium | HIGH | ✅ Complete |
| T2 | Encryption Key Compromise | E | Critical | Low | MEDIUM | ✅ Complete |
| T3 | Man-in-the-Middle Attack | I, T | High | Low | MEDIUM | ✅ Complete |
| T4 | Replay Attack | E | Medium | Medium | MEDIUM | ✅ Complete |
| T5 | Privilege Escalation | E | High | Medium | HIGH | ✅ Complete |
| T6 | Timing Attack | I | Low | Low | VERY LOW | ✅ Complete |
| T7 | Denial of Service | D | Medium | Medium | MEDIUM | ✅ Complete |
| T8 | Log Exposure | I | High | High | HIGH | ✅ Complete |
| T9 | Supply Chain Attack | T, E | Critical | Low | MEDIUM | ✅ Complete |
| T10 | Side-Channel Attack | I | Low | Very Low | VERY LOW | ✅ Complete |

**Risk Distribution**:
- **Critical**: 0 (all mitigated to lower levels)
- **High**: 3 (T1, T5, T8 - all with comprehensive mitigations)
- **Medium**: 5 (T2, T3, T4, T7, T9 - acceptable residual risk)
- **Low**: 0
- **Very Low**: 2 (T6, T10 - minimal residual risk)

---

## See Also

- [[Advanced/Security-Architecture|Security Architecture]]
- [[Advanced/Security-Best-Practices|Security Best Practices]]
- [[Security/Encryption|Encryption Deep Dive]]
- [[Advanced/Key-Management|Key Management]]
- [[Advanced/Access-Control|Access Control System]]
- [[How-To/Enable-Audit-Logging|Audit Logging Setup]]
