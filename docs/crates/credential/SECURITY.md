# Security

## Threat Model

### Assets

**Primary:**
- Credentials (OAuth2 tokens, API keys, passwords, certificates)
- Encryption keys (master key, per-credential encryption keys)
- User identity data (usernames, bind DNs, group memberships)

**Secondary:**
- Credential metadata (creation time, expiration, usage stats)
- Audit logs
- Rotation state and configuration

### Trust Boundaries

- **Untrusted:** Network, storage backends, user input, OAuth2/SAML/LDAP providers, cache contents
- **Trusted:** OS kernel, Rust std, RustCrypto libs, HSM/KMS (when used)

### Threat Actors

| Actor | Capabilities | Motivation | Risk |
|-------|-------------|------------|------|
| External Attacker | Network access, public exploits | Data theft, ransom, disruption | HIGH |
| Malicious Insider | System access, architecture knowledge | Data exfiltration, sabotage | CRITICAL |
| Compromised Service Account | API access, limited privileges | Lateral movement, privilege escalation | HIGH |
| Nation State | APT, zero-days | Espionage, long-term access | CRITICAL |

### STRIDE Analysis

| Category | Threat | Severity | Mitigations |
|----------|--------|----------|-------------|
| **Spoofing** | Attacker impersonates legitimate caller | HIGH | OAuth2 PKCE, SAML signatures, mTLS client certs, scope enforcement |
| **Tampering** | Attacker modifies credentials in storage | CRITICAL | AES-256-GCM authenticated encryption (AEAD), HMAC integrity |
| **Repudiation** | User denies performing credential operation | MEDIUM | Structured audit logging with timestamps, trace IDs, context |
| **Information Disclosure** | Credentials leaked via logs/errors/memory | CRITICAL | `SecretString` redaction, encrypted storage, zeroization |
| **Denial of Service** | System overwhelmed by credential operations | MEDIUM | Rate limiting (P-004), circuit breakers, cache, timeouts |
| **Elevation of Privilege** | User accesses credentials outside their scope | CRITICAL | Scope isolation, owner-based ACL, permission checks on every op |

### Threat Scenarios (T1–T10)

#### T1: Credential Theft from Storage

**Impact:** CRITICAL — full credential compromise if encryption broken

**Attack vectors:** SQL injection, filesystem misconfiguration, cloud storage misconfiguration, backup file exposure

**Mitigations:**
- AES-256-GCM encryption with Argon2id key derivation
- Keys stored separately from encrypted data (KMS/HSM in production)
- Parameterized queries via sqlx (compile-time validated)
- Least privilege for storage backend access
- Encrypted backups with separate key

**Residual risk:** LOW (requires BOTH storage access AND encryption key)

#### T2: Encryption Key Compromise

**Impact:** CRITICAL — can decrypt all stored credentials

**Attack vectors:** Memory dump, KMS access compromise, env variable exposure, log leakage

**Mitigations:**
- `ZeroizeOnDrop` for `EncryptionKey` — memory cleared on drop
- KMS/HSM for key storage in production (local derivation for dev)
- Key rotation policy (90-day default)
- `SecretString` — key never appears in Debug/Display/logs
- Key derivation via Argon2id (not stored raw)

**Residual risk:** MEDIUM (depends on key management practices)

#### T3: Man-in-the-Middle (MITM)

**Impact:** HIGH — session hijacking, credential theft during OAuth2/SAML/LDAP flows

**Mitigations:**
- TLS 1.3 mandatory for all provider backends (AWS, Vault, LDAP)
- Certificate validation enforced (no `danger_accept_invalid_certs`)
- PKCE for OAuth2 (prevents authorization code interception)
- SAML signature verification
- LDAPS (LDAP over TLS)

**Residual risk:** LOW

#### T4: Credential Replay Attack

**Impact:** HIGH — unauthorized access via captured tokens

