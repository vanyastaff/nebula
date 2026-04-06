# nebula-credential v3 — Design Spec

## Goal

Redesign nebula-credential with security-first approach, open AuthScheme trait, 12 universal auth patterns as building blocks, clean separation of secret material from transport concerns, and tight integration with nebula-parameter for credential configuration UI.

## Philosophy

- **Credential = Parameters → AuthScheme transformer.** User fills a form (parameters), credential resolves it into secret material (scheme). Transport injection is NOT credential's concern.
- **AuthScheme = open trait.** 12 built-in patterns cover common cases. `AuthPattern::Custom` + open trait covers everything else. Plugins add protocol-specific types.
- **Security-first.** Encryption key rotation, hard AAD enforcement (no legacy fallback), `Zeroizing<Vec<u8>>` for all plaintext buffers. No shortcuts.
- **Three authoring levels.** Derive macro (5 lines), composition (extends, 1 level max), manual impl (full control).
- **Two key types.** `CredentialKey` = type identity (compile-time, e.g. `"stripe"`). `CredentialId` = instance identity (runtime UUID).

## Post-Review Amendments

Amendments from agent review (security lead, architect, SDK user):

1. **DatabaseAuth removed from built-in schemes.** host/port/db/ssl are resource concern. Credential provides `IdentityPassword`; database plugin extends it with connection config.
2. **Field-to-scheme mapping explicit.** `#[credential(into = "field")]` attribute maps struct fields to scheme fields. Compile error on incomplete mapping.
3. **AWS/SSH Agent → Custom schemes via open trait.** 12 built-in patterns are convenience, not limitation.
4. **CredentialKey + CredentialId both kept.** Type key (compile-time) vs instance key (runtime UUID).
5. **AAD legacy fallback removed entirely.** One-time migration re-encrypts all records. No `legacy_compat` flag.
6. **Zeroization uses `Zeroizing<Vec<u8>>`**, not manual `.zeroize()` on clones.
7. **`extends` limited to 1 level** of delegation. No multi-level chain.
8. **OAuth2 `token_url` must be HTTPS.** Validated at macro expansion time.
9. **Plugin AuthScheme deserialization sandboxed.** Core store never deserializes plugin types directly.
10. **Rate limiting on `continue_resolve()`.** Framework enforces per-session rate limit.
11. **Size limits on deserialized CredentialState.** Prevents OOM from corrupted records.

---

## 1. AuthScheme — Open Trait

### 1.1 The Trait

```rust
/// Marker trait for authentication material.
///
/// Implemented by all auth scheme types. The credential crate provides
/// 12 built-in types; plugins add their own.
pub trait AuthScheme: Send + Sync + Debug + Clone + Serialize + DeserializeOwned + 'static {
    /// Classification for UI, logging, and tooling.
    fn pattern() -> AuthPattern;
}
```

### 1.2 AuthPattern — Classification Enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AuthPattern {
    SecretToken,
    IdentityPassword,
    OAuth2,
    KeyPair,
    Certificate,
    RequestSigning,
    FederatedIdentity,
    ChallengeResponse,
    OneTimePasscode,
    ConnectionUri,
    InstanceIdentity,
    SharedSecret,
    Custom,
}
```

### 1.3 Built-in Scheme Types (12)

Each is a pure data struct. No transport/injection logic.

```rust
/// Opaque secret string (API key, bearer token, session token).
pub struct SecretToken {
    pub token: SecretString,
}

/// Identity + password pair (user/email/account + password).
pub struct IdentityPassword {
    pub identity: String,
    pub password: SecretString,
}

/// OAuth2/OIDC token set.
pub struct OAuth2Token {
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub token_type: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub scope: Option<String>,
}

/// Asymmetric key pair (SSH, PGP, crypto wallets).
pub struct KeyPair {
    pub public_key: String,
    pub private_key: SecretString,
    pub passphrase: Option<SecretString>,
    pub algorithm: Option<String>,
}

/// X.509 certificate + private key (mTLS, TLS client auth).
pub struct Certificate {
    pub cert_chain: String,          // PEM-encoded
    pub private_key: SecretString,   // PEM-encoded
    pub passphrase: Option<SecretString>,
}

/// Request signing credentials (HMAC, SigV4, webhook signatures).
pub struct SigningKey {
    pub key: SecretString,
    pub algorithm: String,           // "hmac-sha256", "aws-sigv4", etc.
}

