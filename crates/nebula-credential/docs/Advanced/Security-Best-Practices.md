---
title: Security Best Practices
tags: [security, best-practices, secure-coding, penetration-testing, advanced]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [advanced, developer, security-engineer]
estimated_reading: 30
priority: P3
---

# Security Best Practices

> [!NOTE] Secure Coding Guidelines
> This document provides comprehensive security best practices for developing and deploying credential management systems with nebula-credential, including secure coding patterns, penetration testing scenarios, and common vulnerability prevention.

## TL;DR

Security best practices for nebula-credential:
- ✅ **Never hardcode credentials** (use environment variables or KMS)
- ✅ **Always use encryption** (AES-256-GCM at rest, TLS 1.3 in transit)
- ✅ **Implement least privilege** (minimal permissions by default)
- ✅ **Enable audit logging** (comprehensive logging with retention)
- ✅ **Regular security testing** (penetration testing, dependency scanning)
- ✅ **Incident response plan** (documented procedures and playbooks)

---

## Secure Coding Guidelines

### 1. Credential Handling

#### ✅ DO: Use SecretString for All Credentials

```rust
use secrecy::{Secret, ExposeSecret};

// GOOD: Credentials automatically redacted
let api_key = SecretString::new("sk_live_123456".to_string());
log::info!("API key loaded: {:?}", api_key);
// Output: "API key loaded: [REDACTED]"

// Access only when needed
let token = api_key.expose_secret();
make_api_call(token)?;
// Secret zeroized on drop
```

#### ❌ DON'T: Use Plain Strings

```rust
// BAD: Credential exposed in logs
let api_key = "sk_live_123456".to_string();
log::info!("API key loaded: {}", api_key);  // Leaks to logs!

// BAD: Credential may remain in memory
let password = user_input.clone();  // Not zeroized on drop
```

---

### 2. Encryption

#### ✅ DO: Generate Unique Nonces

```rust
use rand::RngCore;

// GOOD: Generate new nonce for EACH encryption
pub fn encrypt_credential(plaintext: &[u8], key: &EncryptionKey) -> Result<EncryptedData, Error> {
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);  // Random nonce
    
    let ciphertext = aes_gcm_encrypt(plaintext, key, &nonce)?;
    
    Ok(EncryptedData {
        nonce,
        ciphertext,
    })
}
```

#### ❌ DON'T: Reuse Nonces

```rust
// BAD: Reusing nonce completely breaks GCM security
const NONCE: [u8; 12] = [0; 12];  // NEVER DO THIS!

pub fn encrypt_credential(plaintext: &[u8], key: &EncryptionKey) -> Result<Vec<u8>, Error> {
    aes_gcm_encrypt(plaintext, key, &NONCE)  // CRITICAL VULNERABILITY!
}
```

**Why**: Reusing a nonce with the same key allows attackers to decrypt all messages encrypted with that nonce.

---

### 3. Key Management

#### ✅ DO: Store Keys in HSM/KMS

```rust
// GOOD: Use cloud KMS for production
use aws_sdk_kms::Client;

pub async fn get_encryption_key(kms: &Client) -> Result<EncryptionKey, Error> {
    let response = kms
        .generate_data_key()
        .key_id("alias/nebula-credentials")
        .key_spec(DataKeySpec::Aes256)
        .send()
        .await?;
    
    let key = EncryptionKey::from_bytes(
        response.plaintext().as_ref().try_into()?
    );
    
    Ok(key)
}
```

#### ❌ DON'T: Hardcode Keys

```rust
// BAD: Hardcoded key in source code
const ENCRYPTION_KEY: [u8; 32] = [
    0x00, 0x01, 0x02, // ... NEVER DO THIS!
];

// BAD: Key in environment variable (visible in process list)
let key = std::env::var("ENCRYPTION_KEY")?;  // Insecure!
```

---

### 4. Authentication & Authorization

#### ✅ DO: Always Validate Permissions

```rust
// GOOD: Check permission before every operation
pub async fn delete_credential(
    &self,
    id: &CredentialId,
    requester: &OwnerId,
) -> Result<(), CredentialError> {
    // Authenticate
    self.authenticate(requester).await?;
    
    // Authorize
    let credential = self.storage.get(id).await?;
    if !credential.acl.has_permission(requester, Permission::Delete) {
        return Err(CredentialError::PermissionDenied);
    }
    
    // Execute
    self.storage.delete(id).await?;
    
    // Audit
    self.audit_logger.log(AuditEvent::CredentialDeleted {
        credential_id: id.clone(),
        deleter: requester.clone(),
        timestamp: Utc::now(),
    }).await;
    
    Ok(())
}
```

