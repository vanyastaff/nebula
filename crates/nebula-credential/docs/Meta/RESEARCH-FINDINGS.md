# Research Findings: nebula-credential Documentation Best Practices

**Date**: 2026-02-03  
**Feature**: [[spec.md|Improve nebula-credential Documentation]]  
**Purpose**: Research best practices from Rust libraries, OAuth2 standards, and cloud providers to inform documentation structure and content.

---

## Executive Summary

Research conducted via three parallel agents covering:
1. **Rust Credential Libraries** (secrecy, aes-gcm, tokio patterns)
2. **OAuth2 Documentation Standards** (RFC 6749, provider patterns, security)
3. **Cloud Provider Integration** (AWS, Vault, Azure, Kubernetes)

**Key Findings**:
- **CRITICAL**: AES-GCM nonce reuse is catastrophic security failure - must be prominently documented
- **PKCE** now mandatory in OAuth 2.1 - current docs missing this
- **Managed identities** preferred over hardcoded credentials across all cloud providers
- **Lease/TTL management** varies significantly across providers - needs provider-specific guides

---

## Part 1: Rust Credential Libraries Best Practices

### 1.1 Secrecy Crate - Secret Handling Patterns

**Decision**: Use `secrecy::Secret<T>` for all credential types in documentation examples  
**Rationale**: Prevents accidental exposure in logs/debug, automatic zeroization, auditable access  
**Implementation**:

```rust
use secrecy::{Secret, ExposeSecret};

// GOOD: Explicit access makes secret handling auditable
let secret_token: Secret<String> = Secret::new("sensitive_value".to_string());
let exposed = secret_token.expose_secret(); // Only when needed

// BAD: Direct String allows accidental logging
let secret = "sensitive_value".to_string();  // Can be logged accidentally
```

**Documentation Requirements**:
- Show `ExposeSecret` trait usage prominently
- Explain automatic zeroization on drop
- Document common pitfalls (moves, copies, heap reallocation)
- Emphasize that secrecy prevents problematic patterns, zeroize handles cleanup

---

### 1.2 AES-GCM Encryption - Nonce Management

**Decision**: Include "CRITICAL: Nonce Management" warning section in encryption docs  
**Rationale**: Nonce reuse completely breaks GCM security - attackers can decrypt all messages with single plaintext/ciphertext pair  
**Implementation**:

```rust
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::Rng;

// CORRECT: Generate new nonce for EACH encryption
let nonce = Nonce::from_slice(&rng.gen::<[u8; 12]>());
let ciphertext = cipher.encrypt(&nonce, plaintext.as_ref())?;

// STORE: nonce with ciphertext (nonce doesn't need to be secret)
// NEVER: Reuse the same nonce with the same key
```

**Documentation Requirements**:
- **Mark as CRITICAL** in all encryption documentation
- Explain catastrophic failure mode (Internet scan found 184 HTTPS servers with repeated nonces)
- Show correct nonce generation patterns
- Mention AES-GCM-SIV as alternative for nonce-misuse-resistance
- Document GCM authentication tag (128-bit) handling
- Show constant-time implementation benefits (hardware AES-NI)

**Alternative Pattern**: Consider documenting AES-GCM-SIV for cases where nonce reuse is risk

---

### 1.3 Tokio Async Patterns - Credential Refresh

**Decision**: Document background refresh pattern with `Arc<RwLock<>>` and timeout handling  
**Rationale**: Production systems need automatic credential refresh without blocking workflows  
**Implementation**:

```rust
use tokio::time::{interval, timeout, Duration};
use std::sync::Arc;
use tokio::sync::RwLock;

// Shared credential with refresh logic
let credentials = Arc::new(RwLock::new(Credentials::default()));

// Background refresh task
tokio::spawn(async move {
    let mut refresh_interval = interval(Duration::from_secs(300)); // 5min
    
    loop {
        refresh_interval.tick().await;
        
        // Always set timeouts on credential refresh operations
        match timeout(Duration::from_secs(10), fetch_credentials()).await {
            Ok(Ok(new_creds)) => {
                *credentials.write().await = new_creds;
            }
            Ok(Err(e)) => {
                eprintln!("Credential refresh failed: {}", e);
                // Continue with existing credentials
            }
            Err(_) => {
                eprintln!("Credential refresh timeout");
            }
        }
    }
});
```

