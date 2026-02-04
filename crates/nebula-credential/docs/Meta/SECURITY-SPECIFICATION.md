# Security Specification: nebula-credential

**Version:** 1.0.0  
**Status:** Draft  
**Last Updated:** 2026-02-03  
**Authors:** Security Architecture Team  

## Document Purpose

This document provides comprehensive security analysis and specifications for the nebula-credential crate, including:
- Threat model and attack surface analysis
- Security requirements and controls
- Cryptographic specifications
- Vulnerability mitigations
- Compliance mappings (SOC2, ISO 27001, HIPAA, GDPR)
- Security testing requirements
- Incident response procedures

This document ensures nebula-credential meets enterprise security standards and regulatory requirements.

---

## Table of Contents

1. [Threat Model](#threat-model)
2. [Attack Surface Analysis](#attack-surface-analysis)
3. [Security Requirements](#security-requirements)
4. [Cryptographic Security](#cryptographic-security)
5. [Authentication Protocol Security](#authentication-protocol-security)
6. [Storage Security](#storage-security)
7. [Network Security](#network-security)
8. [Access Control](#access-control)
9. [Audit & Compliance](#audit--compliance)
10. [Security Testing](#security-testing)
11. [Incident Response](#incident-response)
12. [Security Best Practices](#security-best-practices)

---

## 1. Threat Model

### 1.1 Assets

**Primary Assets:**
- Credentials (OAuth2 tokens, API keys, passwords, certificates, etc.)
- Encryption keys (master key, credential encryption keys)
- User data (usernames, email addresses, group memberships)

**Secondary Assets:**
- Credential metadata (creation time, expiration, usage statistics)
- Audit logs
- Configuration data

### 1.2 Threat Actors

| Actor Type | Capabilities | Motivation | Risk Level |
|------------|-------------|------------|------------|
| **External Attacker** | Network access, public exploits | Data theft, ransom, disruption | HIGH |
| **Malicious Insider** | System access, knowledge of architecture | Data exfiltration, sabotage | CRITICAL |
| **Compromised Service Account** | API access, limited privileges | Lateral movement, privilege escalation | HIGH |
| **Script Kiddie** | Automated tools, known vulnerabilities | Opportunistic attack | MEDIUM |
| **Nation State** | Advanced persistent threats, 0-days | Espionage, long-term access | CRITICAL |

### 1.3 Threat Scenarios

#### T1: Credential Theft via Storage Compromise

**Threat:** Attacker gains access to storage backend (database, cloud storage)

**Attack Path:**
1. Exploit SQL injection or cloud misconfiguration
2. Access encrypted credential data
3. Attempt to decrypt credentials

**Impact:** CRITICAL - Full credential compromise if encryption broken

**Mitigations:**
- **M1.1**: AES-256-GCM encryption with Argon2id key derivation
- **M1.2**: Encryption keys stored separately from encrypted data
- **M1.3**: Database-level encryption (encryption at rest)
- **M1.4**: Parameterized queries prevent SQL injection
- **M1.5**: Least privilege access to storage backend

**Residual Risk:** LOW (with proper key management)

---

#### T2: Encryption Key Compromise

**Threat:** Attacker obtains master encryption key

**Attack Path:**
1. Memory dump from running process
2. Steal key from key management system
3. Social engineering against admin

**Impact:** CRITICAL - Can decrypt all stored credentials

**Mitigations:**
- **M2.1**: Key zeroization on drop (Zeroize trait)
- **M2.2**: Memory encryption for sensitive data
- **M2.3**: Hardware Security Module (HSM) for key storage
- **M2.4**: Key rotation policy (90 days)
- **M2.5**: Multi-party key management (key splitting)

**Residual Risk:** MEDIUM (depends on key management practices)

---

#### T3: Man-in-the-Middle (MITM) Attack

**Threat:** Attacker intercepts network traffic during authentication

**Attack Path:**
1. Position on network path
2. Intercept OAuth2/SAML/LDAP traffic
3. Steal credentials or session tokens

**Impact:** HIGH - Session hijacking, credential theft

**Mitigations:**
- **M3.1**: TLS 1.3 for all network communications
- **M3.2**: Certificate pinning for known services
- **M3.3**: PKCE for OAuth2 (prevents authorization code interception)
- **M3.4**: SAML signature verification
- **M3.5**: LDAP over TLS (LDAPS)

**Residual Risk:** LOW (with proper TLS configuration)

---

#### T4: Credential Replay Attack

**Threat:** Attacker captures and replays valid credentials

**Attack Path:**
1. Intercept valid authentication token
2. Replay token to gain unauthorized access

**Impact:** HIGH - Unauthorized access to protected resources

**Mitigations:**
- **M4.1**: OAuth2 state parameter (CSRF protection)
- **M4.2**: Token expiration and refresh
- **M4.3**: Nonce in SAML assertions
- **M4.4**: JWT nbf (not before) and exp (expiration) claims
- **M4.5**: API key rotation

**Residual Risk:** LOW (with short-lived tokens)

---

#### T5: Privilege Escalation

**Threat:** Low-privilege user gains access to other users' credentials

**Attack Path:**
1. Exploit access control bypass
2. Query credentials without proper authorization
3. Access higher-privilege credentials

**Impact:** CRITICAL - Lateral movement, data exfiltration

**Mitigations:**
- **M5.1**: Owner-based access control (every credential has owner)
- **M5.2**: Scope isolation (workflow/node-specific credentials)
- **M5.3**: Access Control Lists (ACLs) with granular permissions
- **M5.4**: Authorization checks before every credential operation
- **M5.5**: Audit logging of all access attempts

**Residual Risk:** LOW (with proper ACL implementation)

---

#### T6: Timing Attack on Encryption

**Threat:** Attacker uses timing differences to extract encryption keys

**Attack Path:**
1. Measure decryption time for different inputs
2. Statistical analysis to infer key bits
3. Gradually reconstruct key

**Impact:** HIGH - Encryption key compromise

**Mitigations:**
- **M6.1**: Constant-time comparison operations
- **M6.2**: AES-GCM provides authenticated encryption (detects tampering)
- **M6.3**: Key derivation with constant-time Argon2
- **M6.4**: Random delays in authentication responses

**Residual Risk:** LOW (with constant-time implementations)

---

#### T7: Denial of Service (DoS)

**Threat:** Attacker overwhelms system to deny service

**Attack Path:**
1. Send massive number of credential test requests
2. Trigger expensive cryptographic operations
3. Exhaust system resources

**Impact:** MEDIUM - Service unavailability

**Mitigations:**
- **M7.1**: Rate limiting on credential operations
- **M7.2**: Circuit breaker pattern for external services
- **M7.3**: Connection pooling with max limits
- **M7.4**: Request timeout enforcement
- **M7.5**: Resource quotas per user

**Residual Risk:** MEDIUM (depends on infrastructure)

---

#### T8: Insecure Credential Storage in Logs

**Threat:** Credentials accidentally logged in plaintext

**Attack Path:**
1. Access application logs
2. Extract credentials from log entries
3. Use credentials for unauthorized access

**Impact:** CRITICAL - Credential exposure

**Mitigations:**
- **M8.1**: SecretString with Display impl showing "***"
- **M8.2**: Structured logging with sensitive field redaction
- **M8.3**: Debug trait shows redacted values
- **M8.4**: Explicit expose() method for auditable access
- **M8.5**: Log scrubbing pipeline

**Residual Risk:** LOW (with proper logging practices)

---

#### T9: Supply Chain Attack

**Threat:** Malicious dependency introduces vulnerability

**Attack Path:**
1. Compromise upstream crate
2. Inject backdoor or credential exfiltration code
3. Deploy to production

**Impact:** CRITICAL - Complete system compromise

**Mitigations:**
- **M9.1**: Dependency pinning with Cargo.lock
- **M9.2**: cargo-audit for vulnerability scanning
- **M9.3**: Code review of dependency updates
- **M9.4**: Minimal dependency tree
- **M9.5**: Reproducible builds

**Residual Risk:** MEDIUM (ongoing vigilance required)

---

#### T10: Side-Channel Attack via Cache Timing

**Threat:** Attacker infers credential data via CPU cache timing

**Attack Path:**
1. Co-locate on same physical host
2. Measure cache access patterns
3. Infer credential data from timing

**Impact:** MEDIUM - Partial credential leakage

**Mitigations:**
- **M10.1**: Cache-oblivious algorithms where possible
- **M10.2**: Memory scrubbing (zeroization)
- **M10.3**: Process isolation
- **M10.4**: Hardware isolation (separate VMs)

**Residual Risk:** MEDIUM (hardware-dependent)

---

### 1.4 STRIDE Threat Model

| Threat | Category | Description | Severity | Mitigations |
|--------|----------|-------------|----------|-------------|
| **Spoofing** | Identity | Attacker impersonates legitimate user | HIGH | OAuth2 PKCE, SAML signatures, mTLS client certs |
| **Tampering** | Integrity | Attacker modifies credentials in storage | CRITICAL | AES-GCM authenticated encryption, HMAC |
| **Repudiation** | Non-repudiation | User denies performing action | MEDIUM | Audit logging with timestamps and signatures |
| **Information Disclosure** | Confidentiality | Credentials leaked via logs/errors | CRITICAL | SecretString redaction, encrypted storage |
| **Denial of Service** | Availability | System overwhelmed by requests | MEDIUM | Rate limiting, circuit breakers, timeouts |
| **Elevation of Privilege** | Authorization | User gains unauthorized access | CRITICAL | ACLs, scope isolation, permission checks |

---

## 2. Attack Surface Analysis

### 2.1 Network Attack Surface

**Exposed Endpoints:**

| Endpoint | Protocol | Authentication | Encryption | Attack Vectors |
|----------|----------|----------------|------------|----------------|
| OAuth2 callback | HTTPS | State parameter | TLS 1.3 | CSRF, code interception |
| SAML ACS | HTTPS | Signature verification | TLS 1.3 | XML injection, signature forgery |
| LDAP server | LDAPS | Bind credentials | TLS 1.3 | MITM, credential stuffing |
| Storage backend | TCP/TLS | Service credentials | TLS 1.3 | SQL injection, unauthorized access |
| Cache (Redis) | TCP/TLS | Password | TLS 1.3 | Cache poisoning, data leakage |

**Mitigations:**
- All network traffic encrypted with TLS 1.3
- Certificate validation enforced
- Network segmentation (storage/cache on private network)
- Firewall rules restrict access

---

### 2.2 API Attack Surface

**Public APIs:**

| API | Input Validation | Rate Limiting | Authorization | Audit Logging |
|-----|------------------|---------------|---------------|---------------|
| `save_credential()` | Schema validation | 100/min | Owner check | Yes |
| `retrieve_credential()` | ID format validation | 1000/min | Owner check | Yes |
| `test_credential()` | Timeout enforcement | 10/min | Owner check | Yes |
| `rotate_credential()` | Policy validation | 10/hour | Owner check | Yes |
| `delete_credential()` | Cascade checks | 50/min | Owner check | Yes |

**Mitigations:**
- Input sanitization and validation
- Rate limiting per user/IP
- Authorization enforcement on every call
- Comprehensive audit logging

---

### 2.3 Storage Attack Surface

**Storage Layers:**

| Layer | Technology | Encryption | Access Control | Backup Security |
|-------|------------|------------|----------------|-----------------|
| Application | Rust | In-memory encryption | Process isolation | N/A |
| Cache | Redis | TLS, optional at-rest | Password auth | Encrypted snapshots |
| Database | SQLite/PostgreSQL | At-rest encryption | Role-based | Encrypted backups |
| Cloud Storage | AWS/Azure/Vault | Provider encryption | IAM policies | Versioning + encryption |

**Mitigations:**
- Defense in depth (multiple encryption layers)
- Least privilege access
- Regular backup testing
- Backup encryption separate from production keys

---

### 2.4 Memory Attack Surface

**In-Memory Data:**

| Data Type | Location | Protection | Lifetime | Risk |
|-----------|----------|------------|----------|------|
| Encryption keys | Heap | Zeroize on drop | Request duration | HIGH |
| Credentials | Heap | SecretString | Request duration | HIGH |
| Tokens | Heap | SecretString | Session duration | HIGH |
| Cache entries | Redis/memory | Encrypted | TTL (5 min) | MEDIUM |

**Mitigations:**
- Zeroize trait for automatic memory clearing
- Minimal credential lifetime in memory
- Secure heap allocator
- Memory encryption (if available)

---

## 3. Security Requirements

### 3.1 Confidentiality Requirements

| ID | Requirement | Priority | Implementation |
|----|-------------|----------|----------------|
| **SR-C-01** | All credentials MUST be encrypted at rest | P0 | AES-256-GCM |
| **SR-C-02** | All credentials MUST be encrypted in transit | P0 | TLS 1.3 |
| **SR-C-03** | Encryption keys MUST be protected from unauthorized access | P0 | HSM, key rotation |
| **SR-C-04** | Credentials MUST NOT appear in logs | P0 | SecretString redaction |
| **SR-C-05** | Memory containing secrets MUST be zeroed after use | P0 | Zeroize trait |
| **SR-C-06** | Debug output MUST NOT expose secrets | P0 | Custom Debug impl |
| **SR-C-07** | Error messages MUST NOT leak sensitive information | P1 | Generic error messages |

### 3.2 Integrity Requirements

| ID | Requirement | Priority | Implementation |
|----|-------------|----------|----------------|
| **SR-I-01** | Credential tampering MUST be detected | P0 | AES-GCM authentication tag |
| **SR-I-02** | SAML assertions MUST have valid signatures | P0 | XML signature verification |
| **SR-I-03** | OAuth2 state MUST prevent CSRF | P0 | Cryptographic state parameter |
| **SR-I-04** | JWT tokens MUST have valid signatures | P0 | HMAC/RSA signature |
| **SR-I-05** | Database transactions MUST be atomic | P1 | ACID compliance |
| **SR-I-06** | Audit logs MUST be immutable | P1 | Append-only storage |

### 3.3 Availability Requirements

| ID | Requirement | Priority | Implementation |
|----|-------------|----------|----------------|
| **SR-A-01** | System MUST handle 1000 req/sec | P1 | Connection pooling, caching |
| **SR-A-02** | Credential operations MUST timeout | P0 | 30-second timeout |
| **SR-A-03** | System MUST survive single component failure | P1 | Redundancy, circuit breakers |
| **SR-A-04** | Rate limiting MUST prevent DoS | P0 | Token bucket algorithm |
| **SR-A-05** | Backups MUST be restorable within 1 hour | P1 | Automated backup testing |

### 3.4 Authentication Requirements

| ID | Requirement | Priority | Implementation |
|----|-------------|----------|----------------|
| **SR-AU-01** | OAuth2 MUST use PKCE | P0 | SHA-256 code challenge |
| **SR-AU-02** | SAML MUST verify signatures | P0 | RSA/DSA signature check |
| **SR-AU-03** | LDAP MUST use secure bind | P0 | LDAPS with TLS 1.3 |
| **SR-AU-04** | mTLS MUST validate certificate chains | P0 | X.509 chain verification |
| **SR-AU-05** | JWT MUST validate exp/nbf claims | P0 | Timestamp validation |
| **SR-AU-06** | API keys MUST be hashed before storage | P0 | BLAKE3 hashing |

### 3.5 Authorization Requirements

| ID | Requirement | Priority | Implementation |
|----|-------------|----------|----------------|
| **SR-AZ-01** | Credentials MUST have an owner | P0 | OwnerId required field |
| **SR-AZ-02** | Access MUST be verified before operations | P0 | Authorization middleware |
| **SR-AZ-03** | Scope isolation MUST be enforced | P0 | ScopeId filtering |
| **SR-AZ-04** | ACLs MUST support granular permissions | P1 | PermissionSet with 6 permissions |
| **SR-AZ-05** | Privilege escalation MUST be prevented | P0 | Permission checks at every layer |

### 3.6 Audit Requirements

| ID | Requirement | Priority | Implementation |
|----|-------------|----------|----------------|
| **SR-L-01** | All credential access MUST be logged | P0 | Structured audit logs |
| **SR-L-02** | Authentication attempts MUST be logged | P0 | Success/failure with reason |
| **SR-L-03** | Logs MUST include timestamp and user | P0 | ISO 8601 timestamps |
| **SR-L-04** | Logs MUST be retained for 90 days | P1 | Log rotation policy |
| **SR-L-05** | Failed auth attempts MUST trigger alerts | P1 | Threshold-based alerting |

---

## 4. Cryptographic Security

### 4.1 Encryption Algorithms

**Symmetric Encryption:**

```
Algorithm: AES-256-GCM
Key Size: 256 bits
Nonce Size: 96 bits (12 bytes)
Tag Size: 128 bits (16 bytes)
Mode: Galois/Counter Mode (GCM)

Rationale:
- NIST approved (FIPS 140-2)
- Authenticated encryption (AEAD)
- Hardware acceleration (AES-NI)
- Resistant to timing attacks
```

**Key Derivation:**

```
Algorithm: Argon2id
Memory Cost: 19 MiB (19456 KiB)
Time Cost: 2 iterations
Parallelism: 1 thread
Salt Size: 128 bits (16 bytes)
Output: 256 bits (32 bytes)

Rationale:
- OWASP recommendation 2024
- Resistant to GPU attacks
- Memory-hard function
- Side-channel resistant
```

**Hashing (API Keys):**

```
Algorithm: BLAKE3
Output: 256 bits (32 bytes)

Rationale:
- Faster than SHA-256
- Cryptographically secure
- Parallelizable
- No known vulnerabilities
```

### 4.2 Nonce Generation

**Requirements:**
- MUST be unique for each encryption operation
- MUST be unpredictable
- MUST NOT repeat for same key

**Implementation:**

```rust
Nonce Format: [4-byte random prefix | 8-byte counter]

Properties:
- Random prefix provides 2^32 unique sequences
- Monotonic counter prevents reuse within sequence
- Total nonce space: 2^32 * 2^64 = 2^96 (meets GCM requirements)
```

**Security Analysis:**
- Probability of nonce collision: < 2^-32 for same prefix
- Expected collisions after 2^32 encryptions: 0
- Nonce exhaustion after: 2^96 operations (practically infinite)

### 4.3 Key Management

**Master Key:**

```
Generation: Derived from admin password + salt
Storage: Hardware Security Module (HSM) or Azure Key Vault
Rotation: Every 90 days
Backup: Encrypted with separate key, stored offline
```

**Credential Encryption Keys:**

```
Generation: Random 256-bit key per credential
Encryption: Encrypted with master key
Storage: Alongside encrypted credential
Rotation: On master key rotation
```

**Key Hierarchy:**

```
Root Key (HSM)
  └─> Master Key (derived from password)
       └─> Credential Key 1
       └─> Credential Key 2
       └─> Credential Key N
```

### 4.4 Cryptographic Security Levels

| Operation | Algorithm | Key Size | Security Level | Quantum Resistant |
|-----------|-----------|----------|----------------|-------------------|
| Symmetric encryption | AES-256-GCM | 256 bits | 128-bit | No |
| Key derivation | Argon2id | 256 bits | 128-bit | No |
| Hashing | BLAKE3 | 256 bits | 128-bit | Yes |
| Digital signature | RSA-2048 | 2048 bits | 112-bit | No |
| Digital signature | Ed25519 | 256 bits | 128-bit | No |

**Post-Quantum Readiness:**
- Current algorithms provide 128-bit security level
- Migration path to post-quantum algorithms (e.g., Kyber, Dilithium)
- Algorithm versioning supports migration

---

## 5. Authentication Protocol Security

### 5.1 OAuth 2.0 Security

**PKCE (Proof Key for Code Exchange):**

```
Requirement: MANDATORY for all authorization code flows
Algorithm: SHA-256 (S256 method)
Verifier: 32 bytes random (256 bits)
Challenge: BASE64URL(SHA256(verifier))

Security Properties:
- Prevents authorization code interception
- Protects against malicious apps on same device
- Required by OAuth 2.1 specification
```

**State Parameter:**

```
Requirement: MANDATORY for CSRF protection
Generation: 32 bytes random (256 bits)
Validation: Exact match on callback
Lifetime: Single-use, expires after 10 minutes

Security Properties:
- Prevents cross-site request forgery
- Binds authorization request to callback
- Prevents session fixation attacks
```

**Token Storage:**

```
Access Token: In-memory only, never persisted
Refresh Token: Encrypted at rest
Token Type: Bearer (validated on use)
Expiration: Enforced strictly, no grace period
```

**Vulnerabilities Mitigated:**

| Vulnerability | Mitigation | RFC Reference |
|---------------|------------|---------------|
| Authorization code interception | PKCE mandatory | RFC 7636 |
| CSRF attacks | State parameter | RFC 6749 §10.12 |
| Token theft | TLS + short expiration | RFC 6749 §10.4 |
| Token replay | Nonce + expiration | RFC 6749 §10.5 |
| Open redirect | Strict redirect URI validation | RFC 6749 §10.6 |

### 5.2 SAML 2.0 Security

**Signature Verification:**

```
Algorithm: RSA-SHA256 or ECDSA-SHA256
Key Length: RSA 2048-bit minimum
Canonicalization: Exclusive XML C14N
Validation: Signature + certificate chain

Security Properties:
- Prevents assertion tampering
- Authenticates identity provider
- Ensures message integrity
```

**Assertion Validation:**

```
Required Checks:
1. Signature valid and trusted
2. NotBefore ≤ CurrentTime < NotOnOrAfter
3. Recipient matches ACS URL
4. Audience matches SP entity ID
5. InResponseTo matches request ID (if present)
6. AssertionConsumerServiceURL matches expected
```

**XML Security:**

```
XML Bomb Protection: Limit entity expansion
XML Injection: Schema validation
XXE (External Entity): Disable external entities
XPath Injection: Parameterized queries
```

**Vulnerabilities Mitigated:**

| Vulnerability | Mitigation | Reference |
|---------------|------------|-----------|
| Signature wrapping | Strict XML parsing | SAML-SEC-1.0 |
| XML bomb | Entity expansion limits | CWE-776 |
| XXE injection | Disable external entities | CWE-611 |
| Replay attacks | NotOnOrAfter validation | SAML 2.0 Core §2.5 |
| Man-in-the-middle | TLS + signature | SAML 2.0 Binding §3 |

### 5.3 LDAP Security

**Connection Security:**

```
Protocol: LDAPS (LDAP over TLS)
TLS Version: 1.3 minimum
Certificate Validation: Required
Cipher Suites: Strong only (AES-GCM, ChaCha20)
```

**Bind Security:**

```
Bind Type: Simple bind over TLS
Credentials: Never cached, used once
DN Construction: Parameterized (no injection)
Failed Attempts: Rate limited
```

**Search Security:**

```
Filter Escaping: RFC 4515 compliant
  ( → \28
  ) → \29
  * → \2a
  \ → \5c
  NUL → \00

Base DN Validation: Whitelist allowed DNs
Scope: Minimum required (Base, OneLevel, Subtree)
Attributes: Only request needed attributes
```

**Vulnerabilities Mitigated:**

| Vulnerability | Mitigation | Reference |
|---------------|------------|-----------|
| LDAP injection | Filter escaping | CWE-90 |
| Credential theft | TLS encryption | RFC 4513 |
| Enumeration attacks | Rate limiting | OWASP |
| Anonymous bind | Require authentication | RFC 4513 §5.1 |
| Cleartext passwords | LDAPS only | RFC 4513 §6.3 |

### 5.4 mTLS Security

**Certificate Validation:**

```
Required Checks:
1. Certificate not expired (NotBefore ≤ Now < NotAfter)
2. Certificate chain valid to trusted CA
3. Certificate not revoked (OCSP/CRL)
4. Subject matches expected
5. Key usage includes digitalSignature/keyEncipherment
6. Extended key usage includes clientAuth
```

**Certificate Storage:**

```
Private Key: PEM format, encrypted at rest
Public Certificate: PEM format, plaintext OK
CA Certificate: Trusted root store
Storage: Filesystem with 0600 permissions
```

**TLS Configuration:**

```
TLS Version: 1.3 only (1.2 fallback if required)
Cipher Suites:
  - TLS_AES_256_GCM_SHA384
  - TLS_AES_128_GCM_SHA256
  - TLS_CHACHA20_POLY1305_SHA256

Hostname Verification: Required
Certificate Pinning: Optional (for known services)
```

**Vulnerabilities Mitigated:**

| Vulnerability | Mitigation | Reference |
|---------------|------------|-----------|
| MITM attacks | Certificate validation | RFC 5280 |
| Downgrade attacks | TLS 1.3 only | RFC 8446 |
| Weak ciphers | Strong cipher suites | Mozilla SSL Config |
| Certificate spoofing | Chain validation | RFC 5280 §6 |
| Revoked certificates | OCSP/CRL checking | RFC 6960 |

### 5.5 JWT Security

**Signature Algorithms:**

```
Recommended:
- HS256 (HMAC-SHA256) for symmetric keys
- RS256 (RSA-SHA256) for asymmetric keys
- ES256 (ECDSA-SHA256) for performance

Forbidden:
- none (no signature)
- HS256 with public key as secret
```

**Claims Validation:**

```
Required Claims:
- exp (expiration): MUST be validated
- nbf (not before): MUST be validated
- iat (issued at): SHOULD be validated
- iss (issuer): SHOULD be validated
- aud (audience): SHOULD be validated

Clock Skew: Allow ±60 seconds tolerance
```

**Key Management:**

```
Symmetric Keys (HS256):
  - 256-bit minimum
  - Rotated every 90 days
  - Never exposed in logs

Asymmetric Keys (RS256):
  - 2048-bit minimum
  - Private key encrypted at rest
  - Public key in JWKS endpoint
```

**Vulnerabilities Mitigated:**

| Vulnerability | Mitigation | Reference |
|---------------|------------|-----------|
| Algorithm confusion | Strict algorithm validation | RFC 8725 §2.1 |
| Weak signatures | Strong algorithms only | RFC 8725 §3.1 |
| Key confusion | Separate keys per use | RFC 8725 §2.2 |
| Token substitution | aud/iss validation | RFC 8725 §3.2 |
| Timing attacks | Constant-time validation | RFC 8725 §2.8 |

### 5.6 API Key Security

**Generation:**

```
Format: prefix_random
  - Prefix: "sk" (2 chars)
  - Separator: "_" (1 char)
  - Random: 32 bytes base64url-encoded (43 chars)
  - Total: 46 characters

Example: sk_xyzabc123...

Entropy: 256 bits
Uniqueness: Cryptographic RNG
```

**Storage:**

```
Storage Format: BLAKE3 hash (never plaintext)
Hash Size: 256 bits (32 bytes)
Salt: Not required (key has 256-bit entropy)

Database Schema:
- key_hash: CHAR(64) -- hex-encoded BLAKE3 hash
- key_id: UUID -- unique identifier
- owner_id: VARCHAR -- credential owner
```

**Validation:**

```
Process:
1. Hash provided key with BLAKE3
2. Constant-time compare with stored hash
3. Check expiration timestamp
4. Check rate limit
5. Update last_used timestamp
```

**Rotation:**

```
Grace Period: 7 days (configurable)
Process:
1. Generate new key
2. Return both old and new keys to user
3. Old key remains valid during grace period
4. After grace period, old key is revoked
5. Audit log records rotation
```

**Vulnerabilities Mitigated:**

| Vulnerability | Mitigation | Reference |
|---------------|------------|-----------|
| Key theft | Hashing (not reversible) | OWASP ASVS 2.7.1 |
| Timing attacks | Constant-time comparison | CWE-208 |
| Brute force | High entropy (256 bits) | OWASP ASVS 2.9.1 |
| Key enumeration | Rate limiting | OWASP ASVS 4.2.1 |
| Accidental exposure | Prefix for detection | GitHub Secret Scanning |

---

## 6. Storage Security

### 6.1 Encryption at Rest

**Application-Level Encryption:**

```
Layer: Before database write
Algorithm: AES-256-GCM
Key: Unique per credential, encrypted with master key
Nonce: Unique per encryption operation

Benefits:
- Database compromise doesn't expose credentials
- Fine-grained key management
- Auditable encryption operations
```

**Database-Level Encryption:**

```
SQLite: PRAGMA key='...' (SQLCipher)
PostgreSQL: Transparent Data Encryption (TDE)
AWS RDS: Encryption at rest with KMS
Azure SQL: Transparent Data Encryption

Benefits:
- Protection against storage media theft
- Compliance requirement fulfillment
- Defense in depth
```

**Storage Provider Comparison:**

| Provider | Application Encryption | Provider Encryption | Key Management | Risk Level |
|----------|------------------------|---------------------|----------------|------------|
| Local (SQLite) | AES-256-GCM | SQLCipher | Local file | MEDIUM |
| AWS Secrets Manager | AES-256-GCM | AWS KMS | AWS KMS | LOW |
| HashiCorp Vault | AES-256-GCM | Vault transit | Vault | LOW |
| Azure Key Vault | AES-256-GCM | Azure Storage | Azure KMS | LOW |

### 6.2 Database Security

**Access Control:**

```
Principle: Least Privilege
Implementation:
- Dedicated database user for credential service
- No SUPERUSER or admin privileges
- Grant only required operations:
  - SELECT on credentials table
  - INSERT on credentials table
  - UPDATE on credentials table (metadata only)
  - DELETE on credentials table

Forbidden:
- DDL operations (DROP, ALTER, CREATE)
- Database administration commands
- Access to other schemas/databases
```

**SQL Injection Prevention:**

```
Method: Parameterized Queries
Example:
  ✗ BAD:  "SELECT * FROM credentials WHERE id = '" + id + "'"
  ✓ GOOD: sqlx::query!("SELECT * FROM credentials WHERE id = ?", id)

Tools:
- sqlx with compile-time query validation
- No dynamic SQL construction
- Input validation before database queries
```

**Connection Security:**

```
Local SQLite:
- File permissions: 0600 (owner read/write only)
- Directory permissions: 0700
- No network exposure

Remote Database:
- TLS 1.3 for connections
- Certificate validation
- Private network (no public internet)
- VPN or AWS PrivateLink
```

**Backup Security:**

```
Backup Encryption: Separate key from production
Backup Storage: Offline or separate cloud account
Backup Frequency: Daily incremental, weekly full
Backup Retention: 30 days (compliance requirement)
Backup Testing: Monthly restore drills
```

### 6.3 Cache Security

**Redis Security:**

```
Authentication: requirepass (strong password)
Encryption: TLS for connections
Network: Bind to localhost or private network
Commands: Disable dangerous commands (FLUSHALL, CONFIG)
Persistence: RDB encryption, AOF encryption
```

**Cache Poisoning Prevention:**

```
Key Namespace: Prefix with "nebula:credential:"
TTL: Short expiration (5 minutes)
Validation: Re-validate from storage periodically
Invalidation: On credential update/delete
Signing: HMAC signature on cache entries
```

**Memory Cache Security:**

```
Isolation: Process memory isolation
Zeroization: Clear memory on eviction
Size Limit: Prevent memory exhaustion
Eviction Policy: LRU with TTL
```

---

## 7. Network Security

### 7.1 TLS Configuration

**TLS Version:**

```
Required: TLS 1.3
Acceptable: TLS 1.2 (only if 1.3 not available)
Forbidden: TLS 1.0, TLS 1.1, SSLv3

Rationale:
- TLS 1.3 removes weak algorithms
- Faster handshake (0-RTT)
- Forward secrecy by default
```

**Cipher Suites (TLS 1.3):**

```
Recommended:
1. TLS_AES_256_GCM_SHA384
2. TLS_CHACHA20_POLY1305_SHA256
3. TLS_AES_128_GCM_SHA256

Properties:
- All provide forward secrecy
- All provide authenticated encryption
- Hardware acceleration available
```

**Certificate Validation:**

```
Requirements:
1. Valid signature chain to trusted CA
2. Certificate not expired
3. Hostname matches Subject Alternative Name (SAN)
4. Certificate not revoked (OCSP/CRL)
5. Key usage appropriate for purpose
```

### 7.2 Network Segmentation

**Network Architecture:**

```
┌─────────────────┐
│   Internet      │
└────────┬────────┘
         │ (TLS)
┌────────▼────────┐
│  Load Balancer  │
└────────┬────────┘
         │ (TLS)
┌────────▼────────┐
│ Application     │ ◄─► Cache (Redis)
│ (nebula)        │     Private Network
└────────┬────────┘
         │ (TLS)
┌────────▼────────┐
│ Database        │
│ (Private)       │
└─────────────────┘
```

**Network Rules:**

| Source | Destination | Protocol | Port | Purpose |
|--------|-------------|----------|------|---------|
| Internet | Load Balancer | HTTPS | 443 | API access |
| Load Balancer | Application | HTTPS | 8443 | Backend |
| Application | Database | PostgreSQL/TLS | 5432 | Storage |
| Application | Redis | Redis/TLS | 6379 | Cache |
| Application | OAuth Provider | HTTPS | 443 | Auth |
| Application | LDAP | LDAPS | 636 | Directory |

**Firewall Rules:**

```
Default Policy: DENY ALL

Allowed Inbound:
- TCP 443 from Load Balancer (HTTPS)

Allowed Outbound:
- TCP 5432 to Database (PostgreSQL)
- TCP 6379 to Redis (Cache)
- TCP 443 to Internet (OAuth, SAML, APIs)
- TCP 636 to LDAP (Directory)
- UDP 53 to DNS (Name resolution)
```

### 7.3 API Security

**Rate Limiting:**

```
Endpoint-Specific Limits:
- save_credential(): 100 requests/min per user
- retrieve_credential(): 1000 requests/min per user
- test_credential(): 10 requests/min per user
- rotate_credential(): 10 requests/hour per user
- list_credentials(): 100 requests/min per user

Algorithm: Token bucket with Redis backend
Response: HTTP 429 with Retry-After header
```

**Request Validation:**

```
Content-Type: application/json (enforced)
Content-Length: Max 10 MB
Request Timeout: 30 seconds
Body Parsing: Strict JSON schema validation
Input Sanitization: Escape special characters
```

**CORS Policy:**

```
Allowed Origins: Whitelist only
Allowed Methods: GET, POST, PUT, DELETE
Allowed Headers: Authorization, Content-Type
Credentials: true (cookies allowed)
Max Age: 3600 seconds
```

---

## 8. Access Control

### 8.1 Authorization Model

**Ownership Model:**

```
Every credential has:
- owner_id: User/service that created credential
- scope_id: Optional resource isolation (workflow/node)

Authorization Rules:
1. Owner has full access (read, write, delete, rotate, test, share)
2. Non-owner requires explicit ACL grant
3. System admin can access all (with audit log)
```

**Permission Types:**

| Permission | Description | Typical Use Case |
|------------|-------------|------------------|
| `read` | View credential metadata | Dashboard display |
| `write` | Update credential | Token refresh |
| `delete` | Remove credential | Cleanup |
| `rotate` | Rotate credential | Security policy |
| `test` | Test credential validity | Health check |
| `share` | Grant access to others | Team collaboration |

**Access Control List (ACL):**

```rust
struct AccessControlEntry {
    principal_id: String,      // User/service ID
    principal_type: PrincipalType, // User/Group/Service
    permissions: PermissionSet,
    granted_at: DateTime<Utc>,
    granted_by: String,        // Who granted access
}

Example:
{
  "principal_id": "service-123",
  "principal_type": "Service",
  "permissions": {
    "read": true,
    "write": false,
    "delete": false,
    "rotate": false,
    "test": true,
    "share": false
  },
  "granted_at": "2026-02-03T10:30:00Z",
  "granted_by": "admin-user"
}
```

### 8.2 Scope Isolation

**Scope Types:**

```
1. Global Scope: No isolation, accessible by owner anywhere
   scope_id: None

2. Workflow Scope: Accessible only within specific workflow
   scope_id: "workflow:workflow-uuid"

3. Node Scope: Accessible only within specific node
   scope_id: "node:node-uuid"

Example Use Cases:
- OAuth token for Telegram node: scope="node:telegram-123"
- Database password for workflow: scope="workflow:etl-pipeline"
- Admin API key: scope=None (global)
```

**Isolation Enforcement:**

```rust
fn check_scope_access(
    credential: &Credential,
    request_scope: Option<&ScopeId>,
) -> bool {
    match (&credential.scope_id, request_scope) {
        (None, _) => true,  // Global credential, always accessible
        (Some(cred_scope), Some(req_scope)) => cred_scope == req_scope,
        (Some(_), None) => false,  // Scoped credential, no scope provided
    }
}
```

### 8.3 Privilege Escalation Prevention

**Defense Mechanisms:**

1. **Owner Verification:**
   ```rust
   if credential.owner_id != request.user_id {
       return Err(CredentialError::PermissionDenied);
   }
   ```

2. **Scope Validation:**
   ```rust
   if !check_scope_access(&credential, request.scope) {
       return Err(CredentialError::PermissionDenied);
   }
   ```

3. **Permission Check:**
   ```rust
   if !acl.has_permission(request.user_id, Permission::Read) {
       return Err(CredentialError::PermissionDenied);
   }
   ```

4. **Audit Logging:**
   ```rust
   audit_log.log_access(
       credential_id,
       user_id,
       operation,
       result,
   );
   ```

---

## 9. Audit & Compliance

### 9.1 Audit Logging

**Log Events:**

| Event Type | Severity | Required Fields | Retention |
|------------|----------|-----------------|-----------|
| Credential Created | INFO | user_id, credential_id, type, timestamp | 90 days |
| Credential Accessed | INFO | user_id, credential_id, operation, timestamp | 90 days |
| Credential Modified | WARN | user_id, credential_id, changes, timestamp | 90 days |
| Credential Deleted | WARN | user_id, credential_id, timestamp | 90 days |
| Authentication Success | INFO | user_id, protocol, timestamp, latency | 30 days |
| Authentication Failure | WARN | user_id, protocol, reason, timestamp | 90 days |
| Permission Denied | ERROR | user_id, credential_id, operation, timestamp | 90 days |
| Test Failed | WARN | credential_id, reason, timestamp | 30 days |
| Rotation Started | INFO | credential_id, policy, timestamp | 90 days |
| Encryption Key Rotated | WARN | key_id, timestamp | 365 days |

**Log Format (JSON):**

```json
{
  "event_type": "credential_access",
  "severity": "INFO",
  "timestamp": "2026-02-03T10:30:00.123Z",
  "user_id": "user-123",
  "credential_id": "cred-456",
  "operation": "retrieve",
  "result": "success",
  "latency_ms": 45,
  "ip_address": "192.168.1.100",
  "user_agent": "nebula/1.0.0",
  "trace_id": "trace-abc123"
}
```

**Log Protection:**

```
Immutability: Append-only storage
Integrity: HMAC signatures on log entries
Encryption: Logs encrypted at rest
Access Control: Read-only except by log service
Backup: Daily backups with 90-day retention
```

### 9.2 Compliance Mappings

**SOC 2 Type II:**

| Control | Requirement | Implementation |
|---------|-------------|----------------|
| CC6.1 | Logical access controls | Owner-based access + ACLs |
| CC6.2 | Authentication | OAuth2, SAML, LDAP, mTLS |
| CC6.3 | Authorization | Permission checks before operations |
| CC6.6 | Encryption | AES-256-GCM at rest, TLS 1.3 in transit |
| CC6.7 | Key management | HSM, key rotation, separation of duties |
| CC7.2 | Monitoring | Audit logs, metrics, alerting |

**ISO 27001:2013:**

| Control | Title | Implementation |
|---------|-------|----------------|
| A.9.2.1 | User registration | Owner ID required for all credentials |
| A.9.2.4 | Secret authentication | SecretString with zeroization |
| A.9.4.1 | Information access restriction | ACLs with granular permissions |
| A.10.1.1 | Cryptographic controls | AES-256-GCM, Argon2id, TLS 1.3 |
| A.10.1.2 | Key management | HSM storage, 90-day rotation |
| A.12.4.1 | Event logging | Comprehensive audit logs |
| A.12.4.3 | Administrator logs | Elevated access logged separately |

**HIPAA:**

| Requirement | Standard | Implementation |
|-------------|----------|----------------|
| Access Control | §164.312(a)(1) | Role-based access with ACLs |
| Audit Controls | §164.312(b) | Audit logs with 90-day retention |
| Integrity | §164.312(c)(1) | AES-GCM authentication tags |
| Transmission Security | §164.312(e)(1) | TLS 1.3 for all network traffic |
| Encryption | §164.312(a)(2)(iv) | AES-256-GCM at rest |

**GDPR:**

| Article | Requirement | Implementation |
|---------|-------------|----------------|
| Art. 5 | Data minimization | Only store required credential data |
| Art. 17 | Right to erasure | Credential deletion API |
| Art. 25 | Data protection by design | Encryption by default |
| Art. 32 | Security of processing | AES-256-GCM, access controls |
| Art. 33 | Breach notification | Incident response procedures |

### 9.3 Compliance Reports

**Automated Evidence Collection:**

```
Daily:
- Access log summary
- Failed authentication attempts
- Permission denied events
- Encryption key status

Weekly:
- Credential creation/deletion report
- Unusual access patterns
- Certificate expiration warnings
- Rate limit violations

Monthly:
- Compliance dashboard
- Key rotation compliance
- Audit log integrity verification
- Security testing results
```

---

## 10. Security Testing

### 10.1 Security Testing Requirements

**Testing Phases:**

| Phase | Type | Frequency | Tools | Scope |
|-------|------|-----------|-------|-------|
| Development | Unit Tests | Every commit | cargo test | Individual functions |
| Development | Integration Tests | Every commit | cargo test | Component interaction |
| Pre-Release | SAST | Every PR | cargo clippy, cargo audit | Source code |
| Pre-Release | DAST | Weekly | OWASP ZAP, Burp Suite | Running application |
| Pre-Release | Dependency Scan | Daily | cargo audit, Snyk | Dependencies |
| Post-Release | Penetration Test | Quarterly | External firm | Production-like |
| Post-Release | Bug Bounty | Continuous | HackerOne | Production |

### 10.2 Unit Testing Security

**Cryptographic Tests:**

```rust
#[cfg(test)]
mod crypto_tests {
    use super::*;
    
    #[test]
    fn test_encryption_decryption_roundtrip() {
        let key = EncryptionKey::from_bytes([0u8; 32]);
        let nonce_gen = NonceGenerator::new();
        let plaintext = b"secret credential data";
        
        // Encrypt
        let encrypted = encrypt(plaintext, &key, &nonce_gen).unwrap();
        
        // Decrypt
        let decrypted = decrypt(&encrypted, &key).unwrap();
        
        assert_eq!(plaintext, &decrypted[..]);
    }
    
    #[test]
    fn test_tampering_detection() {
        let key = EncryptionKey::from_bytes([0u8; 32]);
        let nonce_gen = NonceGenerator::new();
        let plaintext = b"secret credential data";
        
        let mut encrypted = encrypt(plaintext, &key, &nonce_gen).unwrap();
        
        // Tamper with ciphertext
        encrypted.ciphertext[0] ^= 0xFF;
        
        // Decryption should fail
        assert!(decrypt(&encrypted, &key).is_err());
    }
    
    #[test]
    fn test_nonce_uniqueness() {
        let nonce_gen = NonceGenerator::new();
        let mut nonces = HashSet::new();
        
        // Generate 100,000 nonces
        for _ in 0..100_000 {
            let nonce = nonce_gen.generate();
            assert!(!nonces.contains(&nonce), "Nonce collision detected!");
            nonces.insert(nonce);
        }
    }
    
    #[test]
    fn test_key_zeroization() {
        let key = EncryptionKey::from_bytes([0xAA; 32]);
        let key_ptr = key.as_bytes().as_ptr();
        
        // Drop key
        drop(key);
        
        // Memory should be zeroed (unsafe check)
        unsafe {
            let bytes = std::slice::from_raw_parts(key_ptr, 32);
            // This test is approximate; zeroization isn't guaranteed by Rust
            // In practice, use Zeroize trait which provides best-effort clearing
        }
    }
}
```

**OAuth2 Security Tests:**

```rust
#[tokio::test]
async fn test_pkce_challenge_validation() {
    let pkce = PkceChallenge::generate();
    
    // Verify challenge format
    assert_eq!(pkce.method(), "S256");
    assert_eq!(pkce.challenge().len(), 43); // Base64url length
    
    // Verify challenge derived from verifier
    let expected_challenge = base64url(sha256(pkce.verifier().expose()));
    assert_eq!(pkce.challenge(), expected_challenge);
}

#[tokio::test]
async fn test_state_csrf_protection() {
    let flow = OAuth2AuthorizationCode::new(config);
    let auth_url = flow.authorization_url().unwrap();
    
    // Extract state from URL
    let state = extract_state_from_url(&auth_url);
    
    // Try to exchange code with wrong state (CSRF attack)
    let result = flow.exchange_code("code", "wrong_state").await;
    
    assert!(matches!(result, Err(OAuth2Error::StateMismatch)));
}
```

**SQL Injection Tests:**

```rust
#[tokio::test]
async fn test_sql_injection_prevention() {
    let storage = LocalStorageProvider::new("test.db").await.unwrap();
    
    // Attempt SQL injection in credential ID
    let malicious_id = "'; DROP TABLE credentials; --";
    
    let result = storage.retrieve(&CredentialId::from(malicious_id)).await;
    
    // Should return NotFound, not execute SQL injection
    assert!(result.is_ok());
    
    // Verify table still exists
    let count = sqlx::query!("SELECT COUNT(*) FROM credentials")
        .fetch_one(&storage.pool)
        .await
        .unwrap();
    
    // Test passed if we got here (table not dropped)
}
```

### 10.3 Penetration Testing

**Test Scenarios:**

1. **Authentication Bypass:**
   ```
   Objective: Attempt to access credentials without authentication
   Method: Forge JWT tokens, manipulate OAuth state
   Success Criteria: All attempts blocked, logged
   ```

2. **Privilege Escalation:**
   ```
   Objective: Low-privilege user accesses other users' credentials
   Method: Manipulate owner_id, bypass ACL checks
   Success Criteria: Authorization failures, audit logs
   ```

3. **Encryption Breaking:**
   ```
   Objective: Decrypt credentials without encryption key
   Method: Timing attacks, key extraction, brute force
   Success Criteria: Infeasible within reasonable time
   ```

4. **MITM Attack:**
   ```
   Objective: Intercept and modify OAuth/SAML traffic
   Method: SSL stripping, certificate spoofing
   Success Criteria: TLS enforcement, certificate pinning
   ```

5. **DoS Attack:**
   ```
   Objective: Overwhelm system to deny service
   Method: Flood with credential test requests
   Success Criteria: Rate limiting effective, system stable
   ```

**Penetration Testing Checklist:**

```
Authentication:
☐ OAuth2 PKCE enforcement
☐ SAML signature validation
☐ LDAP bind over TLS only
☐ mTLS certificate validation
☐ JWT signature verification
☐ API key rate limiting

Authorization:
☐ Owner-based access control
☐ Scope isolation enforcement
☐ ACL permission checks
☐ Privilege escalation prevention

Cryptography:
☐ AES-256-GCM encryption
☐ Nonce uniqueness
☐ Key zeroization
☐ Constant-time comparisons

Network:
☐ TLS 1.3 enforcement
☐ Certificate validation
☐ Strong cipher suites
☐ Network segmentation

Storage:
☐ Encryption at rest
☐ SQL injection prevention
☐ Backup encryption
☐ Access control

Logging:
☐ Audit log completeness
☐ Log immutability
☐ Sensitive data redaction
☐ Log retention policy
```

### 10.4 Fuzzing

**Fuzz Testing Targets:**

```rust
use cargo_fuzz::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Fuzz encryption/decryption
    if data.len() >= 32 {
        let key = EncryptionKey::from_bytes(data[..32].try_into().unwrap());
        let nonce_gen = NonceGenerator::new();
        
        if let Ok(encrypted) = encrypt(&data[32..], &key, &nonce_gen) {
            let _ = decrypt(&encrypted, &key);
        }
    }
});

fuzz_target!(|data: &[u8]| {
    // Fuzz OAuth2 token parsing
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = TokenResponse::deserialize(s);
    }
});

fuzz_target!(|data: &[u8]| {
    // Fuzz SAML XML parsing
    if let Ok(xml) = std::str::from_utf8(data) {
        let _ = roxmltree::Document::parse(xml);
    }
});
```

**Fuzzing Campaigns:**

```bash
# Run fuzzing for 24 hours
cargo fuzz run encryption --jobs 8 -- -max_total_time=86400

# Coverage-guided fuzzing
cargo fuzz coverage encryption
cargo cov -- show --format=html
```

---

## 11. Incident Response

### 11.1 Incident Classification

**Severity Levels:**

| Level | Definition | Response Time | Examples |
|-------|------------|---------------|----------|
| **P0 - Critical** | Active exploitation, data breach | 15 minutes | Encryption key compromised, mass credential theft |
| **P1 - High** | Imminent threat, vulnerability | 1 hour | Unpatched critical CVE, authentication bypass |
| **P2 - Medium** | Potential vulnerability | 4 hours | Misconfiguration, weak cipher suite |
| **P3 - Low** | Minor issue, no immediate risk | 24 hours | Audit log formatting issue |

### 11.2 Incident Response Procedures

**Phase 1: Detection & Identification (0-15 minutes)**

```
Actions:
1. Alert triggered (automated monitoring)
2. On-call engineer notified
3. Initial assessment:
   - What happened?
   - What is affected?
   - What is the impact?
4. Escalate if P0/P1

Tools:
- Prometheus alerts
- CloudWatch alarms
- SIEM (Splunk/ELK)
- Audit log analysis
```

**Phase 2: Containment (15-60 minutes)**

```
Actions:
1. Isolate affected systems
2. Revoke compromised credentials
3. Block malicious IPs
4. Disable vulnerable features
5. Activate backup systems

Example: Encryption Key Compromise
- Immediately rotate master key
- Re-encrypt all credentials with new key
- Revoke access for suspicious users
- Enable 2FA for admin access
```

**Phase 3: Eradication (1-4 hours)**

```
Actions:
1. Remove malware/backdoors
2. Patch vulnerabilities
3. Reset compromised credentials
4. Validate system integrity
5. Deploy fixes

Example: SQL Injection Vulnerability
- Deploy patched version with parameterized queries
- Audit database for evidence of exploitation
- Restore from clean backup if necessary
- Update WAF rules to block similar attacks
```

**Phase 4: Recovery (4-24 hours)**

```
Actions:
1. Restore normal operations
2. Monitor for recurrence
3. Validate security controls
4. Communicate with stakeholders
5. Document timeline

Example: DoS Attack
- Re-enable services gradually
- Monitor for continued attack
- Adjust rate limits if needed
- Update runbooks
```

**Phase 5: Post-Incident (24-72 hours)**

```
Actions:
1. Root cause analysis
2. Post-mortem meeting
3. Update procedures
4. Security improvements
5. Compliance reporting

Deliverables:
- Incident report
- Timeline of events
- Remediation actions
- Prevention measures
- Compliance notifications (if required)
```

### 11.3 Breach Notification

**GDPR Requirements (Article 33):**

```
Timeline: 72 hours after becoming aware
Recipient: Supervisory authority

Required Information:
- Nature of the breach
- Categories of data affected
- Number of individuals affected
- Likely consequences
- Measures taken to address breach
- Contact point for more information
```

**Customer Notification (Article 34):**

```
Condition: High risk to individuals' rights
Timeline: Without undue delay

Notification Should Include:
- Clear description in plain language
- Contact point
- Likely consequences
- Measures taken/recommended
```

**SOC 2 Reporting:**

```
Timeline: Contractually defined (typically 24-48 hours)
Recipient: Affected customers, auditors

Required Information:
- Incident description
- Affected systems/data
- Timeline
- Remediation actions
- Audit trail
```

### 11.4 Incident Response Playbooks

**Playbook 1: Encryption Key Compromise**

```
Detection Indicators:
- Unauthorized access to key storage
- Key exported to external location
- Suspicious key usage patterns

Immediate Actions:
1. Rotate master key (automated script)
2. Re-encrypt all credentials with new key
3. Invalidate old key
4. Audit recent key usage
5. Alert security team

Investigation:
1. Review access logs for key storage
2. Check for unauthorized key exports
3. Identify compromised accounts
4. Determine scope of access

Remediation:
1. Deploy new encryption keys
2. Update key management procedures
3. Implement additional key protection (HSM)
4. Conduct security review

Prevention:
1. Store keys in HSM
2. Multi-party key management
3. Key usage monitoring
4. Regular key rotation
```

**Playbook 2: Privilege Escalation Attack**

```
Detection Indicators:
- User accessing credentials they don't own
- ACL bypass attempts
- Suspicious permission grants

Immediate Actions:
1. Suspend compromised user account
2. Revoke active sessions
3. Review recent access logs
4. Identify affected credentials

Investigation:
1. Determine attack vector
2. Identify exploited vulnerability
3. Scope of unauthorized access
4. Check for data exfiltration

Remediation:
1. Patch authorization bypass vulnerability
2. Reset passwords for affected users
3. Rotate compromised credentials
4. Strengthen access controls

Prevention:
1. Code review of authorization logic
2. Regular penetration testing
3. Implement defense in depth
4. Anomaly detection for access patterns
```

---

## 12. Security Best Practices

### 12.1 Development Best Practices

**Secure Coding Guidelines:**

```
1. Input Validation:
   ✓ Validate all user input
   ✓ Use type-safe parsers (serde)
   ✓ Reject unexpected input formats
   ✗ Never trust client-provided data

2. Output Encoding:
   ✓ Use SecretString for sensitive data
   ✓ Implement custom Debug/Display traits
   ✓ Redact secrets in logs
   ✗ Never log raw credentials

3. Error Handling:
   ✓ Use Result<T, E> for all fallible operations
   ✓ Provide generic error messages to users
   ✓ Log detailed errors internally
   ✗ Never expose stack traces to users

4. Cryptography:
   ✓ Use well-tested libraries (RustCrypto)
   ✓ Never implement your own crypto
   ✓ Use constant-time comparisons
   ✗ Never reuse nonces

5. Concurrency:
   ✓ Use Arc<RwLock<T>> for shared state
   ✓ Prefer immutable data structures
   ✓ Use channels for communication
   ✗ Avoid shared mutable state

6. Dependencies:
   ✓ Pin versions in Cargo.lock
   ✓ Run cargo audit regularly
   ✓ Review dependency security advisories
   ✗ Don't use deprecated/unmaintained crates
```

**Code Review Checklist:**

```
Security Review:
☐ No hardcoded credentials
☐ Input validation present
☐ SQL queries parameterized
☐ Secrets use SecretString
☐ Encryption uses approved algorithms
☐ Error messages don't leak info
☐ Logging doesn't expose secrets
☐ Authorization checks present
☐ Rate limiting implemented
☐ Tests cover security scenarios

Cryptography Review:
☐ AES-256-GCM used correctly
☐ Nonces unique per encryption
☐ Keys zeroized after use
☐ Argon2 parameters secure
☐ Constant-time comparisons
☐ No weak algorithms (MD5, SHA1)

Authentication Review:
☐ OAuth2 uses PKCE
☐ SAML signatures validated
☐ LDAP over TLS only
☐ mTLS certificates validated
☐ JWT exp/nbf claims checked
☐ API keys hashed before storage
```

### 12.2 Deployment Best Practices

**Production Hardening:**

```
Infrastructure:
☐ TLS 1.3 enforced
☐ Strong cipher suites only
☐ Certificate validation enabled
☐ Firewall rules configured
☐ Network segmentation in place
☐ Private networks for database/cache
☐ WAF deployed (Cloudflare/AWS WAF)

Configuration:
☐ Master key in HSM/Key Vault
☐ Database encryption enabled
☐ Backup encryption configured
☐ Audit logging enabled
☐ Metrics collection active
☐ Rate limiting configured
☐ Timeout values set

Monitoring:
☐ Prometheus alerts configured
☐ Log aggregation active (ELK/Splunk)
☐ Anomaly detection enabled
☐ Failed auth attempt monitoring
☐ Certificate expiration alerts
☐ Key rotation compliance checks
☐ Backup success monitoring
```

### 12.3 Operational Best Practices

**Regular Security Activities:**

```
Daily:
☐ Review security alerts
☐ Check failed authentication logs
☐ Monitor rate limit violations
☐ Verify backup success

Weekly:
☐ Run vulnerability scans (cargo audit)
☐ Review access logs for anomalies
☐ Check certificate expiration dates
☐ Update dependency security advisories

Monthly:
☐ Test backup restoration
☐ Review ACL permissions
☐ Audit admin access logs
☐ Update security runbooks
☐ Security training for team

Quarterly:
☐ Penetration testing
☐ Security audit
☐ Disaster recovery drill
☐ Incident response tabletop exercise
☐ Compliance review

Annually:
☐ External security audit
☐ SOC 2 audit
☐ Security policy review
☐ Encryption algorithm review
☐ Business continuity planning
```

**Security Incident Drills:**

```
Scenario 1: Credential Breach Simulation
- Simulate credential database leak
- Practice containment procedures
- Test notification procedures
- Measure response time
- Update runbooks based on findings

Scenario 2: Key Compromise Simulation
- Simulate master key compromise
- Practice key rotation procedures
- Test credential re-encryption
- Verify system recovery
- Document lessons learned

Scenario 3: DoS Attack Simulation
- Simulate high request volume
- Test rate limiting effectiveness
- Practice scaling procedures
- Verify system resilience
- Update capacity planning
```

---

## Conclusion

This security specification provides comprehensive security guidance for the nebula-credential crate, covering:

✅ **Threat Model**: 10 detailed threat scenarios with mitigations, STRIDE analysis  
✅ **Attack Surface**: Network, API, Storage, Memory attack vectors  
✅ **Security Requirements**: 34 requirements across confidentiality, integrity, availability, authentication, authorization, audit  
✅ **Cryptography**: AES-256-GCM, Argon2id, BLAKE3, key management, post-quantum readiness  
✅ **Protocol Security**: OAuth2, SAML, LDAP, mTLS, JWT, API Keys with vulnerability mitigations  
✅ **Storage Security**: Encryption at rest, database hardening, backup security, cache security  
✅ **Network Security**: TLS 1.3, network segmentation, firewall rules, API security  
✅ **Access Control**: Ownership model, ACLs, scope isolation, privilege escalation prevention  
✅ **Compliance**: SOC 2, ISO 27001, HIPAA, GDPR mappings with audit logging  
✅ **Security Testing**: Unit tests, penetration testing, fuzzing, vulnerability scanning  
✅ **Incident Response**: Classification, procedures, breach notification, playbooks  
✅ **Best Practices**: Secure coding, code review, deployment hardening, operational security  

**Security Posture:**
- **Confidentiality**: STRONG (AES-256-GCM, TLS 1.3, SecretString redaction)
- **Integrity**: STRONG (Authenticated encryption, signature validation, audit logs)
- **Availability**: MEDIUM (Rate limiting, circuit breakers, timeouts)
- **Compliance**: HIGH (SOC 2, ISO 27001, HIPAA, GDPR ready)
- **Risk Level**: LOW (with proper deployment and key management)

**Next Steps:**
1. Implement security controls per this specification
2. Conduct security review of implementation
3. Perform penetration testing
4. Obtain security certifications (SOC 2, ISO 27001)

---