/// Third-party identity assertion (SAML, JWT, Kerberos ticket).
pub struct FederatedAssertion {
    pub assertion: SecretString,     // JWT string, SAML XML, ticket bytes
    pub issuer: String,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Challenge-response protocol credentials (Digest, NTLM, SCRAM).
pub struct ChallengeSecret {
    pub identity: String,
    pub secret: SecretString,
    pub protocol: String,            // "digest", "ntlm", "scram-sha-256"
}

/// TOTP/HOTP seed or OTP delivery config.
pub struct OtpSeed {
    pub seed: SecretString,
    pub algorithm: String,           // "totp-sha1", "hotp-sha1"
    pub digits: u8,
    pub period: Option<u32>,         // TOTP period in seconds
}

/// Compound connection URI (postgres://..., redis://..., mongodb://...).
pub struct ConnectionUri {
    pub uri: SecretString,
}

/// Cloud/infrastructure instance identity (IMDS, managed identity).
pub struct InstanceBinding {
    pub provider: String,            // "aws", "gcp", "azure"
    pub role_or_account: String,
    pub region: Option<String>,
}

/// Pre-shared symmetric key (TLS-PSK, WireGuard, IoT).
pub struct SharedKey {
    pub key: SecretString,
    pub identity: Option<String>,    // PSK identity hint
}
```

---

## 2. Credential Trait — Unchanged Core, New Derive

### 2.1 Trait (unchanged state machine)

The existing `resolve() → Pending → continue_resolve()` state machine is solid and handles all flow types (single-step, interactive, polling, multi-step). Keep it.

```rust
pub trait Credential: Send + Sync + 'static {
    type Scheme: AuthScheme;
    type State: CredentialState;
    type Pending: PendingState;

    // Capability flags
    const INTERACTIVE: bool = false;
    const REFRESHABLE: bool = false;
    const TESTABLE: bool = false;

    // Required
    fn description() -> CredentialDescription;
    fn resolve(values: &ParameterValues, ctx: &CredentialContext)
        -> impl Future<Output = Result<ResolveResult<Self>, CredentialError>> + Send;
    fn project(state: &Self::State) -> Self::Scheme;

    // Optional with defaults
    fn continue_resolve(pending: &Self::Pending, input: &UserInput, ctx: &CredentialContext)
        -> impl Future<Output = Result<ResolveResult<Self>, CredentialError>> + Send { ... }
    fn refresh(state: &mut Self::State, ctx: &CredentialContext)
        -> impl Future<Output = Result<RefreshOutcome, CredentialError>> + Send { ... }
    fn test(scheme: &Self::Scheme) -> impl Future<Output = Option<TestResult>> + Send {
        async { None }
    }
}
```

### 2.2 Credential Description uses Parameters

```rust
pub struct CredentialDescription {
    pub key: CredentialKey,
    pub name: String,
    pub description: String,
    pub icon: Option<String>,
    pub parameters: ParameterCollection,  // from nebula-parameter
    pub pattern: AuthPattern,
}
```

### 2.3 Test — Optional Self-Test

Credential CAN test itself if it knows an endpoint. Otherwise returns None — resource-level testing handles the rest.

```rust
// API credential — can self-test
fn test(scheme: &SecretToken) -> Option<TestResult> {
    Some(http_check("https://api.stripe.com/v1/balance", &scheme.token))
}

// Database credential — cannot self-test without resource context
fn test(scheme: &IdentityPassword) -> Option<TestResult> {
    None  // test via Resource::check() instead
}
```

---

## 3. Three Authoring Levels

### Level 1 — Derive (static credentials, 5-10 lines)

```rust
#[derive(Credential, Parameters)]
#[credential(scheme = SecretToken)]
struct StripeAuth {
    #[param(label = "Secret Key", secret)]
    #[validate(required)]
    #[credential(into = "token")]  // maps api_key → SecretToken.token
    api_key: String,
}
// Auto-generates: resolve(), project(), description(), parameters()
// resolve() reads api_key from ParameterValues, wraps in SecretToken { token }
// secret fields auto-wrapped in SecretString
```

```rust
#[derive(Credential, Parameters)]
#[credential(scheme = IdentityPassword)]
struct BasicAuth {
    #[param(label = "Username")]
    #[validate(required)]
    #[credential(into = "identity")]
    username: String,

    #[param(label = "Password", secret)]
    #[validate(required)]
    #[credential(into = "password")]
    password: String,
}
```

### Level 2 — Extends / Composition (OAuth2 providers)

```rust
#[derive(Credential)]
#[credential(
    extends = OAuth2Base,
    auth_url = "https://github.com/login/oauth/authorize",
    token_url = "https://github.com/login/oauth/access_token",
    scopes = ["repo", "user"],
)]
struct GitHubOAuth2;