**Documentation Requirements**:
- Show TTL/expiration management pattern (refresh at 80% TTL, not 100%)
- Document failure handling (continue with existing credentials, don't panic)
- Show bounded waiting patterns (strict timeouts on all I/O)
- Document state machine for token renewal (Valid → NeedsRefresh → Refreshing → Valid/Error)

**Sources**: 
- [secrecy crate](https://docs.rs/secrecy/latest/secrecy/)
- [aes-gcm crate](https://docs.rs/aes-gcm/latest/aes_gcm/)
- [Attacks on GCM with Repeated Nonces - elttam](https://www.elttam.com/blog/key-recovery-attacks-on-gcm/)

---

## Part 2: OAuth2 Documentation Standards

### 2.1 PKCE (Proof Key for Code Exchange) - MANDATORY

**Decision**: Add PKCE section to OAuth2 documentation (currently missing)  
**Rationale**: RFC 9700 requires PKCE for OAuth 2.1; prevents authorization code interception attacks  
**Implementation**:

```rust
// 1. Generate code_verifier (random 43-128 char string)
let code_verifier = generate_random_string(128);

// 2. Create code_challenge = base64url(SHA256(code_verifier))
let code_challenge = base64_url_encode(sha256(code_verifier));

// 3. Authorization request with code_challenge
GET /authorize?client_id=...&redirect_uri=...&code_challenge={code_challenge}&code_challenge_method=S256

// 4. Token exchange with code_verifier
POST /token
  code={authorization_code}&code_verifier={code_verifier}
```

**Documentation Requirements**:
- Explain why PKCE critical for public clients (mobile, SPA)
- Show complete flow with code_verifier/code_challenge
- Reference RFC 7636
- Mark as REQUIRED in OAuth 2.1

---

### 2.2 State Parameter Validation - CSRF Protection

**Decision**: Document proper state parameter validation (currently minimal)  
**Rationale**: Prevents CSRF attacks where attacker tricks user into approving access for attacker's account  
**Implementation**:

```rust
// Generate cryptographically random 32+ byte state
let state = generate_random_bytes(32);

// Store in session with expiry
session.set("oauth_state", state, Duration::from_secs(600));

// On callback: VERIFY returned state matches stored state
if callback_state != session.get("oauth_state") {
    return Err("State mismatch - possible CSRF attack");
}
```

**Documentation Requirements**:
- Show secure state generation
- Document secure storage mechanisms
- Emphasize exact string comparison (not just presence check)
- Explain attack vectors prevented

---

### 2.3 Redirect URI Validation - Exact Matching

**Decision**: Document strict redirect URI validation (currently missing)  
**Rationale**: Pattern matching allows attackers to register `https://app.example.com.evil.com`  
**Implementation**:

```rust
// ✅ GOOD: Exact string comparison (case-sensitive)
fn validate_redirect_uri(provided: &str, registered: &str) -> Result<(), Error> {
    if provided == registered {
        Ok(())
    } else {
        Err("Redirect URI mismatch")
    }
}

// ❌ BAD: Pattern matching
fn bad_validate(provided: &str, pattern: &str) -> bool {
    provided.starts_with(pattern) // Vulnerable to subdomain attacks
}
```

**Documentation Requirements**:
- NEVER use wildcards or pattern matching
- ALWAYS exact string match (case-sensitive)
- Validate BOTH host and path components
- Special case: localhost port numbers can be flexible for native apps

---

### 2.4 Token Refresh & Rotation

**Decision**: Add detailed refresh token rotation documentation with atomicity explanation  
**Rationale**: Current docs mention rotation but lack implementation details and race condition handling  
**Implementation**:

```rust
// Atomic rotation pattern:
// 1. Client sends: (refresh_token, code)
// 2. Server validates refresh_token
// 3. If valid:
//    - Issue new access_token
//    - Issue new refresh_token
//    - Mark old refresh_token as INVALID (atomic operation)
//    - Return both new tokens
// 4. If old token reused: Detect possible theft, revoke all tokens
```

**Documentation Requirements**:
- Show atomic operation pattern (prevents race conditions)
- Document database lock example
- Explain timeout handling (what if token not used within 30 seconds)
- Cover concurrency: What happens when 2 requests try to refresh simultaneously
- Document expiration timeline: Access (15-30 min), Refresh (7-14 days), Auth Code (~10 min)

---

### 2.5 Provider-Specific Documentation

**Decision**: Create comparison table and provider-specific guides for GitHub, Google, AWS Cognito  
**Rationale**: Each provider has unique quirks that cause common user errors  

| Aspect | GitHub | Google | AWS Cognito |
|--------|--------|--------|-------------|
| **Authorization Code TTL** | 10 minutes (expires quickly) | ~10 minutes | Configurable |
| **PKCE Support** | Recommended but optional | Recommended | Supported |
| **Offline Access** | N/A | Requires `access_type=offline` | N/A |
| **Scope Separator** | Space-separated | Space-separated | Space-separated |
| **Refresh Token Rotation** | New token on refresh | Not always issued | Configurable |
| **Incremental Auth** | Not supported | Supported (request scopes incrementally) | Not supported |

**Documentation Requirements**:
- Create table comparing 3-5 major providers
- Document each provider's quirks
- Show provider-specific example code
- Link to provider official documentation

---

### 2.6 Sequence Diagrams - Mermaid Visualization

**Decision**: Add Mermaid sequence diagrams for OAuth flows  
**Rationale**: Visual representation significantly improves comprehension; Mermaid renders natively in Obsidian  

```mermaid
sequenceDiagram
    participant User as Resource Owner
    participant Client as Your App
    participant AuthServer as OAuth Provider
    participant ResourceServer as API Server
    
    User->>Client: Click "Login with Provider"
    Client->>AuthServer: GET /authorize?client_id=...&state=...&code_challenge=...
    AuthServer->>User: Show consent screen
    User->>AuthServer: Grant permission
    AuthServer->>Client: Redirect ?code=AUTH_CODE&state=...
    Client->>Client: Verify state parameter
    Client->>AuthServer: POST /token (code, code_verifier, client_secret)
    AuthServer->>Client: Return access_token, refresh_token
    Client->>ResourceServer: GET /api/user (Bearer token)
    ResourceServer->>Client: Return protected resource
```

**Documentation Requirements**:
- Create diagrams for: Authorization code flow, PKCE variant, Token refresh, Error scenarios
- Include parameter names in diagrams
- Show HTTP methods (GET/POST)
- Document token lifecycle timeline

**Sources**:
- [RFC 6749 - OAuth 2.0 Authorization Framework](https://datatracker.ietf.org/doc/html/rfc6749)
- [RFC 7636 - PKCE](https://datatracker.ietf.org/doc/html/rfc7636)
- [RFC 9700 - OAuth 2.0 Security Best Practices](https://datatracker.ietf.org/doc/rfc9700/)
- [GitHub OAuth2 Docs](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps)
- [Google OAuth2 Docs](https://developers.google.com/identity/protocols/oauth2)
- [PortSwigger - OAuth Vulnerabilities](https://portswigger.net/web-security/oauth)

---

## Part 3: Cloud Provider Integration Patterns

### 3.1 Common Setup Flow Across Providers

**Decision**: Document unified setup pattern applicable to all providers  
**Rationale**: Despite implementation differences, setup steps are conceptually similar  

**Universal Setup Pattern**:
```
1. Create/Deploy Secret Management System
2. Configure Authentication (IAM/Managed Identity/RBAC/AppRole)
3. Define Access Policies/Roles
4. Establish Network Security (firewall/VPC/private endpoints)
5. Enable Audit Logging
6. Test Access from Applications
7. Implement Rotation Strategy
8. Set up Alerting/Monitoring
```

---

### 3.2 AWS Secrets Manager

**Decision**: Document IAM-based authentication with rotation lambdas  
**Rationale**: AWS-native integration for AWS-heavy deployments  

**Key Patterns**:
- **Authentication**: IAM users/roles with access keys or assumed roles
- **Rotation**: Lambda functions with staged rotation (Create→Set→Test→Finish)
- **Permissions**: Identity-based policies, resource-based policies, ABAC (attribute-based)
- **Security**: Custom KMS keys (not AWS-managed), CloudTrail logging, VPC endpoints

**Common Issues**:
- Lambda concurrency below 10 causing throttling
- Missing KMS decrypt permissions
- VPC Lambda without private endpoints

**Documentation Requirements**:
- Show IAM policy examples for read/write/rotate
- Document Lambda rotation function structure
- Provide CloudWatch log debugging guide
- Show VPC endpoint configuration

---

### 3.3 HashiCorp Vault

**Decision**: Document lease-based dynamic secrets and transit engine  
**Rationale**: Multi-cloud solution with strongest dynamic secret support  

**Key Patterns**:
- **Lease System**: All dynamic secrets have TTLs, consumers must renew before expiration
- **Transit Engine**: Encryption-as-a-service (zero-knowledge rotation via data rewrapping)
- **Dynamic Secrets**: On-demand credential generation for databases, cloud providers, PKI
- **AppRole Auth**: Preferred over user tokens for applications

**Key Rotation Pattern**:
- Versioned keys allow new encryptions with latest version
- Old ciphertexts remain decryptable
- Rewrap operation upgrades ciphertexts to newer versions without plaintext exposure

**Documentation Requirements**:
- Show lease renewal workflow
- Document transit engine encrypt/decrypt flow
- Provide dynamic database secret example
- Show AppRole authentication setup

---

### 3.4 Azure Key Vault

**Decision**: Document managed identity authentication with RBAC  
**Rationale**: Azure-native integration eliminates hardcoded credentials  

**Key Patterns**:
- **Managed Identities**: System-assigned (1:1 resource) or user-assigned (shared)
- **RBAC with PIM**: Just-in-time role activation with MFA
- **Network Security**: Private endpoints + disabled public access (most secure)
- **Autorotation**: Event Grid publishes "near expiry" event → Function App rotation

**What NOT to Store**:
- Customer configurations (use Azure App Configuration)
- Large customer data (use Azure Storage/Cosmos DB)
- Certificates as secrets (use Key Vault certificate objects)

**Documentation Requirements**:
- Show managed identity setup (system vs user-assigned)
- Document RBAC role assignments
- Provide Event Grid + Function App rotation example
- Show private endpoint configuration

---

### 3.5 Kubernetes Secrets

**Decision**: Document External Secrets Operator (ESO) as recommended pattern  
**Rationale**: Native K8s secrets have security issues (base64 encoding, not encryption); ESO delegates to external providers  

**Key Patterns**:
- **Native Issues**: Base64 encoding (not encryption), no built-in rotation, minimal auditing
- **ESO**: Synchronizes external stores (AWS, Vault, Azure) into K8s secrets
- **Sealed Secrets**: Asymmetric encryption for git-friendly storage (alternative pattern)

**Security Best Practices**:
- Namespace isolation (prevent cross-namespace SecretStore references)
- Pod-level RBAC (fine-grained secret access)
- NetworkPolicies (restrict outbound traffic)
- Encrypted etcd storage (must be configured)

**Documentation Requirements**:
- Show ESO ExternalSecret and SecretStore CRD examples
- Document RBAC configuration
- Provide migration guide from native secrets to ESO
- Show Sealed Secrets alternative pattern

---

### 3.6 Provider Comparison Matrix

**Decision**: Create comparison table for quick reference  
**Rationale**: Users need to select provider based on environment and requirements  

| Feature | AWS Secrets Manager | HashiCorp Vault | Azure Key Vault | Kubernetes Secrets |
|---------|---------------------|-----------------|-----------------|-------------------|
| **Cloud-Agnostic** | AWS-only | Yes | Azure-only | Any cloud |
| **Dynamic Secrets** | No | Yes | Limited | No (ESO integration) |
| **Automatic Rotation** | Lambda-based | Lease expiration | Event Grid | ESO-dependent |
| **Encryption** | KMS mandatory | Configurable | Built-in | etcd optional |
| **Auth Method** | IAM | AppRole/LDAP/K8s/JWT | Managed Identity | ServiceAccount |
| **Network Isolation** | VPC endpoints | Firewall | Private endpoints | NetworkPolicies |
| **Audit Logging** | CloudTrail | Native | Azure Monitor | Standard K8s audit |
| **Learning Curve** | Steep | High | Medium | Low (ESO adds complexity) |

---

### 3.7 Migration Strategy

**Decision**: Recommend orchestration/broker pattern with phased rollout  
**Rationale**: Minimizes disruption, maintains investment in existing infrastructure  

**Recommended Approach**:
1. **Phase 1**: Pilot with non-critical secrets (test provider)
2. **Phase 2**: Expand to production services (validate at scale)
3. **Phase 3**: Migrate remaining secrets (complete transition)
4. **Phase 4**: Sunset legacy system (decommission)

**Key Considerations**:
- Document secret inventory (map all secrets across systems)
- Secure migration path (never expose secrets during transfer)
- Key management (migrate encryption keys safely)
- Rollback plan (maintain access to legacy system during transition)

**Documentation Requirements**:
- Provide step-by-step migration checklist
- Show dual-write pattern (write to both systems during transition)
- Document validation procedures
- Provide rollback procedures

**Sources**:
- [AWS Secrets Manager - Auth & Access Control](https://docs.aws.amazon.com/secretsmanager/latest/userguide/auth-and-access.html)
- [HashiCorp Vault - Transit Engine](https://developer.hashicorp.com/vault/docs/secrets/transit)
- [Azure Key Vault - Security Best Practices](https://learn.microsoft.com/en-us/azure/key-vault/general/secure-key-vault)
- [External Secrets Operator - Security](https://external-secrets.io/latest/guides/security-best-practices/)
- [Kubernetes Secrets Management Guide](https://auth0.com/blog/kubernetes-secrets-management/)

---

## Research Conclusions

### Critical Documentation Gaps Identified

1. ✅ **Nonce Management in Encryption** - MUST add prominent warning about GCM nonce reuse
2. ✅ **PKCE in OAuth2** - MUST document PKCE flow (currently missing)
3. ✅ **Redirect URI Validation** - MUST show exact matching pattern
4. ✅ **Token Refresh Atomicity** - MUST explain race conditions and database locks
5. ✅ **Provider Comparison** - MUST create comparison matrix for selection guidance
6. ✅ **Sequence Diagrams** - MUST add Mermaid visualizations for OAuth flows

### Documentation Structure Decisions

**Confirmed from Research**:
- Progressive disclosure works: Quick Start → Common Patterns → Advanced Topics
- Code examples MUST be complete and runnable (no pseudocode)
- Security sections MUST use warning callouts for critical issues
- Provider-specific guides essential (quirks cause user errors)
- Visual aids (Mermaid) significantly improve comprehension

### Content Priorities

**P1 (Critical for MVP)**:
- Quick Start tutorial with working example
- OAuth2 complete flow with PKCE
- Database credential with connection pooling
- AWS credential with AssumeRole
- Encryption security (nonce management)

**P2 (Production Readiness)**:
- Provider integration guides (AWS, Vault, Azure, K8s)
- Credential rotation patterns
- Troubleshooting common errors
- API reference documentation

**P3 (Compliance & Advanced)**:
- Security hardening guide
- Custom provider implementation
- Performance tuning
- Audit logging setup

---

## Part 4: Enterprise Authentication Methods

### 4.1 SAML (Security Assertion Markup Language) Authentication

**Decision**: Document SAML 2.0 as primary enterprise SSO protocol for legacy/regulated industries  
**Rationale**: Required for Active Directory integration, compliance requirements (SOC2, HIPAA), B2B enterprise partnerships  

**Key Components**:
- **Identity Provider (IdP)**: Okta, Azure AD, OneLogin, Ping Identity, Auth0
- **Service Provider (SP)**: Your application (Nebula)
- **SAML Assertions**: XML documents containing authentication/authorization statements
- **Bindings**: HTTP-POST (responses), HTTP-Redirect (requests)
- **Metadata Exchange**: XML configuration files for IdP/SP setup

**Authentication Flow (SP-Initiated)**:
```
1. User → SP: Access protected resource
2. SP → IdP: AuthnRequest (HTTP Redirect)
3. User → IdP: Authenticate (login form)
4. IdP → SP: SAML Response with signed assertion (HTTP POST)
5. SP validates assertion → Grant access
```

**Critical Security Validations**:
1. **XML Signature**: Verify using IdP public certificate
2. **Timestamps**: Check `NotBefore` and `NotOnOrAfter` (allow 60s clock skew)
3. **Audience**: Verify matches SP Entity ID
4. **Recipient**: Check ACS URL matches
5. **InResponseTo**: Match against pending request ID
6. **Replay Protection**: Store assertion IDs with expiry (Redis/database)

**Rust Implementation**:
- **Library**: `samael` (most actively maintained Rust SAML library)
- **Key Features**: Service Provider functionality, XML signature validation, async support
- **Dependencies**: `samael`, `openssl`, `chrono`, `uuid`, `base64`

**Common Issues Documented**:
- Certificate expiration (30-day monitoring recommended)
- Clock skew between SP/IdP (configure NTP sync)
- DN formatting in XML
- Attribute mapping inconsistencies across IdPs
- URL encoding issues with RelayState

**Provider Comparison Table**:
| Provider | Auth Code TTL | PKCE Support | Offline Access | Refresh Token Rotation |
|----------|---------------|--------------|----------------|------------------------|
| Okta | 10 min | Recommended | Via scope | Configurable |
| Azure AD | ~10 min | Supported | N/A | Automatic |
| Auth0 | Configurable | Recommended | Via param | Configurable |
| OneLogin | 10 min | Optional | Via scope | Not always |

**Documentation Requirements**:
- Step-by-step setup guide (certificate generation, metadata exchange, IdP configuration)
- Mermaid sequence diagrams for SP-initiated, IdP-initiated, and Single Logout flows
- Certificate rotation procedures with monitoring
- Troubleshooting guide for common errors
- Provider-specific quirks and configurations

---

### 4.2 LDAP (Lightweight Directory Access Protocol) Authentication

**Decision**: Document LDAP/Active Directory integration for enterprise directory services  
**Rationale**: Widely deployed in enterprises for centralized user management, Windows domain integration, legacy system compatibility  

**Core LDAP Concepts**:
- **DN (Distinguished Name)**: Unique identifier `cn=John Smith,ou=IT,dc=example,dc=com`
- **Bind Operation**: Authentication method (Simple Bind, SASL)
- **Search Filters**: RFC 4515 syntax `(&(objectClass=person)(uid=jsmith))`
- **Connection Security**: StartTLS (port 389) or LDAPS (port 636)

**Authentication Patterns**:
1. **Simple Bind**: DN + password (requires TLS)
2. **Anonymous Bind**: ⚠️ Security risk - must disable in production
3. **Unauthenticated Bind**: ⚠️ Critical vulnerability with empty passwords
4. **SASL Bind**: GSSAPI/Kerberos, EXTERNAL (certificate), DIGEST-MD5

**Active Directory Specific**:
- Use `sAMAccountName` for usernames (not `uid`)
- Group membership in `memberOf` attribute
- Nested groups: Use `LDAP_MATCHING_RULE_IN_CHAIN` (OID 1.2.840.113556.1.4.1941)
- Common attributes: `userPrincipalName`, `displayName`, `mail`, `department`

**Rust Implementation**:
- **Library**: `ldap3` (primary), `deadpool-ldap` (connection pooling)
- **Features**: Async/sync APIs, SASL support, TLS/LDAPS
- **Dependencies**: `ldap3`, `tokio`, `deadpool-ldap` (production)

**User Authentication Pattern**:
```rust
// 1. Bind with service account
ldap.simple_bind("cn=service,dc=example,dc=com", "password").await?;

// 2. Search for user DN
let filter = format!("(&(objectClass=person)(uid={}))", username);
let (results, _) = ldap.search(base_dn, Scope::Subtree, &filter, vec!["dn"]).await?;
let user_dn = results[0].dn;

// 3. Rebind as user to verify password
ldap.simple_bind(&user_dn, user_password).await?;
```

**Connection Pooling**:
```rust
use deadpool_ldap::{Manager, Pool};

let pool = Pool::builder(manager)
    .max_size(10)
    .wait_timeout(Some(Duration::from_secs(5)))
    .build()?;
```

**Common Issues**:
- Connection timeouts (increase timeout, check firewall)
- Certificate validation errors (add CA to trust store, verify hostname)
- DN formatting (escape special characters: `,` `+` `"` `\` `<` `>` `;`)
- Group nesting performance (cache memberships, use AD native resolution)

**Security Requirements**:
- TLS/LDAPS mandatory for production
- Disable anonymous and unauthenticated binds
- Store service account credentials in secrets manager
- Implement connection pooling securely (separate user/service pools)
- Rate limiting to prevent brute force
- Input validation to prevent LDAP injection

**Documentation Requirements**:
- Connection string format and TLS configuration
- Authentication workflow examples
- Active Directory integration guide
- Search filter syntax and examples
- Troubleshooting common issues
- Performance optimization for nested groups

---

### 4.3 Auth0 Integration

**Decision**: Document Auth0 as Identity-as-a-Service (IDaaS) platform for modern authentication  
**Rationale**: Comprehensive authentication solution, Universal Login, social providers, MFA, enterprise connections, no Rust SDK (use OAuth2 libraries)  

**Key Features**:
- **Universal Login**: Hosted authentication page (recommended)
- **Social Login**: 30+ providers (Google, GitHub, Facebook, etc.)
- **MFA**: Push notifications, SMS, TOTP, WebAuthn
- **Rules/Actions**: Extensible authentication flows (Rules EOL: Nov 18, 2026)
- **Management API**: RESTful API for tenant configuration

**Authentication Flows**:
1. **Universal Login** (recommended): Redirect to Auth0 hosted page
2. **Embedded Login** (legacy): Custom login form in application
3. **Machine-to-Machine** (M2M): Client credentials for service-to-service

**Token Types**:
- **Access Token**: API access, short-lived (hours-days), JWT or opaque
- **Refresh Token**: Long-lived, obtain new access tokens, rotation supported
- **ID Token**: User identity claims, always JWT

**Rust Integration**:
- **Libraries**: `oauth2-rs` (primary), `oauth-kit` (90+ providers pre-configured), `openidconnect-rs` (OIDC)
- **Code Samples**: Official Actix-web examples available
- **Dependencies**: `oauth2`, `reqwest`, `tokio`, `jsonwebtoken`

**Token Validation**:
```rust
// 1. Decode header to get 'kid'
let header = decode_header(token)?;

// 2. Fetch JWKS and find matching key
let jwks = fetch_jwks(jwks_url).await?;
let jwk = jwks.find_key(&header.kid)?;

// 3. Create decoding key from RSA components
let decoding_key = DecodingKey::from_rsa_components(&jwk.n, &jwk.e)?;

// 4. Set validation parameters
let mut validation = Validation::new(Algorithm::RS256);
validation.set_issuer(&["https://your-tenant.auth0.com/"]);
validation.set_audience(&["your_client_id"]);

// 5. Decode and validate
let token_data = decode::<Claims>(token, &decoding_key, &validation)?;
```

**Refresh Token Rotation**:
Auth0 supports automatic rotation:
- New access token + new refresh token issued on refresh
- Old refresh token immediately invalidated
- Token family tracking to detect reuse
- Entire family invalidated on suspicious activity

**Configuration Requirements**:
- **Tenant**: Create at auth0.com, note domain and region
- **Application**: Register in dashboard, get Client ID/Secret
- **Allowed Callback URLs**: `https://app.example.com/callback` (critical)
- **Allowed Logout URLs**: `https://app.example.com`
- **Allowed Web Origins (CORS)**: For embedded login
- **API**: Define identifier (audience), scopes/permissions

**Custom Claims** (via Actions):
```javascript
// Auth0 Action (JavaScript)
exports.onExecutePostLogin = async (event, api) => {
  const namespace = 'https://your-app.com/';
  api.idToken.setCustomClaim(`${namespace}roles`, event.user.roles);
  api.accessToken.setCustomClaim(`${namespace}permissions`, event.authorization?.permissions);
};
```

**Security Best Practices**:
- Use HTTPS for all callbacks
- Implement PKCE for OAuth2 flows
- Enable refresh token rotation
- Configure appropriate token lifetimes
- Set up MFA for privileged users
- Enable breach password detection
- Monitor authentication events

**Documentation Requirements**:
- Tenant and application setup guide
- OAuth2/OIDC integration examples
- Token management and validation
- Refresh token rotation patterns
- Custom claims and metadata
- MFA configuration
- CORS setup for SPAs

---

### 4.4 SSE (Server-Sent Events) for Real-Time Notifications

**Clarification**: SSE is NOT an authentication protocol - it's for real-time server-to-client communication  

**Use Cases for Nebula**:
- **Credential Rotation Notifications**: Alert clients when credentials rotated
- **Expiration Warnings**: Notify before credentials expire
- **Interaction Required**: Prompt user action for OAuth re-authorization
- **Live Dashboards**: Real-time credential status updates

**SSE vs WebSocket**:
| Feature | SSE | WebSocket |
|---------|-----|-----------|
| Direction | Server → Client only | Bidirectional |
| Protocol | HTTP/HTTPS | WebSocket (ws://, wss://) |
| Browser API | Native EventSource | Native WebSocket |
| Reconnection | Automatic | Manual |
| Message Format | Text (JSON) | Binary or Text |
| Overhead | Lower | Higher |

**Authentication Challenges**:
EventSource API doesn't support custom headers (`Authorization`).

**Workarounds**:
1. **Query Parameter** (less secure): `/api/events?token={token}`
2. **Cookies** (recommended): `withCredentials: true` + HttpOnly cookies
3. **Short-Lived SSE Tokens**: Exchange auth token for temporary SSE token

**Rust Implementation (Axum)**:
```rust
use axum::response::sse::{Event, Sse};
use futures_util::stream::Stream;

async fn sse_handler() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::channel(100);
    
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let event = Event::default()
                .data("credential_rotated")
                .event("rotation_complete");
            if tx.send(Ok(event)).await.is_err() {
                break; // Client disconnected
            }
        }
    });
    
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

**Credential Events Pattern**:
```rust
enum CredentialEvent {
    RotationScheduled { credential_id: String, scheduled_at: DateTime<Utc> },
    RotationComplete { credential_id: String, rotated_at: DateTime<Utc> },
    RotationFailed { credential_id: String, error: String },
    ExpirationWarning { credential_id: String, expires_at: DateTime<Utc> },
    InteractionRequired { credential_id: String, reason: String, action_url: String },
}
```

**Client-Side**:
```javascript
const eventSource = new EventSource('/api/credentials/events', {
    withCredentials: true
});

eventSource.addEventListener('rotation_complete', (event) => {
    const data = JSON.parse(event.data);
    console.log(`Credential ${data.credential_id} rotated`);
});

eventSource.addEventListener('interaction_required', (event) => {
    const data = JSON.parse(event.data);
    showNotification({ message: data.reason, action: data.action_url });
});
```

**Security Best Practices**:
- Validate user identity before streaming
- Check authorization for credential access
- Use session/token authentication
- Implement CSRF protection
- Sanitize event data (XSS prevention)
- Rate limit connections per user
- Set maximum message size
- Use connection timeouts

**Documentation Requirements**:
- SSE authentication patterns
- Credential event types
- Server implementation (Axum/Actix-web)
- Client integration examples
- Security considerations
- Error handling and reconnection

---

### 4.5 Certificate-Based Authentication (mTLS)

**Decision**: Document mTLS for high-security service-to-service authentication  
**Rationale**: Zero-trust architectures, IoT devices, regulatory compliance (PCI-DSS, HIPAA)  

**Critical 2026 Update**:
⚠️ Starting **June 15, 2026**, public CAs will NOT issue SSL certificates with Client Authentication EKU.

**Impact**:
- Must use private CAs for mTLS client certificates after June 2026
- Transition to private CA infrastructure before May 2026
- Alternative: Device identity solutions, other authentication methods

**mTLS Overview**:
- Both client and server present X.509 certificates
- Mutual verification of identity
- Standard TLS: Only server presents certificate
- mTLS: Client AND server present certificates

**Use Cases**:
- Service-to-service authentication
- IoT device authentication
- High-security APIs
- Zero-trust network architectures

**Rust Implementation**:
- **Libraries**: `rustls`, `tokio-rustls`, `rcgen` (certificate generation), `x509-parser`
- **Server**: Configure `ClientCertVerifier` to validate client certificates
- **Client**: Load client certificate/key with `Identity`

**Certificate Structure**:
```
Certificate:
    Version: 3
    Serial Number: 4096
    Signature Algorithm: sha256WithRSAEncryption
    Issuer: CN=My Private CA
    Validity:
        Not Before: Jan  1 00:00:00 2026 GMT
        Not After : Dec 31 23:59:59 2026 GMT
    Subject: CN=client.example.com
    X509v3 Extended Key Usage:
        TLS Web Client Authentication
```

**Server-Side Validation**:
```rust
impl ClientCertVerifier for ClientAuth {
    fn verify_client_cert(&self, end_entity: &Certificate, ...) -> Result<...> {
        // Validate certificate chain
        // Verify signed by trusted CA
        // Check validity period
        // Verify not revoked (CRL/OCSP)
        Ok(rustls::ServerCertVerified::assertion())
    }
}
```

**Client-Side**:
```rust
let client_cert = fs::read("client.crt")?;
let client_key = fs::read("client.key")?;
let identity = Identity::from_pem(&[client_cert, client_key].concat())?;

let client = reqwest::Client::builder()
    .identity(identity)
    .add_root_certificate(ca)
    .build()?;
```

**Certificate Lifecycle Management**:
- **Generation**: Use `rcgen` for self-signed/private CA
- **Storage**: Encrypt at rest, strict file permissions (0600)
- **Rotation**: 30-day warning, automated renewal
- **Revocation**: CRL or OCSP for compromised certificates

**Documentation Requirements**:
- mTLS concept explanation
- Certificate generation guide
- Server/client implementation
- Certificate lifecycle management
- Private CA setup (post-June 2026)
- Security best practices
- Troubleshooting certificate errors

---

### 4.6 API Key Management

**Decision**: Document modern API key patterns with automatic rotation  
**Rationale**: Simplest authentication for APIs, third-party integrations, CLI tools, webhooks  

**API Key Patterns**:
1. **Simple Header**: `X-API-Key: sk_live_51H8F9AKj...`
2. **HMAC Signature**: Key + Secret with request signing
3. **JWT-based**: Self-contained tokens with permissions

**Modern Best Practices (2026)**:
- **Rotation Schedule**: 90 days for moderate-risk, 30 days for high-risk
- **Zero-Downtime Rotation**: 24-hour overlap period for old/new keys
- **Secure Storage**: Hash keys (Argon2), encrypt backups, store prefix only
- **Monitoring**: Track usage patterns, detect anomalies, alert on suspicious activity

**Zero-Downtime Rotation**:
```rust
pub async fn rotate_key(&self, key_id: &str) -> Result<(String, String), Error> {
    // 1. Generate new key
    let new_key = self.generate_new_key(key_id).await?;
    
    // 2. Store new key (both keys valid)
    self.store_key(&new_key).await?;
    
    // 3. Grace period: Old key remains valid for 24 hours
    tokio::spawn(async move {
        tokio::time::sleep(Duration::hours(24).to_std().unwrap()).await;
        self.revoke_old_key(key_id).await;
    });
    
    Ok((new_key.key_id, new_key.secret))
}
```

**Secure Storage**:
```rust
pub struct ApiKeyStorage {
    pub key_hash: String,           // Argon2 hash for validation
    pub key_prefix: String,          // First 8 chars for identification
    pub encrypted_key: Vec<u8>,      // Encrypted full key (recovery)
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    pub usage_count: u64,
}
```

**Validation with Timing-Attack Protection**:
```rust
// Use constant-time comparison
use argon2::{Argon2, PasswordVerifier, PasswordHash};

let argon2 = Argon2::default();
let hash = PasswordHash::new(&stored_key.key_hash)?;
argon2.verify_password(provided_key.as_bytes(), &hash)
    .map_err(|_| Error::InvalidKey)?;
```

**AWS Secrets Manager Integration**:
```rust
// Store API keys in Secrets Manager
self.client
    .update_secret()
    .secret_id(secret_name)
    .secret_string(&new_key)
    .send()
    .await?;

// Automatic rotation with versioning
// Applications get new key on next fetch
```

**Rate Limiting & Quotas**:
```rust
pub struct ApiKeyQuota {
    pub requests_per_minute: u32,
    pub requests_per_day: u32,
    pub concurrent_requests: u32,
}
```

**Documentation Requirements**:
- API key patterns comparison
- Generation and rotation procedures
- Secure storage architecture
- Validation implementation
- AWS Secrets Manager integration
- Rate limiting strategies
- Monitoring and alerting

---

### 4.7 SSO (Single Sign-On) Protocol Comparison

**Decision**: Document both SAML and OIDC for enterprise SSO  
**Rationale**: 72% of enterprises use multi-protocol SSO (both SAML for legacy, OIDC for modern apps)  

**SAML vs OIDC**:
| Aspect | SAML | OIDC |
|--------|------|------|
| Format | XML | JSON |
| Transport | HTTP POST/Redirect | REST API |
| Token Format | XML assertions | JWT |
| Mobile Support | Poor | Excellent |
| API Support | Limited | Excellent |
| Complexity | High | Lower |
| Enterprise Adoption | Very High | Growing |
| Legacy Systems | Excellent | Limited |
| Single Logout | Robust | Limited |

**When to Use SAML**:
- Enterprise SSO with Active Directory
- Compliance requirements (SOC2, HIPAA)
- B2B integrations with enterprise partners
- Legacy system integration

**When to Use OIDC**:
- Modern cloud-native applications
- Mobile and SPA applications
- API-first architectures
- Microservices authentication

**Enterprise SSO Providers**:
- **Auth0**: SAML + OIDC support
- **Okta**: Comprehensive enterprise platform
- **Microsoft Entra ID** (Azure AD): Deep Microsoft integration
- **OneLogin**: User-friendly, mid-sized businesses
- **PingIdentity**: Enterprise-grade, regulated industries

**Rust OIDC Implementation**:
```rust
use openidconnect::{
    core::{CoreClient, CoreProviderMetadata},
    IssuerUrl, ClientId, ClientSecret, RedirectUrl,
};

// Discover OIDC configuration
let provider_metadata = CoreProviderMetadata::discover_async(
    IssuerUrl::new("https://accounts.google.com".to_string())?,
    async_http_client,
).await?;

// Create client
let client = CoreClient::from_provider_metadata(
    provider_metadata,
    ClientId::new(env::var("CLIENT_ID")?),
    Some(ClientSecret::new(env::var("CLIENT_SECRET")?)),
)
.set_redirect_uri(RedirectUrl::new("http://localhost:8080/callback".to_string())?);
```

**Documentation Requirements**:
- SAML vs OIDC comparison matrix
- Use case recommendations
- Provider comparison
- Implementation examples (both protocols)
- Token validation patterns
- Single logout flows

---

### 4.8 Kerberos Authentication

**Decision**: Document Kerberos for Windows domain integration  
**Rationale**: Integrated Windows authentication, Windows Active Directory environments, Microsoft Entra Kerberos (2026 cloud support)  

**Kerberos Overview**:
- Ticket-based authentication protocol
- Mutual authentication (client + server)
- Single sign-on for domain resources
- Integrated with Windows logon

**Authentication Flow**:
```
1. User → KDC: Authentication Request
2. KDC → User: TGT (Ticket Granting Ticket)
3. User → TGS: Service Ticket Request + TGT
4. TGS → User: Service Ticket
5. User → Service: Service Request + Service Ticket
6. Service → User: Access Granted
```

**Microsoft Entra Kerberos (2026)**:
- Cloud-only identities can use Kerberos
- No domain controllers required for cloud resources
- Windows Hello for Business integration
- Azure Virtual Desktop, Azure Files support

**Rust Implementation**:
Limited native support - practical approaches:
1. FFI to system Kerberos libraries (GSSAPI)
2. Delegate to system authentication (SSPI on Windows, PAM on Linux)
3. Authentication proxy/gateway pattern

**Documentation Requirements**:
- Kerberos concept overview
- Windows domain integration
- Microsoft Entra Kerberos setup
- Browser-based SSO configuration
- Rust integration strategies

---

## Authentication Method Selection Guide

| Use Case | Recommended Method | Notes |
|----------|-------------------|-------|
| User authentication (web) | Auth0 with Universal Login | Best UX, security, features |
| User authentication (mobile) | Auth0 with OIDC | Native mobile support |
| Service-to-service | mTLS or JWT | High security, no user interaction |
| Third-party API access | OAuth2 with scoped tokens | Standard, delegated authorization |
| Enterprise SSO (legacy) | SAML 2.0 | Required for AD integration |
| Enterprise SSO (modern) | OIDC | Better DX, cloud-native |
| Windows domain | Kerberos | Integrated Windows auth |
| Simple API auth | API Key with rotation | Balance of security and simplicity |
| High-security APIs | mTLS + JWT | Defense in depth |
| Directory services | LDAP/Active Directory | Centralized user management |
| Real-time notifications | SSE | Credential rotation alerts |

---

## Research Conclusions (Updated)

### Critical Documentation Gaps Identified

1. ✅ **Nonce Management in Encryption** - MUST add prominent warning about GCM nonce reuse
2. ✅ **PKCE in OAuth2** - MUST document PKCE flow (currently missing)
3. ✅ **Redirect URI Validation** - MUST show exact matching pattern
4. ✅ **Token Refresh Atomicity** - MUST explain race conditions and database locks
5. ✅ **Provider Comparison** - MUST create comparison matrix for selection guidance
6. ✅ **Sequence Diagrams** - MUST add Mermaid visualizations for OAuth flows
7. ✅ **SAML Enterprise Integration** - MUST document SAML 2.0 for enterprise SSO
8. ✅ **LDAP/Active Directory** - MUST document directory service integration
9. ✅ **Auth0 IDaaS Platform** - MUST document modern authentication platform
10. ✅ **mTLS Certificate Auth** - MUST document with 2026 public CA restrictions
11. ✅ **API Key Rotation** - MUST document zero-downtime rotation patterns
12. ✅ **SSE for Notifications** - MUST document real-time credential events

### Documentation Structure Decisions (Updated)

**Confirmed from Research**:
- Progressive disclosure works: Quick Start → Common Patterns → Advanced Topics
- Code examples MUST be complete and runnable (no pseudocode)
- Security sections MUST use warning callouts for critical issues
- Provider-specific guides essential (quirks cause user errors)
- Visual aids (Mermaid) significantly improve comprehension
- **NEW**: Multi-protocol support essential (SAML + OIDC + LDAP + OAuth2)
- **NEW**: Enterprise authentication patterns require dedicated sections
- **NEW**: Real-time notification patterns (SSE) for credential lifecycle

### Content Priorities (Updated)

**P1 (Critical for MVP)**:
- Quick Start tutorial with working example
- OAuth2 complete flow with PKCE
- Database credential with connection pooling
- AWS credential with AssumeRole
- Encryption security (nonce management)
- **NEW**: Auth0 integration guide
- **NEW**: Basic API key management

**P2 (Production Readiness)**:
- Provider integration guides (AWS, Vault, Azure, K8s)
- Credential rotation patterns
- Troubleshooting common errors
- API reference documentation
- **NEW**: SAML 2.0 enterprise SSO
- **NEW**: LDAP/Active Directory integration
- **NEW**: mTLS certificate authentication
- **NEW**: SSE real-time notifications

**P3 (Compliance & Advanced)**:
- Security hardening guide
- Custom provider implementation
- Performance tuning
- Audit logging setup
- **NEW**: Kerberos Windows domain integration
- **NEW**: Advanced API key rotation strategies
- **NEW**: Multi-protocol SSO architecture

---

## Next Steps

1. ✅ **Phase 0 Complete** - Research findings consolidated
2. **Phase 1** - Create data model (documentation page structure, frontmatter schema)
3. **Phase 1** - Generate contracts (page templates for Quick Start, How-To, Examples, Troubleshooting)
4. **Phase 1** - Create quickstart validation workflow
5. **Phase 2** - Generate tasks.md (via `/speckit.tasks`)

---

**Research Completed**: 2026-02-03  
**Agents Used**: 6 parallel exploration agents (Rust libraries, OAuth2, Cloud providers, SAML, LDAP, Auth0/SSE)  
**Total Sources**: 80+ authoritative sources (RFCs, official docs, security guides, enterprise IdP documentation)