**Mitigations:**
- OAuth2 `state` parameter (CSRF protection, single-use, 10-minute TTL)
- Short-lived access tokens (15–60 min), refresh token rotation
- Unique nonces per encryption operation
- SAML `NotOnOrAfter` validation
- JWT `exp`/`nbf` claim enforcement

**Residual risk:** LOW

#### T5: Privilege Escalation (Cross-Scope Access)

**Impact:** CRITICAL — lateral movement, data exfiltration

**Mitigations:**
- Immutable credential ownership (`OwnerId` set at creation, no setter)
- `CredentialContext` scope validated on every `retrieve`, `list`, `validate`
- Cache keyed by `(CredentialId, ScopeLevel)` — no cross-scope hits
- `caller_scope.is_contained_in_strict(&entry.owner_scope, resolver)` — verified on every retrieve; uses `ScopeResolver` to check full ownership chain (execution→workflow→project→organization)
- `CredentialContext.caller_scope: ScopeLevel` carries the requester's runtime scope; never trusts caller-provided string claims
- `ScopeViolation` error logged with full context (caller_scope, credential_id, owner_scope) before returning `Err`
- `#![forbid(unsafe_code)]` on all scope enforcement paths

**Residual risk:** LOW

#### T6: Timing Attack on Encryption

**Impact:** MEDIUM — partial key leakage via timing differences

**Mitigations:**
- Constant-time comparison via `subtle` crate
- AES-GCM provides authenticated encryption (detects tampering before comparison)
- Argon2id is constant-time by design
- No early-exit on decryption failure paths

**Residual risk:** VERY LOW

#### T7: Denial of Service (Credential Fetch Storm)

**Impact:** MEDIUM — service unavailability, provider rate limit exhaustion

**Mitigations:**
- Cache layer reduces storage pressure (>80% hit ratio target)
- Rate limiting per user/scope (P-004)
- Connection pooling with max limits
- Request timeout enforcement (30s default, 5s for DB, 10s for HTTP)
- Backpressure via circuit breaker (nebula-resilience integration)

**Residual risk:** MEDIUM

#### T8: Secret Exposure in Logs

**Impact:** CRITICAL — credential plaintext in log files

**Mitigations:**
- `SecretString` implements `Debug` → `"***"`, `Display` → `"***"`
- `Serialize` impl redacts value
- Explicit `expose_secret()` method — makes usage auditable in code review
- No credential material in error messages or panic messages
- Tracing structured fields never include secret values

**Residual risk:** LOW

#### T9: Supply Chain Attack

**Impact:** CRITICAL — malicious dependency introduces backdoor

**Mitigations:**
- `Cargo.lock` pinned in repository
- `cargo audit` in CI pipeline
- Minimal dependency tree (only RustCrypto, tokio, serde, thiserror)
- Code review required for dependency version updates
- `#![forbid(unsafe_code)]` limits attack surface

**Residual risk:** MEDIUM (ongoing vigilance required)

#### T10: Side-Channel Attack via Cache Timing

**Impact:** LOW — partial credential leakage via CPU cache timing

**Mitigations:**
- Memory zeroization (`Zeroize` on drop)
- Process isolation
- Cache-oblivious patterns where possible
- `SecretString` keeps sensitive data lifetime minimal

**Residual risk:** VERY LOW

---

## Cryptographic Specifications

### Symmetric Encryption

```
Algorithm:   AES-256-GCM (AEAD)
Key Size:    256 bits
Nonce Size:  96 bits (12 bytes)
Tag Size:    128 bits (16 bytes)
Mode:        Galois/Counter Mode

Rationale: NIST approved (FIPS 140-2), hardware-accelerated (AES-NI),
           authenticated encryption detects tampering, resistant to timing attacks.
```

### Key Derivation

```
Algorithm:    Argon2id
Memory Cost:  19 MiB (19456 KiB)
Time Cost:    2 iterations
Parallelism:  1 thread
Salt Size:    128 bits (16 bytes)
Output:       256 bits (32 bytes)

Rationale: OWASP recommendation, resistant to GPU/ASIC attacks,
           memory-hard function, side-channel resistant (id variant).
```