#### ❌ DON'T: Skip Authorization Checks

```rust
// BAD: No permission check
pub async fn delete_credential(
    &self,
    id: &CredentialId,
) -> Result<(), CredentialError> {
    self.storage.delete(id).await  // Anyone can delete!
}
```

---

### 5. Input Validation

#### ✅ DO: Validate and Sanitize All Inputs

```rust
// GOOD: Validate credential ID format
pub fn validate_credential_id(id: &str) -> Result<CredentialId, ValidationError> {
    // Check format: cred_[a-zA-Z0-9]{16}
    let re = Regex::new(r"^cred_[a-zA-Z0-9]{16}$")?;
    if !re.is_match(id) {
        return Err(ValidationError::InvalidFormat);
    }
    
    // Check length
    if id.len() > 100 {
        return Err(ValidationError::TooLong);
    }
    
    Ok(CredentialId::from(id))
}

// GOOD: Sanitize user input
pub fn sanitize_search_query(query: &str) -> String {
    query
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .take(100)  // Limit length
        .collect()
}
```

#### ❌ DON'T: Trust User Input

```rust
// BAD: No validation (SQL injection risk)
pub async fn find_credential(&self, user_input: &str) -> Result<Credential, Error> {
    self.db.query(&format!("SELECT * FROM credentials WHERE name = '{}'", user_input)).await
    // If user_input = "'; DROP TABLE credentials; --", you're in trouble!
}
```

---

### 6. Error Handling

#### ✅ DO: Return Safe Error Messages

```rust
// GOOD: Generic error message (no sensitive data)
pub enum CredentialError {
    #[error("Decryption failed")]
    DecryptionFailed,  // Doesn't reveal why
    
    #[error("Access denied")]
    PermissionDenied,  // Doesn't reveal what exists
    
    #[error("Invalid request")]
    InvalidRequest,  // Doesn't reveal internal details
}
```

#### ❌ DON'T: Leak Information in Errors

```rust
// BAD: Error reveals sensitive information
pub enum CredentialError {
    #[error("Key version {0} not found in key store at /var/secrets/keys")]
    KeyNotFound(u32),  // Reveals internal paths!
    
    #[error("User {0} attempted to access credential {1} but has permission {2:?}")]
    PermissionDenied(OwnerId, CredentialId, Vec<Permission>),  // Leaks too much!
}
```

---

### 7. Logging

#### ✅ DO: Log Security Events (Without Secrets)

```rust
// GOOD: Log security events with redaction
pub async fn authenticate(
    &self,
    credentials: &AuthenticationCredentials,
) -> Result<OwnerId, AuthError> {
    match self.verify_credentials(credentials).await {
        Ok(owner_id) => {
            log::info!(
                "Authentication successful: user={}, method={:?}",
                owner_id,
                credentials.method  // Safe to log
            );
            Ok(owner_id)
        }
        Err(e) => {
            log::warn!(
                "Authentication failed: user={}, method={:?}, error={}",
                credentials.claimed_id,
                credentials.method,
                e  // Don't log the actual password/token!
            );
            Err(e)
        }
    }
}
```

#### ❌ DON'T: Log Sensitive Data

```rust
// BAD: Logs password in plaintext
pub async fn authenticate(&self, password: &str) -> Result<OwnerId, AuthError> {
    log::info!("Attempting authentication with password: {}", password);  // LEAKED!
    // ...
}
```

---

## Common Vulnerabilities

### 1. SQL Injection

**Vulnerability**:
```rust
// VULNERABLE: SQL injection
let query = format!(
    "SELECT * FROM credentials WHERE owner = '{}'",
    user_input  // If user_input = "' OR '1'='1", returns all credentials!
);
```

**Fix**:
```rust
// SAFE: Use parameterized queries
let credentials = sqlx::query_as!(
    Credential,
    "SELECT * FROM credentials WHERE owner = $1",
    owner_id  // Parameterized, not concatenated
)
.fetch_all(&self.pool)
.await?;
```

---

### 2. Command Injection

**Vulnerability**:
```rust
// VULNERABLE: Command injection
use std::process::Command;

let output = Command::new("sh")
    .arg("-c")
    .arg(format!("cat /var/credentials/{}", user_filename))  // Injection!
    .output()?;
// If user_filename = "; rm -rf /", you're in trouble!
```