// OAuth2Base provides: resolve() with PKCE, continue_resolve() with code exchange,
// refresh(), full interactive flow. GitHubOAuth2 only overrides URLs + scopes.

#[derive(Credential)]
#[credential(extends = GoogleOAuth2, scopes = ["spreadsheets"])]
struct GoogleSheetsAuth;
// Inherits from GoogleOAuth2 which inherits from OAuth2Base
```

### Level 3 — Manual (full control)

```rust
impl Credential for KerberosAuth {
    type Scheme = FederatedAssertion;
    type State = KerberosState;
    type Pending = KerberosPending;

    const INTERACTIVE: bool = true;
    const REFRESHABLE: bool = true;

    fn resolve(...) -> ... {
        // Custom: contact KDC, get TGT, negotiate service ticket
    }

    fn continue_resolve(...) -> ... {
        // Multi-step: TGT → service ticket → done
    }
}
```

---

## 4. Security Fixes

### 4.1 Encryption Key Rotation

```rust
pub struct EncryptedData {
    pub key_id: String,          // NEW: which key encrypted this
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub aad: Vec<u8>,
}

pub struct EncryptionLayer<S> {
    keys: HashMap<String, Arc<EncryptionKey>>,  // multiple keys
    current_key_id: String,                     // active key for new encrypts
    inner: S,
}
```

- Encrypt with `current_key_id`
- Decrypt by looking up `key_id` from `EncryptedData`
- Re-encrypt on read if `key_id != current_key_id` (lazy rotation)

### 4.2 Remove AAD Legacy Fallback — Hard Cutover

```rust
// REMOVE entirely: no fallback, no config flag
// Provide one-time migration tool that re-encrypts all records with AAD
// After migration: AAD validation failure = hard error, always
```

No `legacy_compat` flag — an attacker who can swap records can also strip AAD to force fallback, making the flag useless.

### 4.3 Plaintext Zeroization — Use `Zeroizing<Vec<u8>>`

```rust
use zeroize::Zeroizing;

// In EncryptionLayer::put():
let plaintext: Zeroizing<Vec<u8>> = Zeroizing::new(serde_json::to_vec(&credential.data)?);
let encrypted = self.encrypt(&plaintext)?;
// plaintext auto-zeroized on drop — no manual .zeroize() needed
// never clone plaintext buffers — pass by reference only
```

All intermediate plaintext buffers use `Zeroizing<T>` wrapper, NOT manual `.zeroize()` on clones.

### 4.4 SecretString Serde Safety

- `Serialize` → `[REDACTED]` (already correct for logging)
- Storage serialization → must use `#[serde(with = "serde_secret")]`
- NEW: `#[deny(secret_serialize_without_serde_secret)]` — custom lint or compile-time check ensuring `SecretString` fields in CredentialState always use `serde_secret`

---

## 5. Credential Identity — Two Key Types

### 5.1 Problem

Rotation subsystem uses `CredentialId` (UUID). Resolver uses `&str`. Spec initially tried to unify — wrong, they serve different purposes.

### 5.2 Solution — Keep Both

| Key | Purpose | Type | Example |
|-----|---------|------|---------|
| `CredentialKey` | **Type** identity (compile-time) | `domain_key!` string | `"stripe"`, `"github_oauth2"` |
| `CredentialId` | **Instance** identity (runtime) | UUID | `550e8400-e29b-...` |

```rust
// Registry uses CredentialKey (which credential TYPE)
registry.register::<StripeAuth>();  // key = "stripe"

// Store uses CredentialId (which credential INSTANCE)
store.get(credential_id)?;  // specific user's Stripe key

// Resolver uses CredentialId to load, CredentialKey to dispatch
pub async fn resolve<C: Credential>(
    &self,
    credential_id: &CredentialId,
) -> Result<CredentialHandle<C::Scheme>, ResolveError>
```

Resolver internally: `CredentialId` → load from store → `state_kind` string → `CredentialKey` → dispatch to registered `Credential` impl.

---

## 6. identity_state! → Automatic via Derive

Remove the need for manual `identity_state!` calls. The derive macro auto-generates `CredentialState` impl:

```rust
#[derive(Credential, Parameters)]
#[credential(scheme = SecretToken)]
struct StripeAuth { ... }
// Derive auto-generates:
// - impl CredentialState for SecretToken { KIND = "secret_token"; VERSION = 1; }
// - impl HasParameters for StripeAuth
// - impl Credential for StripeAuth
```

For AuthScheme types that don't go through derive: provide a simple attribute macro:

```rust
#[derive(AuthScheme)]
#[auth_scheme(pattern = SecretToken)]
pub struct SecretToken { ... }
// Generates: impl AuthScheme + impl CredentialState
```

---

## 7. Storage Layers — Enhanced

### 7.1 Existing (keep)
- `EncryptionLayer` — AES-256-GCM (enhanced with key rotation)
- `CacheLayer` — moka LRU + TTL
- `AuditLayer` — audit trail
- `ScopeLayer` — scoped visibility

### 7.2 New: Persistent PendingStateStore

```rust
pub trait PendingStateStore: Send + Sync {
    async fn store(&self, token: &PendingToken, state: EncryptedPending, ttl: Duration) -> Result<()>;
    async fn retrieve(&self, token: &PendingToken) -> Result<Option<EncryptedPending>>;
    async fn consume(&self, token: &PendingToken) -> Result<Option<EncryptedPending>>;
}

// Implementations:
// - InMemoryPendingStore (existing, for dev/test)
// - RedisPendingStore (new, for production)
// - PostgresPendingStore (new, for production)
```

---

## 8. What Stays in Credential Core vs Plugins

### Core (nebula-credential)

- AuthScheme trait + AuthPattern enum
- 12 built-in scheme structs (pure data, no transport)
- Credential trait + state machine (resolve/continue_resolve/refresh)
- OAuth2Base composition block (reusable for any OAuth2 provider)
- Storage layers (encryption, cache, audit, scope)
- Resolver + RefreshCoordinator
- CredentialRegistry
- Derive macros (Credential, AuthScheme)
- Rotation subsystem

### Plugins (separate crates)

- Protocol-specific credentials (SshCredential, LdapCredential, KerberosCredential)
- Provider-specific OAuth2 (GitHubOAuth2, GoogleOAuth2, SlackOAuth2)
- Transport-specific injection (how to apply SecretToken to HTTP vs gRPC)
- Resource-level test-connection

---

## 9. Integration with Parameters

Credential parameters use the same nebula-parameter system. The derive macro reads `#[param(...)]` and `#[validate(...)]` attributes from credential struct fields.

**Important:** Credential fields contain ONLY auth material. Connection config (host/port/database/ssl) belongs on the Resource, not the Credential. A database credential is just `IdentityPassword`.

```rust
// CORRECT: credential = auth material only
#[derive(Credential, Parameters)]
#[credential(scheme = IdentityPassword)]
struct DatabaseLogin {
    #[param(label = "Username")]
    #[validate(required)]
    #[credential(into = "identity")]
    username: String,

    #[param(label = "Password", secret)]
    #[validate(required)]
    #[credential(into = "password")]
    password: String,
}

// Connection config lives on the RESOURCE (plugin side):
// PostgresResource { host, port, database, ssl_mode, credential: CredentialId }
```

More complex example — OAuth2 with scope selection:

```rust
#[derive(Credential)]
#[credential(
    extends = OAuth2Base,
    auth_url = "https://accounts.google.com/o/oauth2/v2/auth",
    token_url = "https://oauth2.googleapis.com/token",
)]
struct GoogleAuth;

// Plugin extends with specific scopes:
#[derive(Credential)]
#[credential(extends = GoogleAuth, scopes = ["https://www.googleapis.com/auth/drive"])]
struct GoogleDriveAuth;
```

User fills form → ParameterValues → Credential::resolve() → AuthScheme → Resource uses it to connect.

---

## 10. Breaking Changes Summary

| Change | Impact |
|--------|--------|
| AuthScheme: closed enum → open trait | All scheme consumers adapt to trait objects or generics |
| 13 specific schemes → 12 universal patterns | SSH/LDAP/Kerberos/AWS move to plugins |
| EncryptedData gains `key_id` | Storage migration needed |
| AAD fallback removed | Old non-AAD data unreadable without migration |
| Credential ID: `CredentialId` (UUID) → `CredentialKey` (string) | Rotation subsystem updated |
| `identity_state!` → automatic via derive | Simpler, but manual callers update |
| `Credential::test()` → returns `Option<TestResult>` | Existing impls update signature |

---

## 11. Not In Scope (Deferred)

- Hardware/biometric auth (FIDO2, WebAuthn) — Phase 2
- Delegation/impersonation patterns — composition over existing
- Actual plugin implementations (SSH, LDAP, Kerberos) — separate crates
- PendingStateStore Redis/Postgres backends — after storage crate stabilizes
- Credential UI components — desktop app concern
- Localization of credential forms — uses parameter localization system