### Nonce Generation

```
Format:    [4-byte random prefix | 8-byte counter]
Space:     2^96 total (exceeds GCM safety margin)
Collision: < 2^-32 for same prefix
CRITICAL:  Nonce reuse with same key is a catastrophic failure —
           AES-GCM becomes unauthenticated and key-recoverable.
```

### Key Hierarchy

```
Root Key (HSM/KMS in production)
  └─> Master Key (derived from admin password + salt)
       └─> Per-credential encryption key (random 256-bit)
            Encrypted with master key, stored alongside ciphertext
```

### Post-Quantum Readiness

Current algorithms provide 128-bit security level. Migration path:
- Algorithm version field in `EncryptedData` enables transparent migration
- When NIST PQC standards stabilize (Kyber, Dilithium), add new version
- Existing data re-encrypted on rotation or explicit migration

---

## Security Controls

### Authentication Protocol Security

#### OAuth2

- PKCE **mandatory** for all authorization code flows (OAuth 2.1 requirement)
- `state` parameter: 32 bytes random, single-use, 10-minute TTL
- Exact redirect URI match (no pattern matching)
- `AuthStyle::Header` (RFC 6749 standard) or `AuthStyle::PostBody` (GitHub, Slack)
- Access tokens: short-lived, in-memory only during request
- Refresh tokens: encrypted at rest, rotated on each use

| Vulnerability | Mitigation | RFC |
|---------------|------------|-----|
| Code interception | PKCE mandatory | RFC 7636 |
| CSRF | State parameter | RFC 6749 §10.12 |
| Token theft | TLS + short expiration | RFC 6749 §10.4 |
| Open redirect | Strict redirect URI | RFC 6749 §10.6 |

#### SAML 2.0

- RSA-SHA256 or ECDSA-SHA256 signature verification
- Assertion validation: NotBefore, NotOnOrAfter, Recipient, Audience, InResponseTo
- XML security: entity expansion limits, external entity disabled, schema validation
- Binding: HTTP-POST (default) or HTTP-Redirect

#### LDAP

- LDAPS (TLS from start) or STARTTLS — plaintext only for development
- Filter escaping per RFC 4515 (`(` → `\28`, `)` → `\29`, `*` → `\2a`)
- Parameterized DN construction (no injection)
- Rate limiting on bind attempts

#### mTLS

- X.509 certificate chain validation to trusted CA
- Certificate not expired, not revoked (OCSP/CRL)
- Key usage includes `digitalSignature`/`keyEncipherment`
- Extended key usage includes `clientAuth`
- Private key: PEM format, encrypted at rest

#### JWT

- Allowed algorithms: HS256, RS256, ES256 (forbid `none`, forbid HS256 with asymmetric key)
- Required claims: `exp`, `nbf` validated; `iss`, `aud` checked when configured
- Clock skew tolerance: ±60 seconds
- Symmetric keys: 256-bit minimum, rotated every 90 days

#### API Keys

- Format: `sk_<43-char base64url random>` (256-bit entropy, prefix for detection)
- Storage: BLAKE3 hash only (never stored in plaintext)
- Validation: constant-time comparison via `subtle`
- Rotation: 7-day grace period, old and new both valid during transition

---

## Defense-in-Depth Layers

1. **Encryption at rest:** AES-256-GCM; unique nonces; key separation from data
2. **Access control:** `CredentialContext.caller_scope: ScopeLevel` + `is_contained_in_strict` with `ScopeResolver`; least privilege; immutable ownership (`OwnerId` set at creation); `CredentialKey` identifies type (not secret); `CredentialId` identifies instance
3. **Memory protection:** `zeroize` on drop; `SecretString`; minimal secret lifetime in memory
4. **Audit logging:** Credential lifecycle events; rotation outcomes; scope violations
5. **Network:** TLS 1.3 for provider backends (AWS, Vault, LDAP); mTLS where supported