**Fix**:
```rust
// SAFE: Use safe APIs, validate inputs
use std::fs;

let filename = validate_filename(user_filename)?;  // Allow only [a-zA-Z0-9_-]
let path = Path::new("/var/credentials").join(filename);

// Check path is within allowed directory
if !path.starts_with("/var/credentials") {
    return Err(Error::PathTraversal);
}

let contents = fs::read(&path)?;
```

---

### 3. Path Traversal

**Vulnerability**:
```rust
// VULNERABLE: Path traversal
let file_path = format!("/var/credentials/{}", user_input);
// If user_input = "../../../etc/passwd", reads arbitrary files!
```

**Fix**:
```rust
// SAFE: Canonicalize and validate path
use std::path::PathBuf;

let base_dir = PathBuf::from("/var/credentials");
let file_path = base_dir.join(user_input).canonicalize()?;

// Verify path is still within base directory
if !file_path.starts_with(&base_dir) {
    return Err(Error::PathTraversal);
}
```

---

### 4. Timing Attacks

**Vulnerability**:
```rust
// VULNERABLE: Timing attack (character-by-character comparison)
pub fn verify_api_key(provided: &str, stored: &str) -> bool {
    provided == stored  // Early return on first mismatch!
}
// Attacker can measure timing to guess characters one by one
```

**Fix**:
```rust
// SAFE: Constant-time comparison
use subtle::ConstantTimeEq;

pub fn verify_api_key(provided: &[u8], stored: &[u8]) -> bool {
    if provided.len() != stored.len() {
        // Still perform comparison to prevent length leakage
        let dummy = [0u8; 32];
        provided.ct_eq(&dummy[..provided.len()]);
        return false;
    }
    
    provided.ct_eq(stored).into()
}
```

---

### 5. Race Conditions

**Vulnerability**:
```rust
// VULNERABLE: Race condition (TOCTOU - Time Of Check, Time Of Use)
pub async fn update_credential(&self, id: &CredentialId, new_value: &[u8]) -> Result<(), Error> {
    // Check permission
    if self.has_permission(id, Permission::Write).await? {
        // ⚠️ Permission could change here!
        
        // Update (time gap allows race condition)
        self.storage.update(id, new_value).await?;
    }
    Ok(())
}
```

**Fix**:
```rust
// SAFE: Atomic check-and-update
pub async fn update_credential(&self, id: &CredentialId, new_value: &[u8]) -> Result<(), Error> {
    // Use database transaction or CAS operation
    self.storage.transaction(async |tx| {
        let credential = tx.get(id).await?;
        
        // Check permission within transaction
        if !credential.acl.has_permission(&requester, Permission::Write) {
            return Err(Error::PermissionDenied);
        }
        
        // Update atomically
        tx.update(id, new_value).await?;
        Ok(())
    }).await
}
```

---

## Penetration Testing Scenarios

### Scenario 1: Credential Theft via Storage Access

**Objective**: Verify that credentials cannot be read from storage without encryption keys.

**Test Steps**:
1. Gain access to storage backend (simulate SQL injection or file system access)
2. Attempt to read encrypted credentials directly
3. Attempt to decrypt without encryption key

**Expected Result**:
- ✅ Credentials are encrypted (AES-256-GCM)
- ✅ Decryption fails without key
- ✅ Attempt logged in audit trail

**Validation**:
```bash
# Read encrypted credential from storage
$ sqlite3 credentials.db "SELECT * FROM credentials WHERE id='cred_abc123';"
# id|owner|encrypted_value|nonce|key_version
# cred_abc123|user_xyz|<binary_data>|<binary_nonce>|v2

# Attempt to decrypt without key (should fail)
$ echo "<binary_data>" | openssl enc -d -aes-256-gcm -K <wrong_key> -iv <nonce>
# Error: bad decrypt
```

---

### Scenario 2: Privilege Escalation via ACL Manipulation

**Objective**: Verify that users cannot grant themselves elevated permissions.

**Test Steps**:
1. Authenticate as low-privilege user
2. Attempt to grant self `Grant` permission
3. Attempt to modify ACL without `Grant` permission
4. Attempt to change ownership

**Expected Result**:
- ✅ Grant operation denied (requires existing `Grant` permission)
- ✅ Ownership cannot be changed (immutable)
- ✅ All attempts logged as security violations