---

## Abuse Cases

| Abuse Case | Prevention | Detection | Response |
|------------|-----------|-----------|----------|
| **Credential theft from storage** | Encryption at rest; key separation | Decryption failure alerts | Key rotation; re-encrypt all |
| **Cross-tenant access** | `caller_scope.is_contained_in_strict(&owner_scope, resolver)` on every retrieve; hierarchical containment verified via `ScopeResolver` | `ScopeViolation` errors; audit log | Fail-fast; alert; incident |
| **Log exposure of secrets** | `SecretString` redaction; no secrets in errors | Security review; log scanning | Patch; rotate exposed credentials |
| **Fetch storm / DoS** | Cache; rate limits (P-004) | Latency/throughput metrics spike | Backpressure; circuit breaker |
| **Encryption key compromise** | HSM/KMS; key rotation | Key access anomalies | Immediate rotation; re-encrypt all; revoke key |
| **Replay attack** | PKCE; state param; short TTL; nonce | Duplicate request detection | Revoke tokens; rotate credentials |

---

## Compliance Mappings

### SOC 2 Type II

| Control | Requirement | Implementation |
|---------|-------------|----------------|
| CC6.1 | Logical access controls | Owner-based access + scope isolation |
| CC6.2 | Authentication mechanisms | OAuth2, SAML, LDAP, mTLS, API Keys |
| CC6.3 | Authorization enforcement | Permission checks before every operation |
| CC6.6 | Encryption | AES-256-GCM at rest, TLS 1.3 in transit |
| CC6.7 | Key management | HSM/KMS, 90-day rotation, separation of duties |
| CC7.2 | Monitoring | Audit logs, metrics, structured alerting |

### ISO 27001:2013

| Control | Title | Implementation |
|---------|-------|----------------|
| A.9.2.1 | User registration | Owner ID required for all credentials |
| A.9.2.4 | Secret authentication info | `SecretString` with zeroization |
| A.9.4.1 | Information access restriction | Scope isolation with `ScopeLevel` |
| A.10.1.1 | Cryptographic controls | AES-256-GCM, Argon2id, TLS 1.3 |
| A.10.1.2 | Key management | HSM storage, 90-day rotation |
| A.12.4.1 | Event logging | Structured audit logs with retention |

### HIPAA

| Requirement | Standard | Implementation |
|-------------|----------|----------------|
| Access Control | §164.312(a)(1) | Context-driven scope isolation |
| Audit Controls | §164.312(b) | Audit logs with 90-day retention |
| Integrity | §164.312(c)(1) | AES-GCM authentication tags |
| Transmission Security | §164.312(e)(1) | TLS 1.3 for all network traffic |
| Encryption | §164.312(a)(2)(iv) | AES-256-GCM at rest |

### GDPR

| Article | Requirement | Implementation |
|---------|-------------|----------------|
| Art. 5 | Data minimization | Store only required credential fields |
| Art. 17 | Right to erasure | `CredentialManager::delete` with cascade |
| Art. 25 | Data protection by design | Encryption enabled by default |
| Art. 32 | Security of processing | AES-256-GCM, access controls, audit |
| Art. 33 | Breach notification | Incident response procedures (72h) |

### PCI-DSS

| Requirement | Implementation |
|-------------|----------------|
| Passwords changed every 90 days | `RotationPolicy::Periodic` (90-day interval) |
| Encrypted credential storage | AES-256-GCM at rest |
| Access logging | Structured audit events on every operation |
| Key management | HSM/KMS; key rotation; key separation |

---

## Incident Response

### Severity Classification

| Level | Definition | Response Time | Examples |
|-------|-----------|---------------|----------|
| P0 — Critical | Active exploitation, data breach | 15 minutes | Encryption key compromised, mass credential theft |
| P1 — High | Imminent threat, unpatched CVE | 1 hour | Authentication bypass, scope violation spike |
| P2 — Medium | Potential vulnerability | 4 hours | Misconfiguration, weak cipher usage |
| P3 — Low | Minor issue, no immediate risk | 24 hours | Audit log formatting, metric gap |

### Playbook: Encryption Key Compromise

1. **Contain (0–15 min):** Revoke compromised key version; deploy new master key
2. **Eradicate (15–60 min):** Re-encrypt all credentials with new key
3. **Recover (1–4 hours):** Validate decryption with new key; restore normal operations
4. **Post-incident:** Root cause analysis; implement additional key protection (HSM)

### Playbook: Scope Violation (Cross-Tenant Access)

1. **Contain:** Suspend affected user/service account; revoke active sessions
2. **Assess:** Review audit logs for scope of unauthorized access
3. **Remediate:** Patch authorization bypass; rotate compromised credentials
4. **Post-incident:** Add regression test; strengthen scope enforcement

### Breach Notification (GDPR Art. 33)

- **Timeline:** 72 hours after detection
- **Recipient:** Supervisory authority
- **Required info:** Nature of breach, categories of data, number of individuals affected, measures taken

---

## Security Requirements

### Must-Have (P0)

- Encryption enabled by default for all stored credentials
- Scope enforced on every retrieve, list, validate, rotate operation
- `SecretString` for all in-memory secrets
- `#![forbid(unsafe_code)]` at crate root
- No credential material in error messages, logs, or panic messages

### Should-Have (P1)

- Constant-time comparison for secrets (`subtle` crate)
- Rate limiting on credential operations (P-004)
- Audit event coverage for all credential lifecycle operations
- `cargo audit` in CI pipeline
- PKCE for all OAuth2 authorization code flows

---

## Security Test Plan

### Static Analysis

- `cargo audit` — dependency vulnerability scanning
- `cargo clippy` — lint for common security mistakes
- `#![forbid(unsafe_code)]` — compile-time enforcement

### Dynamic Tests

- Scope enforcement: cross-scope retrieve returns `Err(ScopeViolation)`
- Decryption failure: tampered ciphertext returns `Err(DecryptionFailed)`, never partial data
- Invalid ID rejection: path traversal, empty string, special characters
- OAuth2 state mismatch: wrong state parameter → error

### Property Tests (proptest)

- `CredentialId` validation: arbitrary strings never bypass validation
- Encrypted payload round-trip: encrypt → decrypt = identity for all inputs
- Nonce uniqueness: 100K nonces generated, zero collisions

### Fuzz Targets

- `CredentialId::new()` with adversarial input
- `decrypt()` with adversarial ciphertext
- OAuth2 token response parsing
- SAML XML assertion parsing

### Penetration Testing Checklist

```
Authentication:
☐ OAuth2 PKCE enforcement
☐ SAML signature validation
☐ LDAP bind over TLS only
☐ mTLS certificate chain validation
☐ JWT exp/nbf claim enforcement
☐ API key constant-time validation

Authorization:
☐ Owner-based access control
☐ Scope isolation enforcement
☐ No cross-scope cache hits
☐ Privilege escalation prevention

Cryptography:
☐ AES-256-GCM encryption round-trip
☐ Nonce uniqueness verification
☐ Key zeroization on drop
☐ Constant-time comparisons

Storage:
☐ SQL injection prevention (parameterized queries)
☐ Filesystem permission checks
☐ Backup encryption verification

Logging:
☐ No secrets in Debug/Display output
☐ No secrets in error messages
☐ Audit log completeness
```

---

## Comparative Analysis (n8n, Node-RED, etc.)

- **Adopt:** Encrypted storage; scope/tenant isolation; provider abstraction; OAuth2 flows
- **Reject:** Plaintext credential storage; global credential namespace; credentials in workflow JSON
- **Defer:** HSM integration (production add); rate limiting (P-004); audit pipeline (S3/Kafka)