**Test Code**:
```rust
#[test]
async fn test_privilege_escalation_prevention() {
    let service = setup_test_service().await;
    
    // User without Grant permission
    let user = OwnerId::new("low_privilege_user");
    
    // Attempt to grant self elevated permissions
    let result = service.grant_permission(
        &credential_id,
        &user,  // granter (doesn't have Grant permission)
        &user,  // grantee (self)
        Permission::Grant,
    ).await;
    
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), CredentialError::InsufficientPermissions);
    
    // Verify audit log
    let events = service.audit_logger.get_recent_events().await;
    assert!(events.iter().any(|e| matches!(e, AuditEvent::PermissionDenied { .. })));
}
```

---

### Scenario 3: Replay Attack with Stolen Tokens

**Objective**: Verify that tokens cannot be reused after expiration.

**Test Steps**:
1. Obtain valid access token
2. Use token successfully
3. Wait for token expiration
4. Attempt to reuse expired token
5. Attempt to replay request with old timestamp

**Expected Result**:
- ✅ Expired token rejected
- ✅ Replayed request rejected (nonce/timestamp validation)
- ✅ Attempts logged

**Test Code**:
```rust
#[test]
async fn test_replay_attack_prevention() {
    let service = setup_test_service().await;
    
    // Get valid token
    let token = service.authenticate(&credentials).await.unwrap();
    
    // Use token successfully
    let result = service.access_with_token(&token).await;
    assert!(result.is_ok());
    
    // Fast-forward time beyond expiration
    advance_time(Duration::hours(2));
    
    // Attempt to reuse expired token
    let result = service.access_with_token(&token).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), CredentialError::TokenExpired);
}
```

---

### Scenario 4: Man-in-the-Middle (MITM) Attack

**Objective**: Verify that TLS prevents credential interception.

**Test Steps**:
1. Set up proxy to intercept traffic
2. Attempt to downgrade TLS version to 1.2
3. Attempt to use invalid certificate
4. Attempt to capture credentials in transit

**Expected Result**:
- ✅ TLS 1.3 enforced (no downgrade)
- ✅ Invalid certificate rejected
- ✅ Credentials encrypted in transit

**Validation**:
```bash
# Test TLS version enforcement
$ openssl s_client -connect api.example.com:443 -tls1_2
# Expected: Connection refused or downgrade not allowed

# Test certificate validation
$ curl --insecure https://api.example.com/credentials
# Expected: Error (insecure flag should not bypass validation in production)

# Verify TLS 1.3
$ openssl s_client -connect api.example.com:443 -tls1_3
# Expected: TLSv1.3 (cipher suite: TLS_AES_256_GCM_SHA384)
```

---

### Scenario 5: Timing Attack on Password Validation

**Objective**: Verify that password comparison is constant-time.

**Test Steps**:
1. Submit passwords with different lengths
2. Measure response time for each attempt
3. Analyze timing patterns to detect character-by-character comparison

**Expected Result**:
- ✅ Response time consistent regardless of password similarity
- ✅ No timing information leakage

**Test Code**:
```rust
#[test]
fn test_timing_attack_resistance() {
    let stored_password_hash = hash_password("correct_password");
    
    let test_cases = vec![
        "wrong_password",
        "correct_password",
        "c",  // Much shorter
        "correct_passwordXXXXXXXXXXXXX",  // Much longer
    ];
    
    let mut timings = Vec::new();
    
    for password in test_cases {
        let start = Instant::now();
        let _ = verify_password(password, &stored_password_hash);
        let duration = start.elapsed();
        timings.push(duration);
    }
    
    // Verify all timings are within 10% of each other
    let avg = timings.iter().sum::<Duration>() / timings.len() as u32;
    for timing in timings {
        let diff_percent = ((timing.as_nanos() as f64 - avg.as_nanos() as f64).abs() / avg.as_nanos() as f64) * 100.0;
        assert!(diff_percent < 10.0, "Timing variance too high: {}%", diff_percent);
    }
}
```

---

### Scenario 6: Denial of Service (DoS)

**Objective**: Verify that rate limiting prevents DoS attacks.

**Test Steps**:
1. Send 1000 requests in 1 second
2. Verify rate limiting kicks in
3. Verify legitimate users not affected

**Expected Result**:
- ✅ Requests rate-limited after threshold (100 req/min)
- ✅ Rate limiter returns 429 Too Many Requests
- ✅ Legitimate traffic still served

**Test Code**:
```rust
#[tokio::test]
async fn test_rate_limiting() {
    let service = setup_test_service().await;
    
    // Send 200 requests rapidly
    let mut tasks = vec![];
    for _ in 0..200 {
        let svc = service.clone();
        let credential_id = test_credential_id.clone();
        tasks.push(tokio::spawn(async move {
            svc.get_credential(&credential_id).await
        }));
    }
    
    let results = futures::future::join_all(tasks).await;
    
    // Count rate limit errors
    let rate_limited = results.iter()
        .filter(|r| matches!(r, Err(CredentialError::RateLimitExceeded)))
        .count();
    
    // At least 100 requests should be rate-limited
    assert!(rate_limited >= 100, "Rate limiting not effective: only {} requests limited", rate_limited);
}
```

---

### Scenario 7: SQL Injection

**Objective**: Verify that input validation prevents SQL injection.

**Test Steps**:
1. Inject SQL in credential name: `'; DROP TABLE credentials; --`
2. Inject SQL in search query: `' OR '1'='1`
3. Attempt to bypass authentication with SQL injection

**Expected Result**:
- ✅ All inputs sanitized or parameterized
- ✅ SQL injection attempts fail
- ✅ Database remains intact

**Test Code**:
```rust
#[tokio::test]
async fn test_sql_injection_prevention() {
    let service = setup_test_service().await;
    
    // Attempt SQL injection in credential name
    let malicious_input = "'; DROP TABLE credentials; --";
    
    let result = service.find_credentials_by_name(malicious_input).await;
    
    // Should either sanitize or return empty results, not execute SQL
    match result {
        Ok(credentials) => assert!(credentials.is_empty()),
        Err(e) => assert!(matches!(e, CredentialError::InvalidInput)),
    }
    
    // Verify table still exists
    let all_credentials = service.list_credentials().await;
    assert!(all_credentials.is_ok(), "Table was dropped!");
}
```

---

## Security Checklist

### Development Phase

- [ ] All credentials use `SecretString` wrapper
- [ ] All encryption uses unique nonces
- [ ] All keys stored in HSM/KMS (production)
- [ ] All user inputs validated and sanitized
- [ ] All database queries parameterized
- [ ] All error messages sanitized (no sensitive data)
- [ ] All security events logged
- [ ] All passwords hashed with Argon2id
- [ ] All comparisons use constant-time functions
- [ ] All dependencies scanned (`cargo audit`)

### Deployment Phase

- [ ] TLS 1.3 enforced
- [ ] Certificate validation enabled
- [ ] Rate limiting configured
- [ ] Audit logging enabled with 365-day retention
- [ ] Backups encrypted and tested
- [ ] Incident response plan documented
- [ ] Security monitoring configured
- [ ] Access control lists configured
- [ ] Key rotation schedule established
- [ ] Penetration testing completed

### Operational Phase

- [ ] Regular security audits (quarterly)
- [ ] Dependency updates (monthly)
- [ ] Log reviews (weekly)
- [ ] Access reviews (monthly)
- [ ] Backup tests (quarterly)
- [ ] Incident response drills (semi-annual)
- [ ] Penetration testing (annual)
- [ ] Compliance audits (annual)

---

## Security Tools

### Dependency Scanning

```bash
# Install cargo-audit
$ cargo install cargo-audit

# Scan dependencies for known vulnerabilities
$ cargo audit --deny warnings

# Update dependencies
$ cargo update

# Check for outdated dependencies
$ cargo outdated
```

### Static Analysis

```bash
# Run Clippy with security lints
$ cargo clippy -- \
    -D clippy::unwrap_used \
    -D clippy::expect_used \
    -D clippy::panic \
    -D clippy::todo \
    -D clippy::unimplemented

# Run rustfmt
$ cargo fmt --check
```

### Dynamic Analysis

```bash
# Run tests with address sanitizer
$ RUSTFLAGS="-Z sanitizer=address" cargo +nightly test

# Run tests with memory sanitizer
$ RUSTFLAGS="-Z sanitizer=memory" cargo +nightly test

# Run tests with thread sanitizer
$ RUSTFLAGS="-Z sanitizer=thread" cargo +nightly test
```

---

## See Also

- [[Advanced/Security-Architecture|Security Architecture]]
- [[Advanced/Threat-Model|Threat Model]]
- [[Security/Encryption|Encryption Deep Dive]]
- [[Advanced/Key-Management|Key Management]]
- [[Advanced/Access-Control|Access Control System]]
- [[How-To/Enable-Audit-Logging|Audit Logging Setup]]
- [[Troubleshooting/Common-Errors|Common Errors]]
