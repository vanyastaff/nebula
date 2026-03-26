# nebula-credential v2 — High-Level Design

> **Revision 6 (final)** — 3 design rounds + adversarial round.
> (ChatGPT, DeepSeek, Gemini × 4 rounds, triaged by Claude).
>
> **Key changes in Rev 6 (adversarial round):**
> - PendingStateStore: token bound to (credential_kind, owner_id, session_id, token_id)
> - Framework timeout on all credential methods (refresh/test/revoke): 30s hard limit
> - scopeguard on RefreshCoordinator Notify — notify_waiters on any exit (panic, timeout, error)
> - Waiter timeout: 60s max wait on Notify, then re-read from store
> - RetryAdvice clamping: framework enforces min_retry_backoff + circuit breaker
> - Security Model section: trust boundaries, callback security, error normalization, operational hardening

## Overview

nebula-credential manages authentication and authorization for external services
in the Nebula workflow engine. It stores, resolves, refreshes, and rotates
credentials that resources and actions need to connect to third-party APIs,
databases, messaging platforms, and infrastructure.

### What it solves

Every resource in Nebula needs authorization: a database needs username/password,
a GitHub API needs a bearer token, a Telegram bot needs a bot token, an SSH
connection needs keys. The authorization landscape is complex:

| Service | Auth method | Challenge |
|---------|------------|-----------|
| GitHub | API Key OR OAuth2 | Two methods, same result: bearer token |
| Postgres | username + password | Connection string, SSL certs, Vault dynamic secrets |
| Telegram | Bot token | Single secret, no refresh, no expiry |
| Google Sheets | OAuth2 OR Service Account | User-facing flow vs server-to-server JWT |
| AWS | Access key + secret OR STS assume role | Static vs composed (role depends on base cred) |
| SSH | Password OR key pair OR agent | Three methods, same result: authenticated session |
| LDAP | Simple bind OR SASL GSSAPI OR EXTERNAL | Bind method may depend on Kerberos/mTLS credential |
| SAML | IdP redirect + assertion POST | POST-based callback, non-refreshable assertions |

Without nebula-credential, every resource author implements their own credential
handling, every action author manually deserializes JSON blobs, and credential
rotation requires custom code per resource.

With it:

```rust
// Action author — one line, typed, auto-refreshed
let db = ctx.resource::<Postgres>().await?;
db.query("SELECT 1", &[]).await?;
// credential was resolved, refreshed, and injected automatically

// Or direct credential access when needed
let token = ctx.credential::<BearerToken>().await?.snapshot();
http.header("Authorization", token.bearer_header());
```

### Core guarantees

1. **AuthScheme as the contract** — credentials produce typed auth material
   (`BearerToken`, `BasicAuth`, `DatabaseAuth`). Resources consume auth material.
   Credential and resource never know about each other directly. Multiple
   credential types can produce the same auth scheme.

2. **State/Scheme separation** — stored state (`OAuth2State` with refresh_token,
   client_secret) is distinct from consumer-facing auth material (`BearerToken`).
   `project()` extracts the scheme from the state. Refresh internals never leak
   to resources.

3. **Protocol transparency** — a resource declaring `type Auth = BearerToken`
   accepts any credential that produces `BearerToken`: API key, OAuth2,
   service account, SAML bearer assertion — the resource author doesn't care
   how the token was obtained.

4. **Encrypted at rest, zeroized in memory** — credentials are always encrypted
   with AES-256-GCM before persistence. `SecretString` zeroizes on drop.
   Plaintext exists only during active use.

5. **Auto-refresh** — framework transparently refreshes expiring credentials
   (OAuth2 tokens, Kerberos tickets, AWS STS) before they expire. Callers
   see only valid auth material.

6. **Credential composition** — credentials can depend on other credentials.
   AWS Assume Role uses a base AWS credential. LDAP GSSAPI uses a Kerberos
   credential. Framework resolves the chain with cycle detection (configurable
   max depth, default 3).

7. **Layered storage** — encryption, caching, scoping, and audit are composable
   layers wrapping a simple CRUD store. Cache stores ciphertext only — plaintext
   never resides in cache heap. Scope checks happen before any data access.
   Audit layer receives redacted metadata, never plaintext secrets. No god objects.

8. **Serde for storage only** — `CredentialState` types are `Serialize +
   DeserializeOwned` for persistence through `EncryptionLayer`. `AuthScheme`
   types also carry these bounds for the `State = Scheme` identity path.
   **Security contract:** serialization to plaintext JSON happens exclusively
   inside `EncryptionLayer` — never in logging, debugging, IPC, or telemetry.
   All built-in types implement `Debug` with redacted secrets.

### System boundaries and data flow

nebula-credential owns everything between "user fills credential form" and
"resource receives typed auth material":

```
User fills credential form in UI
       │
       ▼
┌─ Credential Setup ──────────────────────────────────────────┐
│  1. Frontend renders ParameterCollection as form             │
│  2. User submits ParameterValues                             │
│  3. Credential::resolve(values, ctx)                         │
│     - Static: → Complete(State)                              │
│     - Interactive: → Pending(PendingToken, InteractionReq)   │
│       - Typed PendingState stored in ephemeral store         │
│       - PendingToken = opaque handle (not raw state)         │
│       - TTL enforced, single-use, encrypted, zeroized        │
│       - Redirect (OAuth2 auth code)                          │
│       - FormPost (SAML POST binding)                         │
│       - DisplayInfo (device code, 2FA)                       │
│     - User completes interaction → continue_resolve(token)   │
│       - PendingState loaded + consumed (deleted after read)  │
│       - Complete(State) → done                               │
│       - Retry { after } → poll again (device code)           │
│  4. State serialized + encrypted → Store                     │
└──────────────────────────────────────────────────────────────┘
       │
       ▼
┌─ Credential Store (layered) ────────────────────────────────┐
│  ScopeLayer (multi-tenant isolation — fail fast)             │
│    → AuditLayer (redacted metadata only — never plaintext)   │
│      → EncryptionLayer (AES-256-GCM — decrypt on read)       │
│        → CacheLayer (moka LRU + TTL — caches ciphertext)     │
│          → Backend (Local / Postgres / Vault / K8s / AWS)     │
│                                                               │
│  Invariants:                                                  │
│  • Scope checked before any data access                       │
│  • Audit never receives plaintext secret material             │
│  • Cache stores ciphertext only — no plaintext in heap        │
│  • Decryption cost per-read (~1-3μs with AES-NI)             │
└──────────────────────────────────────────────────────────────┘
       │
       ▼
┌─ Runtime Resolution ────────────────────────────────────────┐
│  1. Resource acquire triggers credential resolution          │
│  2. CredentialResolver loads stored State from store         │
│  3. project(state) → Scheme (extract consumer-facing auth)   │
│  4. RefreshGuard checks expires_at → auto-refresh if needed  │
│     - Refreshed → update State in store, re-project          │
│     - ReauthRequired → trigger re-resolve (SAML, etc.)       │
│  5. Scheme coercion: TryInto<Resource::Auth> if needed       │
│  6. Typed auth passed to Resource::create(config, auth, ctx) │
│  7. On rotation: CredentialRotated event → EventBus          │
│     → ResourceManager re-authorizes live instances           │
└──────────────────────────────────────────────────────────────┘
```

### What nebula-credential does NOT own

| Concern | Owner | Integration |
|---------|-------|-------------|
| Resource pool management | nebula-resource | Resource declares `type Auth`; framework resolves |
| UI form rendering | frontend (React/Tauri) | Consumes `ParameterCollection` as JSON |
| Expression evaluation | nebula-engine | `$expr` markers in parameter values |
| Retry/circuit-breaker | nebula-resilience | Used internally for token endpoint calls |
| Config hot-reload | nebula-config | `AsyncConfigurable` on `CredentialResolver` |

### Key types at a glance

| Type | Role | Location |
|------|------|----------|
| `AuthScheme` | Marker trait for auth material types | scheme.rs |
| `CredentialState` | Trait for stored state types (no blanket impl) | state.rs |
| `identity_state!` | Macro: opt AuthScheme into CredentialState | state.rs |
| `PendingState` | Typed ephemeral state for interactive flows | resolve/pending.rs |
| `PendingToken` | Opaque handle to stored PendingState | resolve/pending.rs |
| `BearerToken` | Bearer/API key auth material | scheme/bearer.rs |
| `BasicAuth` | Username + password auth material | scheme/basic.rs |
| `DatabaseAuth` | Full database connection auth | scheme/database.rs |
| `ApiKeyAuth` | API key with placement (header/query) | scheme/api_key.rs |
| `HeaderAuth` | Custom header auth material | scheme/header.rs |
| `CertificateAuth` | mTLS certificate material | scheme/certificate.rs |
| `SshAuth` | SSH auth with host/method | scheme/ssh.rs |
| `OAuth2Token` | OAuth2 bearer (no refresh internals) | scheme/oauth2.rs |
| `AwsAuth` | AWS credentials + region | scheme/aws.rs |
| `LdapAuth` | LDAP bind credentials + connection | scheme/ldap.rs |
| `SamlAuth` | SAML assertion + attributes | scheme/saml.rs |
| `KerberosAuth` | Kerberos ticket | scheme/kerberos.rs |
| `HmacSecret` | Webhook signing secret | scheme/hmac.rs |
| `Credential` | Core trait — resolve + project + refresh | credential.rs |
| `CredentialHandle<S>` | Typed handle, snapshot() API (not Deref) | handle.rs |
| `RefreshPolicy` | Per-credential refresh timing (early, backoff, jitter) | refresh/policy.rs |
| `PendingStateStore` | Ephemeral store: put/get/consume/delete with encryption | resolve/pending_store.rs |
| `CredentialLifecycle` | Persisted status: Active/ReauthRequired/Terminal | store.rs |
| `RetryAdvice` | Never/Immediate/After(Duration) on refresh errors | error.rs |
| `CredentialDescription` | Metadata: key, name, icon, parameter schema | description.rs |
| `CredentialStore` | Core CRUD trait with PutMode, returns committed state | store.rs |
| `PutMode` | CreateOnly / Overwrite / CompareAndSwap | store.rs |
| `EncryptionLayer` | Encrypt/decrypt layer over store | layer/encryption.rs |
| `CacheLayer` | Moka LRU + TTL cache layer | layer/cache.rs |
| `ScopeLayer` | Multi-tenant isolation layer | layer/scope.rs |
| `AuditLayer` | Access logging layer | layer/audit.rs |
| `CredentialResolver` | Runtime resolution + refresh | resolver.rs |
| `RefreshCoordinator` | CAS-based refresh coordination | refresh.rs |
| `CredentialRotatedEvent` | EventBus event for resource re-auth | event.rs |

### AuthScheme quick reference

| Scheme | Fields | Produced by | Consumed by |
|--------|--------|------------|-------------|
| `BearerToken` | `token: SecretString` | API Key, OAuth2, Service Account, SAML bearer | HTTP APIs (GitHub, Slack, OpenAI) |
| `BasicAuth` | `username, password` | Basic Auth credential | HTTP Basic, some APIs |
| `DatabaseAuth` | `host, port, database, username, password, ssl` | Database credential, Vault dynamic | Postgres, MySQL, MongoDB |
| `ApiKeyAuth` | `key, placement: Header/QueryParam` | API Key credential | APIs with custom key placement |
| `HeaderAuth` | `name, value` | Header Auth credential | Custom header APIs |
| `CertificateAuth` | `cert_pem, key_pem, ca_pem` | mTLS credential | mTLS services |
| `SshAuth` | `host, port, username, method: Password/Key/Agent` | SSH credential | SSH connections |
| `OAuth2Token` | `access_token, token_type, scopes, expires_at` | OAuth2 credential | APIs needing scope info |
| `AwsAuth` | `access_key, secret_key, session_token, region` | AWS credential, STS | AWS services |
| `LdapAuth` | `host, port, bind_method, tls_mode, base_dn` | LDAP credential | LDAP directories |
| `SamlAuth` | `name_id, attributes, session_index, assertion_b64` | SAML credential | SAML-protected APIs |
| `KerberosAuth` | `principal, realm, service_ticket, expires_at` | Kerberos credential | SPNEGO/GSSAPI services |
| `HmacSecret` | `secret, algorithm` | Webhook signing credential | Webhook verification |
| `NoAuth` | `()` | (implicit) | Resources without credentials |

---

## Action Author View

This section is for engineers writing **actions and triggers** — the consumers
of credentials. You do NOT need to understand protocols, storage layers, or
rotation. You need exactly two patterns.

### Pattern 1: Credential through resource (most common)

99% of the time, you never touch credentials directly. The resource handles it:

```rust
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    // Framework resolved credential, created authorized connection
    let db = ctx.resource::<Postgres>().await?;
    let rows = db.query("SELECT * FROM users WHERE id = $1", &[&input.user_id]).await?;
    Ok(ActionResult::new(rows))
}
```

The credential was:
1. Loaded from encrypted storage
2. Decrypted
3. Auto-refreshed if expired (OAuth2 tokens, Kerberos tickets)
4. Projected from stored state to auth scheme via `project()`
5. Passed to `Postgres::create(config, auth, ctx)`
6. Used to establish the authorized connection

You see none of this. It just works.

### Pattern 2: Direct credential access (rare)

For cases where you need raw auth material — constructing custom requests,
passing tokens to third-party SDKs, multi-credential workflows:

```rust
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    // Typed credential handle — auto-refreshed, zero deserialization
    let token = ctx.credential::<BearerToken>().await?.snapshot();

    let response = reqwest::Client::new()
        .get("https://api.example.com/data")
        .header("Authorization", token.bearer_header())
        .send().await?;

    Ok(ActionResult::new(response.json().await?))
}

// Multiple credentials in one action
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    let github = ctx.credential::<BearerToken>().await?.snapshot();
    let db_auth = ctx.credential::<DatabaseAuth>().await?.snapshot();
    // ...
}
```

### What you see vs what's hidden

| You see | Framework handles |
|---------|-------------------|
| `ctx.credential::<BearerToken>().await?.snapshot()` | Storage lookup, decryption, cache, State → Scheme projection |
| Valid token every time | Auto-refresh per `refresh_policy()` before expiry (OAuth2, AWS STS, Kerberos) |
| Type-safe `Arc<BearerToken>` | Deserialization from stored JSON + project() extraction |
| Instant response (usually) | Moka cache hit, bypass storage I/O |
| Error on missing credential | Scope isolation — can't access other tenant's creds |

### Declaring dependencies

Actions declare credential requirements at registration time:

```rust
impl ActionDependencies for SendGitHubCommentAction {
    fn credential() -> Option<CredentialRequirement> {
        // "I need something that produces BearerToken"
        Some(CredentialRequirement::scheme::<BearerToken>())
    }

    fn resources() -> Vec<Box<dyn AnyResource>> {
        vec![] // No resources — using direct HTTP
    }
}
```

The engine uses this to:
- Validate that a compatible credential is configured before workflow starts
- Show "requires: Bearer Token credential" in the UI
- Provide clear errors at startup, not runtime

### Error handling

Credential errors are simple:

| Error | Meaning | What you do |
|-------|---------|-------------|
| `CredentialNotFound` | No credential configured for this action | Check workflow config |
| `CredentialExpired` | Refresh failed, token truly expired | Re-authenticate |
| `CredentialRevoked` | Credential explicitly revoked | Re-authenticate |
| `ScopeViolation` | Multi-tenant access denied | Check tenant config |
| `ResolutionFailed` | Storage/decryption error | Platform issue |

All credential errors are `ActionError::fatal()` — no retry. If the credential
is broken, retrying won't fix it.

---

## Credential Author View

This section is for engineers adding **new credential types** — new protocols
and new service-specific credentials. You implement one trait.

### The Credential trait

```rust
pub trait Credential: Send + Sync + 'static {
    /// What this credential produces — the consumer-facing auth material.
    type Scheme: AuthScheme;

    /// What gets stored — may include refresh internals not exposed to resources.
    /// For static credentials: same type as Scheme (use identity_state! macro).
    type State: CredentialState;

    /// Typed pending state for interactive flows.
    /// Non-interactive credentials: use `NoPendingState`.
    /// No default — associated type defaults are unstable on stable Rust.
    /// The `#[derive(Credential)]` macro fills `NoPendingState` automatically.
    type Pending: PendingState;

    /// Stable key for this credential type.
    const KEY: CredentialKey;

    /// Capability flags — associated consts (stable, compile-time, no allocation).
    /// Must match actual method behavior — debug_assert in registry validates.
    const INTERACTIVE: bool = false;
    const REFRESHABLE: bool = false;
    const REVOCABLE: bool = false;
    const TESTABLE: bool = false;

    /// Refresh timing policy — associated const (compile-time, consistent with
    /// other const config). Duration::from_secs() is const fn since Rust 1.53.
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

    /// Human-readable metadata: name, icon, documentation URL.
    fn description() -> CredentialDescription
    where
        Self: Sized;

    /// Parameter schema for the setup form.
    fn parameters() -> ParameterCollection
    where
        Self: Sized;

    /// Extract consumer-facing auth material from stored state.
    fn project(state: &Self::State) -> Self::Scheme
    where
        Self: Sized;

    /// Resolve user input into credential state.
    ///
    /// **Framework handles PendingState storage.** Credential returns raw
    /// `Pending { state: P, interaction }` — framework encrypts, stores,
    /// generates PendingToken, and manages the lifecycle. Credential author
    /// never calls store_pending() or consume_pending().
    ///
    /// For non-interactive credentials: use `StaticResolveResult<S>` alias.
    fn resolve(
        values: &ParameterValues,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<Self::State, Self::Pending>, CredentialError>> + Send
    where
        Self: Sized;

    /// Continue interactive resolve after user completes interaction.
    /// Framework loads and consumes PendingState before calling this.
    /// The `pending` parameter is the typed state returned by resolve().
    /// Default: not interactive.
    fn continue_resolve(
        _pending: &Self::Pending,
        _input: &UserInput,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<Self::State, Self::Pending>, CredentialError>> + Send
    where
        Self: Sized,
    {
        async { Err(CredentialError::NotInteractive) }
    }

    /// Test that the credential actually works. Default: untestable.
    fn test(
        _scheme: &Self::Scheme,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<TestResult, CredentialError>> + Send
    where
        Self: Sized,
    {
        async { Ok(TestResult::Untestable) }
    }

    /// Refresh expiring auth material. Default: not refreshable.
    fn refresh(
        _state: &mut Self::State,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<RefreshOutcome, CredentialError>> + Send
    where
        Self: Sized,
    {
        async { Ok(RefreshOutcome::NotSupported) }
    }

    /// Revoke credential. Default: no-op.
    fn revoke(
        _state: &mut Self::State,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<(), CredentialError>> + Send
    where
        Self: Sized,
    {
        async { Ok(()) }
    }
}

/// Convenience alias for non-interactive credentials.
/// Avoids writing `ResolveResult<MyState, NoPendingState>` everywhere.
pub type StaticResolveResult<S> = ResolveResult<S, NoPendingState>;
```

One trait. `type Pending` explicit (no default — unstable on stable Rust).
All credential config as associated consts (INTERACTIVE, REFRESHABLE, REFRESH_POLICY).
`ctx` is now `&CredentialContext` (not `&mut`) — credential doesn't mutate context.
Framework handles PendingState storage — credential returns raw state in
`ResolveResult::Pending { state, interaction }`. `#[derive(Credential)]` macro
fills defaults for static credentials.

### ResolveResult — immediate, interactive, or polling

```rust
pub enum ResolveResult<S, P: PendingState = NoPendingState> {
    /// Credential ready immediately (API key, basic auth, database).
    Complete(S),

    /// Requires user interaction (OAuth2 redirect, SAML, device code, 2FA).
    ///
    /// **Credential returns raw PendingState.** Framework handles:
    /// - Encrypting and storing the state in PendingStateStore
    /// - Generating a CSPRNG PendingToken bound to owner
    /// - Loading and consuming the state before continue_resolve()
    ///
    /// Credential author never calls store_pending() or consume_pending().
    Pending {
        /// Raw typed pending state — framework stores this securely.
        state: P,
        /// What to show/redirect the user.
        interaction: InteractionRequest,
    },

    /// Framework should call continue_resolve() again after delay.
    /// Used by device code flow (RFC 8628) polling pattern.
    Retry {
        after: Duration,
    },
}

/// Convenience alias for non-interactive credentials.
pub type StaticResolveResult<S> = ResolveResult<S, NoPendingState>;
```

### Framework resolve executor — handles PendingState lifecycle

The credential author writes pure functions. The framework executor handles
the PendingState storage lifecycle, timeouts, and error wrapping:

```rust
// Inside framework (NOT written by credential author):
async fn execute_resolve<C: Credential>(
    values: &ParameterValues,
    ctx: &CredentialContext,
    pending_store: &dyn PendingStateStore,
    credential_id: &CredentialId,
) -> Result<UiResponse, ResolutionError> {
    // Framework wraps credential call in timeout (30s hard limit)
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        C::resolve(values, ctx),
    ).await
        .unwrap_or_else(|_| Err(CredentialError::Provider(
            "framework timeout: resolve took too long".into()
        )))
        .map_err(|e| ResolutionError::new(credential_id.clone(), ResolutionStage::Resolve, e))?;

    match result {
        ResolveResult::Complete(state) => {
            // Encrypt + store state, return success
            Ok(UiResponse::Complete)
        }
        ResolveResult::Pending { state, interaction } => {
            // 4-dimensional binding: credential_kind + owner + session + random token
            let token = pending_store.put(
                C::KEY.as_str(),        // credential_kind — framework injects
                &ctx.owner_id,           // owner binding
                ctx.session_id(),        // session binding
                state,
            ).await?;
            Ok(UiResponse::Pending { token, interaction })
        }
        ResolveResult::Retry { after } => Ok(UiResponse::Retry { after }),
    }
}

// On callback, framework loads + consumes pending state before calling credential:
async fn execute_continue<C: Credential>(
    token: &PendingToken,
    input: &UserInput,
    ctx: &CredentialContext,
    pending_store: &dyn PendingStateStore,
    credential_id: &CredentialId,
) -> Result<UiResponse, ResolutionError> {
    // 4-dimensional validation on consume
    let pending: C::Pending = pending_store.consume(
        C::KEY.as_str(),        // must match credential that created it
        token,
        &ctx.owner_id,           // must match original owner
        ctx.session_id(),        // must match original session
    ).await
        .map_err(|e| ResolutionError::new(credential_id.clone(), ResolutionStage::ContinueResolve, e))?;

    // Timeout on continue_resolve too
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        C::continue_resolve(&pending, input, ctx),
    ).await
        .unwrap_or_else(|_| Err(CredentialError::Provider(
            "framework timeout: continue_resolve took too long".into()
        )))
        .map_err(|e| ResolutionError::new(credential_id.clone(), ResolutionStage::ContinueResolve, e))?;

    match result {
        ResolveResult::Complete(state) => Ok(UiResponse::Complete),
        ResolveResult::Pending { state, interaction } => {
            let new_token = pending_store.put(
                C::KEY.as_str(), &ctx.owner_id, ctx.session_id(), state,
            ).await?;
            Ok(UiResponse::Pending { token: new_token, interaction })
        }
        ResolveResult::Retry { after } => Ok(UiResponse::Retry { after }),
    }
}
```

This design means:
- `resolve()` and `continue_resolve()` are **pure functions** — testable without mocking stores
- Credential author **cannot** forget to consume pending state — framework does it
- `PendingToken` is **framework-internal** — never appears in credential author code
- `CredentialContext` has **no pending_store** — simpler, lighter
- **4-dimensional token binding** prevents type confusion, cross-user replay, session fixation
- **30s timeout** on all credential methods — hostile/hung plugins can't block workers

### PendingState — typed ephemeral state for interactive flows

```rust
/// Typed pending state for interactive credential flows.
/// Stored via PendingStateStore with encryption, TTL, single-use.
///
/// Security properties:
/// - TTL enforced: expires_in() determines max lifetime (typically 5-15 min)
/// - Single-use: consumed (deleted) on first read by continue_resolve()
/// - Encrypted at rest by PendingStateStore implementation
/// - ZeroizeOnDrop: secrets zeroed when state is dropped
/// - Serialization buffers wrapped in Zeroizing<Vec<u8>> by store implementation
///
/// Multi-node: PendingStateStore handles encryption before persistence/replication.
/// The Serialize bound enables store implementation to serialize internally.
pub trait PendingState:
    Serialize + DeserializeOwned + Send + Sync + ZeroizeOnDrop + 'static
{
    const KIND: &'static str;
    fn expires_in(&self) -> Duration;
}

/// Marker type for non-interactive credentials.
#[derive(Clone, Serialize, Deserialize)]
pub struct NoPendingState;
impl ZeroizeOnDrop for NoPendingState {}
impl PendingState for NoPendingState {
    const KIND: &'static str = "none";
    fn expires_in(&self) -> Duration { Duration::ZERO }
}

/// Example typed pending state for OAuth2:
#[derive(Clone, Serialize, Deserialize, ZeroizeOnDrop)]
pub struct OAuth2Pending {
    pub pkce_verifier: SecretString,
    pub csrf_state: SecretString,
    pub client_id: String,
    pub client_secret: SecretString,
    pub token_url: String,
    pub redirect_uri: String,
    pub issued_at: DateTime<Utc>,
}

impl PendingState for OAuth2Pending {
    const KIND: &'static str = "oauth2_pending";
    fn expires_in(&self) -> Duration { Duration::from_secs(600) }
}
```

### PendingStateStore — ephemeral state lifecycle

```rust
/// Manages pending state for interactive credential flows.
/// Separate subsystem from CredentialStore — different lifecycle, TTL, security model.
///
/// **Security: 4-dimensional token binding.**
/// Store key = (credential_kind, owner_id, session_id, token_id).
/// All four validated on consume — prevents:
/// - Cross-credential type confusion (credential_kind mismatch)
/// - Cross-user token replay (owner_id mismatch)
/// - Session fixation / confused deputy (session_id mismatch)
/// - Token guessing (token_id = 32-byte CSPRNG)
///
/// Implementation requirements:
/// - Encrypt before persistence: serialize → Zeroizing<Vec<u8>> → encrypt → store
/// - Enforce TTL: auto-delete expired entries (background cleanup)
/// - Single-use: consume() = get + delete atomically
///
/// Deployment:
/// - **Single-node/dev:** InMemoryPendingStore (with encryption + TTL cleanup)
/// - **Multi-node/HA:** Redis/Postgres-backed (shared state for callback routing).
///   In-memory pending store WILL FAIL for multi-node because OAuth2/SAML
///   callbacks may route to a different node than the one that initiated resolve().
pub trait PendingStateStore: Send + Sync {
    /// Store pending state, return opaque token. Encrypts internally.
    /// Framework injects credential_kind (C::KEY) — credential author cannot control it.
    fn put<P: PendingState>(
        &self,
        credential_kind: &str,
        owner_id: &str,
        session_id: &str,
        pending: P,
    ) -> impl Future<Output = Result<PendingToken, CredentialError>> + Send;

    /// Read pending state without consuming (for polling flows like device code).
    fn get<P: PendingState>(
        &self,
        token: &PendingToken,
    ) -> impl Future<Output = Result<P, CredentialError>> + Send;

    /// Read and delete atomically (single-use).
    /// Validates credential_kind + owner_id + session_id — all must match stored values.
    fn consume<P: PendingState>(
        &self,
        credential_kind: &str,
        token: &PendingToken,
        owner_id: &str,
        session_id: &str,
    ) -> impl Future<Output = Result<P, CredentialError>> + Send;

    /// Explicit delete (cleanup on error paths).
    fn delete(
        &self,
        token: &PendingToken,
    ) -> impl Future<Output = Result<(), CredentialError>> + Send;
}
```

### CredentialContext — resolve-time environment

```rust
/// Read-only context provided by framework to credential methods.
/// Does NOT contain pending_store — framework handles PendingState lifecycle.
/// Credential methods receive &CredentialContext (not &mut).
pub struct CredentialContext {
    pub owner_id: String,
    pub caller_scope: Option<ScopeLevel>,
    pub trace_id: Uuid,
    pub timestamp: DateTime<Utc>,
    callback_url: String,
    app_url: String,
    session_id: String,
    /// Credential resolver for composition (AWS assume role → base credential).
    resolver: Option<Arc<dyn CredentialResolverRef>>,
}

impl CredentialContext {
    pub fn callback_url(&self) -> &str { &self.callback_url }
    pub fn app_url(&self) -> &str { &self.app_url }
    pub fn session_id(&self) -> &str { &self.session_id }

    /// Resolve another credential by ID (composition).
    /// AWS Assume Role resolves base AWS credential. LDAP GSSAPI resolves Kerberos.
    /// Max composition depth: configurable (default 3).
    ///
    /// Framework errors (ResolutionError) are mapped to CredentialError::CompositionFailed
    /// so the credential author doesn't need to handle framework-level error types.
    pub async fn resolve_credential<S: AuthScheme>(
        &self,
        credential_id: &str,
    ) -> Result<S, CredentialError> {
        let resolver = self.resolver.as_ref()
            .ok_or(CredentialError::CompositionNotAvailable)?;
        resolver.resolve(credential_id).await
            .map_err(|e| CredentialError::CompositionFailed {
                source: Box::new(e),
            })
    }
}
```

Note: `CredentialContext` is **read-only** (`&self`, not `&mut self`).
Credential methods cannot mutate context. Framework handles all state
management (pending store, credential store, refresh coordination).

### Capability Flags — associated consts

Capabilities are declared as associated consts on the Credential trait.
Stable on Rust 1.94, compile-time, zero allocation. Registry validates
consistency with actual method behavior via `debug_assert!`.

```rust
// Static credential — all defaults (false)
impl Credential for TelegramBotToken {
    const INTERACTIVE: bool = false;  // default
    const REFRESHABLE: bool = false;  // default
    const REVOCABLE: bool = false;    // default
    const TESTABLE: bool = true;      // overrides: has custom test()
    // ...
}

// OAuth2 credential — interactive + refreshable
impl Credential for GoogleSheetsOAuth2 {
    const INTERACTIVE: bool = true;
    const REFRESHABLE: bool = true;
    const REVOCABLE: bool = true;
    const TESTABLE: bool = false;
    // ...
}
```

Registry validation (debug builds):
```rust
fn register<C: Credential>(&mut self) {
    debug_assert!(
        !C::REFRESHABLE || /* C::refresh is not the default impl */,
        "Credential {} claims REFRESHABLE but uses default refresh()", C::KEY
    );
    // ... similar for INTERACTIVE, REVOCABLE, TESTABLE
}
```

### RefreshPolicy — per-credential refresh timing

```rust
/// Controls when and how the framework refreshes this credential.
/// Used as associated const on Credential trait: `const REFRESH_POLICY: RefreshPolicy`.
/// All fields are const-compatible (Duration::from_secs is const fn since Rust 1.53).
#[derive(Clone, Copy, Debug)]
pub struct RefreshPolicy {
    /// Refresh this long before expires_at(). Default: 5 minutes.
    pub early_refresh: Duration,
    /// Minimum backoff between retry attempts on refresh failure. Default: 5 seconds.
    pub min_retry_backoff: Duration,
    /// Add random jitter (0..jitter) to early_refresh to prevent thundering herd.
    pub jitter: Duration,
}

impl RefreshPolicy {
    pub const DEFAULT: Self = Self {
        early_refresh: Duration::from_secs(300),
        min_retry_backoff: Duration::from_secs(5),
        jitter: Duration::from_secs(30),
    };
}
```

### InteractionRequest — what the UI should do

```rust
pub enum InteractionRequest {
    /// Redirect user's browser to this URL (OAuth2 authorization code).
    Redirect { url: String },

    /// Auto-submit a POST form to IdP (SAML POST binding).
    FormPost {
        url: String,
        fields: Vec<(String, String)>,
    },

    /// Display information to user (device code, SMS code, TOTP).
    DisplayInfo {
        title: String,
        message: String,
        data: DisplayData,
        expires_in: Option<u64>,
    },
}

pub enum DisplayData {
    /// Device code flow: user types this code on another device.
    UserCode {
        code: String,
        verification_uri: String,
        verification_uri_complete: Option<String>,
    },
    /// Generic text display (instructions, QR codes, etc.)
    Text(String),
}
```

### UserInput — what the user/callback provides

```rust
pub enum UserInput {
    /// OAuth2 callback: GET with query parameters (code, state).
    Callback { params: HashMap<String, String> },

    /// SAML callback: POST with form data (SAMLResponse, RelayState).
    FormData { params: HashMap<String, String> },

    /// Device code flow: "check if authorized yet" (framework polls).
    Poll,

    /// User entered a code (SMS, TOTP, 2FA).
    Code { code: String },
}
```

### RefreshOutcome

```rust
/// Represents SUCCESSFUL or EXPECTED outcomes from refresh().
/// All FAILURES go through Err(CredentialError::RefreshFailed { kind, retry, source }).
pub enum RefreshOutcome {
    /// Token was refreshed successfully.
    Refreshed,
    /// Credential doesn't support refresh (permanent tokens, API keys).
    NotSupported,
    /// Refresh failed due to expected protocol behavior — needs full re-authentication.
    /// Framework triggers re-resolve with user interaction.
    /// Used by SAML (assertions expire, can't refresh — must re-authenticate).
    ReauthRequired,
}
```

### TestResult

```rust
pub enum TestResult {
    /// Credential works — authenticated successfully.
    Success,
    /// Credential failed — with reason.
    Failed { reason: String },
    /// Credential type doesn't support testing.
    Untestable,
}
```

### CredentialContext

See `CredentialContext` definition in the PendingStateStore section above.
It carries `pending_store`, `resolver`, `callback_url`, `owner_id`, etc.

### Static credential — simplest case

For credentials where form → auth material is pure (no async, no interaction):

```rust
use nebula_credential::{identity_state, Credential, BearerToken, NoPendingState};

// Explicit opt-in: BearerToken is both AuthScheme and CredentialState
identity_state!(BearerToken, "bearer", 1);

pub struct TelegramBotToken;

impl Credential for TelegramBotToken {
    type Scheme = BearerToken;
    type State = BearerToken;         // State = Scheme (identity path)
    type Pending = NoPendingState;    // explicit — no associated type default
    const KEY: CredentialKey = credential_key!("telegram-bot");

    const TESTABLE: bool = true;      // overrides default false

    fn description() -> CredentialDescription {
        CredentialDescription::new("telegram-bot", "Telegram Bot")
            .icon("telegram")
            .doc_url("https://core.telegram.org/bots#botfather")
    }

    fn parameters() -> ParameterCollection {
        ParameterCollection::new()
            .add(Parameter::string("bot_token")
                .label("Bot Token")
                .placeholder("123456:ABC-DEF...")
                .secret()
                .required()
                .with_rule(Rule::regex(r"^\d+:[A-Za-z0-9_-]+$")))
    }

    fn project(state: &BearerToken) -> BearerToken {
        state.clone() // State = Scheme → identity
    }

    async fn resolve(
        values: &ParameterValues,
        _ctx: &CredentialContext,
    ) -> Result<StaticResolveResult<BearerToken>, CredentialError> {
        let token = values.require_secret("bot_token")?;
        Ok(ResolveResult::Complete(BearerToken::new(token)))
    }

    async fn test(scheme: &BearerToken, _ctx: &CredentialContext) -> Result<TestResult, CredentialError> {
        let url = format!("https://api.telegram.org/bot{}/getMe", scheme.expose().as_str());
        match reqwest::get(&url).await {
            Ok(resp) if resp.status().is_success() => Ok(TestResult::Success),
            Ok(resp) => Ok(TestResult::Failed { reason: format!("HTTP {}", resp.status()) }),
            Err(e) => Ok(TestResult::Failed { reason: e.to_string() }),
        }
    }
}
```

### OAuth2 credential — interactive flow with State ≠ Scheme

```rust
pub struct GoogleSheetsOAuth2;

impl Credential for GoogleSheetsOAuth2 {
    type Scheme = OAuth2Token;   // what resources see (access_token, scopes, expiry)
    type State = OAuth2State;       // what's stored (+ refresh_token, client_secret, token_url)
    type Pending = OAuth2Pending;   // typed pending state for interactive flow
    const KEY: CredentialKey = credential_key!("google-sheets-oauth2");

    const INTERACTIVE: bool = true;
    const REFRESHABLE: bool = true;
    const REVOCABLE: bool = true;

    fn description() -> CredentialDescription {
        CredentialDescription::new("google-sheets-oauth2", "Google Sheets (OAuth2)")
            .icon("google-sheets")
    }

    fn parameters() -> ParameterCollection {
        ParameterCollection::new()
            .add(Parameter::string("client_id").label("Client ID").required())
            .add(Parameter::string("client_secret").label("Client Secret").secret().required())
            .add(Parameter::select("scopes")
                .label("Scopes")
                .multiple()
                .option("spreadsheets", "Read/Write Spreadsheets")
                .option("spreadsheets.readonly", "Read Only")
                .default(json!(["spreadsheets"])))
    }

    fn project(state: &OAuth2State) -> OAuth2Token {
        // Extract consumer-facing auth — refresh_token stays internal
        OAuth2Token {
            access_token: state.access_token.clone(),
            token_type: state.token_type.clone(),
            scopes: state.scopes.clone(),
            expires_at: state.expires_at,
        }
    }

    async fn resolve(
        values: &ParameterValues,
        ctx: &CredentialContext,
    ) -> Result<ResolveResult<OAuth2State, OAuth2Pending>, CredentialError> {
        let client_id = values.require_string("client_id")?;
        let client_secret = values.require_secret("client_secret")?;

        let (auth_url, pkce_verifier) = OAuth2Flow::authorization_code()
            .auth_url("https://accounts.google.com/o/oauth2/v2/auth")
            .client_id(&client_id)
            .scopes(&values.get_string_list("scopes").unwrap_or_default())
            .pkce()
            .redirect_url(ctx.callback_url())
            .build_auth_url()?;

        // Return raw pending state — framework handles storage, encryption, token generation
        Ok(ResolveResult::Pending {
            state: OAuth2Pending {
                pkce_verifier: SecretString::new(pkce_verifier),
                csrf_state: SecretString::new(auth_url.state().to_owned()),
                client_id: client_id.to_owned(),
                client_secret: SecretString::new(client_secret),
                token_url: "https://oauth2.googleapis.com/token".into(),
                redirect_uri: ctx.callback_url().to_owned(),
                issued_at: Utc::now(),
            },
            interaction: InteractionRequest::Redirect { url: auth_url.into() },
        })
    }

    async fn continue_resolve(
        pending: &OAuth2Pending,
        input: &UserInput,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<OAuth2State, OAuth2Pending>, CredentialError> {
        let UserInput::Callback { params } = input else {
            return Err(CredentialError::InvalidInput("expected OAuth2 callback".into()));
        };

        let code = params.get("code")
            .ok_or_else(|| CredentialError::InvalidInput("missing code parameter".into()))?;

        let tokens = OAuth2Flow::exchange_code()
            .token_url(&pending.token_url)
            .client_id(&pending.client_id)
            .client_secret(pending.client_secret.expose().as_str())
            .code(code)
            .pkce_verifier(pending.pkce_verifier.expose().as_str())
            .redirect_url(&pending.redirect_uri)
            .execute().await
            .map_err(|e| CredentialError::Provider(Box::new(e)))?;

        Ok(ResolveResult::Complete(OAuth2State {
            access_token: tokens.access_token,
            token_type: tokens.token_type.unwrap_or_else(|| "Bearer".into()),
            scopes: tokens.scopes,
            expires_at: tokens.expires_at,
            refresh_token: tokens.refresh_token,
            client_id: pending.client_id.clone(),
            client_secret: SecretString::new(pending.client_secret.expose().as_str().to_owned()),
            token_url: pending.token_url.clone(),
        }))
    }

    async fn refresh(
        state: &mut OAuth2State,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        let refresh_token = state.refresh_token.as_ref()
            .ok_or_else(|| CredentialError::RefreshFailed {
                kind: RefreshErrorKind::TokenExpired,
                retry: RetryAdvice::Never,
                source: "no refresh token".into(),
            })?;

        let tokens = OAuth2Flow::refresh()
            .token_url(&state.token_url)
            .client_id(&state.client_id)
            .client_secret(&state.client_secret)
            .refresh_token(refresh_token)
            .execute().await
            .map_err(|e| CredentialError::RefreshFailed {
                kind: RefreshErrorKind::TransientNetwork,
                retry: RetryAdvice::After(Duration::from_secs(30)),
                source: Box::new(e),
            })?;

        state.access_token = tokens.access_token;
        state.expires_at = tokens.expires_at;
        if let Some(rt) = tokens.refresh_token {
            state.refresh_token = Some(rt);
        }

        Ok(RefreshOutcome::Refreshed)
    }

    async fn revoke(
        state: &mut OAuth2State,
        _ctx: &CredentialContext,
    ) -> Result<(), CredentialError> {
        let token = state.refresh_token.as_ref()
            .unwrap_or(&state.access_token);
        reqwest::Client::new()
            .post("https://oauth2.googleapis.com/revoke")
            .form(&[("token", token.expose().as_str())])
            .send().await
            .map_err(|e| CredentialError::RevokeFailed { source: Box::new(e) })?;
        Ok(())
    }
}
```

### Protocol reuse — DRY for common patterns

Many credentials follow the same pattern. Protocols are reusable building blocks:

```rust
/// Reusable static protocol: parameters() + build() → AuthScheme.
pub trait StaticProtocol: Send + Sync + 'static {
    type Scheme: AuthScheme;

    fn parameters() -> ParameterCollection where Self: Sized;
    fn build(values: &ParameterValues) -> Result<Self::Scheme, CredentialError> where Self: Sized;
}

/// Reusable OAuth2 protocol — configure once per provider, reuse everywhere.
pub struct OAuth2Protocol { pub config: OAuth2Config }

impl OAuth2Protocol {
    pub fn google(scopes: &[&str]) -> Self { /* ... */ }
    pub fn github(scopes: &[&str]) -> Self { /* ... */ }
    pub fn microsoft(tenant: &str, scopes: &[&str]) -> Self { /* ... */ }
}
```

### Derive macro — zero boilerplate for simple credentials

```rust
/// Generates full Credential impl from a StaticProtocol.
#[derive(Credential)]
#[credential(
    key = "postgres",
    name = "PostgreSQL",
    icon = "postgres",
    scheme = DatabaseAuth,
    protocol = DatabaseProtocol,
)]
pub struct PostgresCredential;

/// Override parameter defaults per credential type.
#[derive(Credential)]
#[credential(
    key = "mysql",
    name = "MySQL",
    icon = "mysql",
    scheme = DatabaseAuth,
    protocol = DatabaseProtocol,
    defaults = { "port": 3306 },   // ← override Postgres default of 5432
)]
pub struct MySqlCredential;
```

#### Derive macro expansion rules

The macro generates a full `impl Credential` from attributes:

| Attribute | What macro generates |
|-----------|---------------------|
| `protocol = StaticProtocol` | `type Pending = NoPendingState` |
| (no `interactive` attr) | `const INTERACTIVE: bool = false` |
| (no `refreshable` attr) | `const REFRESHABLE: bool = false` |
| (no `revocable` attr) | `const REVOCABLE: bool = false` |
| (no `testable` attr) | `const TESTABLE: bool = false` |
| (no `refresh_policy` attr) | `const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT` |
| `defaults = { "port": 3306 }` | Applies defaults to protocol's `parameters()` before `build()` |

Override example:
```rust
#[derive(Credential)]
#[credential(
    key = "google-oauth2",
    name = "Google OAuth2",
    scheme = OAuth2Token,
    protocol = OAuth2Protocol,
    interactive = true,
    refreshable = true,
    revocable = true,
    pending = OAuth2Pending,
    refresh_policy = { early_refresh = 300, jitter = 30 },
)]
pub struct GoogleOAuth2Credential;
```

The macro rejects conflicting attrs at expansion time:
- `interactive = true` requires `pending = SomeType` (not NoPendingState)
- `protocol = StaticProtocol` + `interactive = true` is a compile error

### Credential author summary

| Complexity | What to do | Example |
|-----------|------------|---------|
| Simple static | `#[derive(Credential)]` + protocol | PostgresCredential, RedisCredential |
| Simple custom | Implement `Credential` with `resolve()` → `Complete` | TelegramBotToken |
| Interactive | Implement `Credential` with `continue_resolve()` | GoogleSheetsOAuth2 |
| Composed | Use `ctx.resolve_credential()` in `resolve()` | AwsAssumeRole, LdapGssapi |
| Custom protocol | Create a `StaticProtocol` impl, reuse via derive | LDAP, SMTP |

---

## AuthScheme Design

### The AuthScheme trait

```rust
/// Typed auth material consumed by resources.
/// Resources declare `type Auth: AuthScheme` to specify what they consume.
/// Credentials declare `type Scheme: AuthScheme` to specify what they produce.
///
/// ## Security contract
///
/// `Serialize + DeserializeOwned` bounds exist for the `State = Scheme` identity
/// path (static credentials stored directly). Serialization to plaintext JSON
/// happens **exclusively** inside `EncryptionLayer`. Never serialize AuthScheme
/// types in logging, debugging, IPC, telemetry, or test snapshots.
///
/// All implementations MUST:
/// - Implement `Debug` manually with redacted secrets (never derive Debug)
/// - Use `SecretString` for all secret fields
/// - Use `ZeroizeOnDrop` where applicable
pub trait AuthScheme: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {
    /// Unique key for this scheme. "bearer", "basic", "database", "ssh".
    const KIND: &'static str;

    /// Whether this scheme has expiring material that can be refreshed.
    /// Framework uses this to schedule auto-refresh.
    fn expires_at(&self) -> Option<DateTime<Utc>> { None }
}
```

### CredentialState trait

```rust
/// Trait for stored credential state.
/// May equal AuthScheme (static credentials) or contain additional
/// internal data (refresh tokens, client secrets, KDC config).
///
/// NOTE: No blanket impl from AuthScheme. Static credentials must
/// explicitly opt in via identity_state! macro. This prevents
/// accidentally persisting consumer-facing runtime material.
pub trait CredentialState: Serialize + DeserializeOwned + Send + Sync + Clone + 'static {
    /// Schema version for state migrations.
    const VERSION: u16;
    /// Unique kind identifier.
    const KIND: &'static str;
    /// Zeroize ephemeral secrets before persistence (optional).
    fn scrub_ephemeral(&mut self) {}
}

/// Explicitly opt a type into being both AuthScheme and CredentialState.
/// Used for static credentials where State = Scheme (no separate stored state).
/// This is an intentional act, not an automatic blanket.
#[macro_export]
macro_rules! identity_state {
    ($ty:ty, $kind:literal, $version:expr) => {
        impl CredentialState for $ty {
            const VERSION: u16 = $version;
            const KIND: &'static str = $kind;
        }
    };
}

// Built-in identity states (State = Scheme):
identity_state!(BearerToken, "bearer", 1);
identity_state!(BasicAuth, "basic", 1);
identity_state!(DatabaseAuth, "database", 1);
identity_state!(ApiKeyAuth, "api_key", 1);
identity_state!(HeaderAuth, "header", 1);
identity_state!(CertificateAuth, "certificate", 1);
identity_state!(SshAuth, "ssh", 1);
identity_state!(LdapAuth, "ldap", 1);
identity_state!(HmacSecret, "hmac", 1);
```

### Built-in schemes

All built-in schemes implement `Debug` with redacted secrets. Never
derive `Debug` on types containing `SecretString` — always hand-write
(or use `#[derive(SecretModel)]` in Phase 3).

#### SecretGuard — type-safe secret exposure

```rust
/// Typed guard returned by SecretString::expose().
/// Implements Deref<Target=str> for .len(), .contains(), etc.
/// Display and Debug print "<REDACTED>" — prevents accidental logging.
/// Use .as_str() for explicit protocol transmission (HTTP headers, connection strings).
pub struct SecretGuard<'a>(&'a str);

impl<'a> Deref for SecretGuard<'a> {
    type Target = str;
    fn deref(&self) -> &str { self.0 }
}

impl<'a> fmt::Display for SecretGuard<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<REDACTED>")
    }
}

impl<'a> fmt::Debug for SecretGuard<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretGuard(<REDACTED>)")
    }
}

impl<'a> SecretGuard<'a> {
    /// Explicitly bypass redaction for protocol transmission.
    /// This method name stands out in code review.
    pub fn as_str(&self) -> &'a str { self.0 }
}

impl SecretString {
    /// Returns a SecretGuard with redacted Display/Debug.
    /// Safe to pass to format!(), println!(), tracing — shows "<REDACTED>".
    /// For actual secret value: .expose().as_str()
    pub fn expose(&self) -> SecretGuard<'_> {
        SecretGuard(self.inner.as_str())
    }

    /// Constant-time equality comparison. Use for secret comparisons.
    /// Regular == is NOT constant-time — do not use for secrets.
    pub fn ct_eq(&self, other: &SecretString) -> bool {
        subtle::ConstantTimeEq::ct_eq(
            self.inner.as_bytes(),
            other.inner.as_bytes(),
        ).into()
    }
}
```

**Accidental logging is now safe by default:**
```rust
tracing::info!("token: {}", token.expose());          // prints: <REDACTED>
let header = format!("Bearer {}", token.expose().as_str()); // explicit, intentional
```

#### Built-in AuthScheme types

```rust
// BearerToken — bearer/API key auth
#[derive(Clone, Serialize, Deserialize)]
pub struct BearerToken { token: SecretString }

impl BearerToken {
    pub fn new(token: SecretString) -> Self { Self { token } }
    pub fn bearer_header(&self) -> String {
        format!("Bearer {}", self.token.expose().as_str())
    }
    pub fn expose(&self) -> SecretGuard<'_> { self.token.expose() }
}
impl AuthScheme for BearerToken { const KIND: &'static str = "bearer"; }

// Redacted Debug — REQUIRED for all types with SecretString fields.
// Phase 3: #[derive(SecretModel)] auto-generates this.
impl fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BearerToken").field("token", &"<redacted>").finish()
    }
}

// BasicAuth — username + password
#[derive(Clone, Serialize, Deserialize)]
pub struct BasicAuth { pub username: String, password: SecretString }

impl BasicAuth {
    pub fn encoded(&self) -> String {
        BASE64.encode(format!("{}:{}", self.password.expose().as_str()))
    }
    pub fn authorization_header(&self) -> String { format!("Basic {}", self.encoded()) }
}
impl AuthScheme for BasicAuth { const KIND: &'static str = "basic"; }

// DatabaseAuth — full database connection auth
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseAuth {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    password: SecretString,
    pub ssl_mode: SslMode,
    pub tls: Option<TlsAuth>,   // optional client cert for database mTLS
}

impl DatabaseAuth {
    pub fn connection_string(&self) -> SecretString { /* ... */ }
    pub fn password(&self) -> &str { self.password.expose() }
}
impl AuthScheme for DatabaseAuth { const KIND: &'static str = "database"; }

// ApiKeyAuth — API key with placement (header or query param)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyAuth {
    key: SecretString,
    pub placement: ApiKeyPlacement,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApiKeyPlacement {
    Header { name: String, prefix: String },
    QueryParam { name: String },
}

impl ApiKeyAuth {
    pub fn apply_to_request(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder { /* ... */ }
    pub fn key(&self) -> &str { self.key.expose() }
}
impl AuthScheme for ApiKeyAuth { const KIND: &'static str = "api_key"; }

// OAuth2Token — consumer-facing OAuth2 auth (NO refresh internals)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Token {
    access_token: SecretString,
    pub token_type: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    // NOTE: NO refresh_token here — that's in OAuth2State (internal)
}

impl OAuth2Token {
    pub fn bearer_header(&self) -> String { format!("{} {}", self.token_type, self.access_token.expose()) }
    pub fn access_token(&self) -> &str { self.access_token.expose() }
}
impl AuthScheme for OAuth2Token {
    const KIND: &'static str = "oauth2";
    fn expires_at(&self) -> Option<DateTime<Utc>> { self.expires_at }
}

// SshAuth — SSH auth with connection info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshAuth {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub method: SshAuthMethod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SshAuthMethod {
    Password { password: SecretString },
    KeyPair { private_key: SecretString, passphrase: Option<SecretString> },
    Agent,
}
impl AuthScheme for SshAuth { const KIND: &'static str = "ssh"; }

// LdapAuth — LDAP bind credentials with connection info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapAuth {
    pub host: String,
    pub port: u16,
    pub tls_mode: LdapTlsMode,
    pub base_dn: Option<String>,
    pub bind_method: LdapBindMethod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LdapBindMethod {
    Simple { bind_dn: String, password: SecretString },
    Anonymous,
    SaslGssapi { principal: String, kerberos_credential_id: Option<String> },
    SaslExternal { certificate_credential_id: Option<String> },
}
impl AuthScheme for LdapAuth { const KIND: &'static str = "ldap"; }

// SamlAuth — SAML assertion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamlAuth {
    pub name_id: String,
    pub session_index: Option<String>,
    pub attributes: HashMap<String, Vec<String>>,
    pub not_on_or_after: Option<DateTime<Utc>>,
    assertion_b64: Option<SecretString>,
}
impl AuthScheme for SamlAuth {
    const KIND: &'static str = "saml";
    fn expires_at(&self) -> Option<DateTime<Utc>> { self.not_on_or_after }
}

// KerberosAuth — Kerberos service ticket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KerberosAuth {
    pub principal: String,
    pub realm: String,
    service_ticket: SecretString,
    pub expires_at: DateTime<Utc>,
}
impl AuthScheme for KerberosAuth {
    const KIND: &'static str = "kerberos";
    fn expires_at(&self) -> Option<DateTime<Utc>> { Some(self.expires_at) }
}

// AwsAuth, CertificateAuth, HeaderAuth, HmacSecret — as before

// NoAuth
impl AuthScheme for () { const KIND: &'static str = "none"; }
```

### Scheme coercion — TryFrom for compatibility

AuthScheme coercion allows a credential producing one scheme to work with a
resource expecting a compatible scheme. Coercion is **fallible** via `TryFrom`:

```rust
// OAuth2Token → BearerToken (always succeeds)
impl From<OAuth2Token> for BearerToken {
    fn from(m: OAuth2Token) -> Self {
        BearerToken::new(m.access_token)
    }
}

// ApiKeyAuth → BearerToken (only for Authorization:Bearer header placement)
impl TryFrom<ApiKeyAuth> for BearerToken {
    type Error = CredentialError;
    fn try_from(api: ApiKeyAuth) -> Result<BearerToken, CredentialError> {
        match &api.placement {
            ApiKeyPlacement::Header { name, prefix }
                if name.eq_ignore_ascii_case("authorization")
                    && prefix.to_lowercase().starts_with("bearer") =>
            {
                Ok(BearerToken::new(api.key))
            }
            _ => Err(CredentialError::SchemeMismatch {
                expected: "bearer",
                actual: "api_key (non-bearer placement)".into(),
            }),
        }
    }
}

// SamlAuth → BearerToken (only if assertion_b64 is present)
impl TryFrom<SamlAuth> for BearerToken {
    type Error = CredentialError;
    fn try_from(saml: SamlAuth) -> Result<BearerToken, CredentialError> {
        match saml.assertion_b64 {
            Some(assertion) => Ok(BearerToken::new(assertion)),
            None => Err(CredentialError::SchemeMismatch {
                expected: "bearer",
                actual: "saml (no assertion data)".into(),
            }),
        }
    }
}
```

Framework checks `TryFrom` compatibility at credential-to-resource binding time.
The primary mechanism is `project()` — coercion is a convenience fallback.

---

## Resource Integration View

This section is for engineers implementing **resources** that need credentials.

### Declaring credential requirements

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;
    type Lease: Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static;
    type Auth: AuthScheme;         // ← auth material this resource needs
    const KEY: ResourceKey;

    fn create(
        &self,
        config: &Self::Config,
        auth: &Self::Auth,         // ← typed, not JSON
        ctx: &dyn Ctx,             // ← carries CredentialResolver for composition
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    // ... check, shutdown, destroy unchanged
}
```

**`Ctx` carries `CredentialResolver`** — resources that use composed
credentials (LDAP GSSAPI needs Kerberos ticket) can resolve them at
create time via `ctx.ext::<CredentialResolver>()`.

### Credential rotation — how resources react

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationStrategy {
    /// Call authorize() on live instances (HTTP clients: swap bearer token).
    HotSwap,
    /// Drain pool, create new instances (database: new password = new TCP).
    DrainAndRecreate,
    /// Destroy all immediately, next acquire creates fresh (SSH: key change).
    Reconnect,
}

pub trait HotSwappable: Resource {
    fn authorize(&self, runtime: &Self::Runtime, auth: &Self::Auth)
        -> impl Future<Output = Result<(), Self::Error>> + Send;
}
```

### Type flow — three-step resolution

```
                        ┌──────────────────────┐
                        │   Credential Store    │
                        │  (encrypted State)    │
                        └──────────┬───────────┘
                                   │ load + decrypt
                                   ▼
                        ┌──────────────────────┐
                        │  Credential::State    │
                        │  (OAuth2State with    │
                        │   refresh_token)      │
                        └──────────┬───────────┘
                                   │ project()
                                   ▼
                        ┌──────────────────────┐
                        │ Credential::Scheme    │
                        │  (OAuth2Token)     │
                        └──────────┬───────────┘
                                   │ TryInto (if needed)
                                   ▼
                        ┌──────────────────────┐
                        │   Resource::Auth      │
                        │  (BearerToken)        │
                        └──────────────────────┘
```

Static credentials: State = Scheme = Auth (one step).
OAuth2 → resource wanting BearerToken: State → project() → OAuth2Token → Into → BearerToken.

---

## CredentialHandle — auto-refreshing typed handle

When actions access credentials directly via `ctx.credential::<S>()`, they
receive a `CredentialHandle<S>`:

```rust
pub struct CredentialHandle<S: AuthScheme> {
    scheme: ArcSwap<S>,
    credential_id: CredentialId,
    acquired_at: Instant,
}

impl<S: AuthScheme> CredentialHandle<S> {
    /// Get a snapshot of the current auth material.
    /// Returns Arc<S> — caller holds a reference-counted copy.
    /// If refresh happens after snapshot(), this copy remains valid
    /// until dropped. Next snapshot() returns the refreshed value.
    pub fn snapshot(&self) -> Arc<S> {
        self.scheme.load_full()
    }

    /// Used by RefreshCoordinator to swap in refreshed material.
    pub(crate) fn replace(&self, next: Arc<S>) {
        self.scheme.store(next);
    }

    pub fn credential_id(&self) -> &CredentialId {
        &self.credential_id
    }
}
```

**Why not `Deref`?** `ArcSwap::load()` returns a temporary `Guard` — returning
`&S` from `Deref` creates a dangling reference. `snapshot()` returns `Arc<S>`,
which is correct, explicit, and easy to reason about under concurrent refresh.

Usage in actions:

```rust
let auth = ctx.credential::<BearerToken>().await?.snapshot();
http.header("Authorization", auth.bearer_header());
// auth is Arc<BearerToken> — valid even if refresh happens concurrently
```

Auto-refresh is transparent via `ArcSwap`. When OAuth2/Kerberos/AWS token
approaches expiry (per `Credential::refresh_policy()`), `RefreshCoordinator`
calls `Credential::refresh()` on the State, re-projects to Scheme, and
calls `handle.replace()`. Subsequent `snapshot()` calls return refreshed value.

---

## Layered Storage

### CredentialStore — the core trait

```rust
/// Write mode for put operations.
pub enum PutMode {
    /// Fail if credential already exists.
    CreateOnly,
    /// Overwrite unconditionally (initial setup, admin override).
    Overwrite,
    /// Compare-and-swap: succeed only if stored version matches expected.
    /// Used by RefreshCoordinator to prevent concurrent refresh races.
    CompareAndSwap { expected_version: u64 },
}

pub trait CredentialStore: Send + Sync {
    /// Returns the committed StoredCredential (canonical, with server-side version/timestamps).
    fn put(
        &self,
        id: &CredentialId,
        entry: &StoredCredential,
        mode: PutMode,
    ) -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;

    fn get(
        &self,
        id: &CredentialId,
    ) -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;

    fn delete(
        &self,
        id: &CredentialId,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;

    fn list(
        &self,
        filter: &ListFilter,
    ) -> impl Future<Output = Result<Vec<CredentialEntry>, StoreError>> + Send;

    fn exists(
        &self,
        id: &CredentialId,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;
}
```

5 methods, native async (no `#[async_trait]`). `PutMode` makes CAS explicit
in the trait contract. `put()` returns the committed `StoredCredential` —
canonical state from backend (handles server-side versioning, timestamps).

### StoredCredential — what's persisted

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCredential {
    /// Credential state kind (for type dispatch).
    pub state_kind: String,
    /// Scheme kind (for compatibility checks).
    pub scheme_kind: String,
    /// State schema version at serialize time (for migrations).
    pub state_version: u16,
    /// Serialized State (encrypted by EncryptionLayer).
    pub data: Vec<u8>,
    /// Metadata (includes lifecycle).
    pub metadata: CredentialMetadata,
    /// Version for CAS operations. Incremented on every write.
    pub version: u64,
}

/// Credential lifecycle — persisted status for distributed coordination.
/// CAS losers read this to determine if the winner refreshed successfully
/// or marked the credential as dead/needing re-auth.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq)]
pub enum CredentialLifecycle {
    /// Credential is active and usable.
    Active,
    /// Credential needs full re-authentication (expired refresh token, SAML).
    /// Framework should trigger re-resolve with user interaction.
    ReauthRequired,
    /// Credential is terminally broken (revoked, account disabled).
    /// Framework stops refresh attempts, notifies admin.
    Terminal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMetadata {
    pub owner_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub lifecycle: CredentialLifecycle,
}
```

### Storage layers — compose freely

Layer order matters. Read direction: outer → inner. Write direction: outer → inner.

```rust
let store = LocalFileStore::new(data_dir)
    .layer(CacheLayer::ciphertext(cache_config))   // caches encrypted bytes only
    .layer(EncryptionLayer::new(master_key))        // decrypt on read, encrypt on write
    .layer(AuditLayer::redacted(audit_sink))        // logs access, never sees plaintext
    .layer(ScopeLayer::new(scope_resolver));        // outermost: fail fast on wrong tenant
```

**Invariants:**
- `ScopeLayer` (outermost): checked before any data access
- `AuditLayer`: receives redacted metadata only, never plaintext secrets
- `EncryptionLayer`: boundary between plaintext and ciphertext
- `CacheLayer`: stores **ciphertext** only — no plaintext in cache heap
- AES-256-GCM decryption cost per-read: ~1-3μs with AES-NI (negligible)

**Cache invalidation:** `CacheLayer` intercepts `put()` and `delete()` to
invalidate cached entries immediately. No stale credential served after rotation.

### Storage backends

| Backend | Feature flag | Use case |
|---------|-------------|----------|
| `InMemoryStore` | (always) | Tests, ephemeral environments |
| `LocalFileStore` | `storage-local` | Desktop app, single-node dev |
| `PostgresStore` | `storage-postgres` | Production multi-node |
| `VaultStore` | `storage-vault` | HashiCorp Vault KV engine |
| `AwsSecretsStore` | `storage-aws` | AWS Secrets Manager |
| `K8sSecretsStore` | `storage-k8s` | Kubernetes Secrets |

---

## CredentialResolver — runtime orchestrator

```rust
pub struct CredentialResolver {
    store: Arc<dyn CredentialStore>,
    refresh: RefreshCoordinator,
    rotation_bus: Arc<EventBus<CredentialRotatedEvent>>,
    registry: CredentialRegistry,
    /// Configurable composition depth. Default: 3. No unlimited.
    max_composition_depth: usize,
}

impl CredentialResolver {
    pub async fn resolve<S: AuthScheme>(
        &self,
        credential_id: &CredentialId,
    ) -> Result<S, CredentialError> {
        self.resolve_with_depth(credential_id, 0).await
    }

    async fn resolve_with_depth<S: AuthScheme>(
        &self,
        credential_id: &CredentialId,
        depth: usize,
    ) -> Result<S, CredentialError> {
        if depth >= self.max_composition_depth {
            return Err(CredentialError::CompositionDepthExceeded {
                credential_id: credential_id.clone(),
                depth,
                max: self.max_composition_depth,
            });
        }

        // 1. Load from store
        let stored = self.store.get(credential_id).await
            .map_err(|e| CredentialError::Resolution {
                credential_id: credential_id.clone(),
                stage: ResolutionStage::LoadState,
                source: Box::new(e),
            })?;

        // 2. Verify scheme compatibility
        if stored.scheme_kind != S::KIND {
            return Err(CredentialError::SchemeMismatch {
                credential_id: credential_id.clone(),
                expected: S::KIND,
                actual: stored.scheme_kind.clone(),
            });
        }

        // 3. Deserialize State, project to Scheme
        let handler = self.registry.get(&stored.state_kind)?;
        let scheme: S = handler.load_and_project(&stored.data)
            .map_err(|e| CredentialError::Resolution {
                credential_id: credential_id.clone(),
                stage: ResolutionStage::ProjectScheme,
                source: Box::new(e),
            })?;

        // 4. Auto-refresh if expiring (uses per-credential RefreshPolicy)
        if let Some(expires_at) = scheme.expires_at() {
            let policy = handler.refresh_policy();
            let jitter = rand_jitter(policy.jitter);
            let threshold = policy.early_refresh + jitter;
            if expires_at - Utc::now() < threshold {
                return self.refresh.refresh_if_needed(
                    credential_id, &stored, &self.registry, depth
                ).await;
            }
        }

        Ok(scheme)
    }
}
```

### RefreshCoordinator — hardened against hostile plugins

In-process coordination via DashMap entry API. Multi-node safety via CAS on store.
Lifecycle-aware. **Hardened against:** hung plugins, panics, hot-loop DoS.

```rust
pub struct RefreshCoordinator {
    /// In-process fast path. Uses entry() API for atomic state transitions.
    /// NOTE: Single-node coordination only. Multi-node via PutMode::CompareAndSwap.
    locks: DashMap<CredentialId, RefreshState>,
    /// Per-credential failure counter for circuit breaker.
    failure_counts: DashMap<CredentialId, (u32, Instant)>,
}

enum RefreshState {
    Idle,
    Refreshing(Arc<Notify>),
    Failed { error: String, retry_after: Instant },
}

/// Circuit breaker: stop refreshing after N consecutive failures in time window.
const MAX_REFRESH_FAILURES: u32 = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(300); // 5 min

impl RefreshCoordinator {
    /// Atomic Idle→Refreshing transition via DashMap::entry().
    /// Never replaces Refreshing with Refreshing (prevents Notify loss → hang).
    async fn refresh_if_needed<S: AuthScheme>(
        &self,
        id: &CredentialId,
        stored: &StoredCredential,
        registry: &CredentialRegistry,
        depth: usize,
    ) -> Result<S, ResolutionErrorKind> {
        // Circuit breaker check
        if let Some(entry) = self.failure_counts.get(id) {
            let (count, since) = entry.value();
            if *count >= MAX_REFRESH_FAILURES && since.elapsed() < FAILURE_WINDOW {
                return Err(CredentialError::RefreshFailed {
                    kind: RefreshErrorKind::ProtocolError,
                    retry: RetryAdvice::After(FAILURE_WINDOW),
                    source: "circuit breaker: too many consecutive failures".into(),
                }.into());
            }
        }

        let entry = self.locks.entry(id.clone());
        match entry.or_insert(RefreshState::Idle).get() {
            RefreshState::Refreshing(notify) => {
                let notify = notify.clone();
                drop(entry);
                // Waiter timeout: don't wait forever (60s max)
                match tokio::time::timeout(
                    Duration::from_secs(60),
                    notify.notified(),
                ).await {
                    Ok(()) => {}
                    Err(_) => {
                        // Timeout waiting — re-read from store anyway
                        tracing::warn!(credential_id = %id, "refresh waiter timed out");
                    }
                }
                return self.load_after_refresh(id, registry).await;
            }
            RefreshState::Failed { retry_after, .. } if Instant::now() < *retry_after => {
                drop(entry);
                return Err(CredentialError::RefreshFailed {
                    kind: RefreshErrorKind::ProviderUnavailable,
                    retry: RetryAdvice::After(
                        retry_after.duration_since(Instant::now())
                    ),
                    source: "in backoff window".into(),
                }.into());
            }
            _ => {
                // Idle or past retry_after — claim the refresh
                let notify = Arc::new(Notify::new());
                entry.insert(RefreshState::Refreshing(notify.clone()));
                drop(entry);

                // scopeguard: ALWAYS notify_waiters on any exit (success, error, timeout, panic)
                let _guard = scopeguard::guard(notify.clone(), |n| n.notify_waiters());

                // Framework timeout: 30s hard limit on credential refresh
                let result = tokio::time::timeout(
                    Duration::from_secs(30),
                    self.do_refresh(id, stored, registry),
                ).await.unwrap_or_else(|_| Err(CredentialError::RefreshFailed {
                    kind: RefreshErrorKind::ProviderUnavailable,
                    retry: RetryAdvice::After(Duration::from_secs(60)),
                    source: "framework timeout: refresh took too long".into(),
                }));

                // Clamp RetryAdvice from credential — enforce minimum backoff
                let result = result.map_err(|e| self.clamp_retry(id, e));

                // Update state
                match &result {
                    Ok(_) => {
                        self.locks.insert(id.clone(), RefreshState::Idle);
                        self.failure_counts.remove(id);
                    }
                    Err(e) => {
                        let retry_after = match e.retry_advice() {
                            Some(RetryAdvice::After(d)) => Instant::now() + d,
                            _ => Instant::now() + Duration::from_secs(30),
                        };
                        self.locks.insert(id.clone(), RefreshState::Failed {
                            error: e.to_string(), retry_after,
                        });
                        // Increment circuit breaker
                        self.failure_counts
                            .entry(id.clone())
                            .and_modify(|(c, _)| *c += 1)
                            .or_insert((1, Instant::now()));
                    }
                }

                // _guard dropped here → notify_waiters() called automatically
                return result;
            }
        }
    }

    /// Clamp RetryAdvice: framework enforces minimum backoff, prevents hot-loop DoS.
    fn clamp_retry(&self, id: &CredentialId, mut err: CredentialError) -> CredentialError {
        if let CredentialError::RefreshFailed { ref mut retry, .. } = err {
            let min_backoff = Duration::from_secs(5); // framework minimum
            match retry {
                RetryAdvice::Immediate => *retry = RetryAdvice::After(min_backoff),
                RetryAdvice::After(d) if *d < min_backoff => *retry = RetryAdvice::After(min_backoff),
                _ => {}
            }
        }
        err
    }

    /// CAS write with lifecycle-aware loser handling.
    async fn write_refreshed(
        &self,
        store: &dyn CredentialStore,
        id: &CredentialId,
        current: &StoredCredential,
        refreshed_data: Vec<u8>,
        new_lifecycle: CredentialLifecycle,
    ) -> Result<StoredCredential, ResolutionErrorKind> {
        let mut next = current.clone();
        next.version = current.version + 1;
        next.data = refreshed_data;
        next.metadata.lifecycle = new_lifecycle;
        next.metadata.updated_at = Utc::now();

        match store.put(id, &next, PutMode::CompareAndSwap {
            expected_version: current.version,
        }).await {
            Ok(written) => Ok(written),
            Err(StoreError::Conflict { .. }) => {
                let latest = store.get(id).await.map_err(ResolutionErrorKind::Store)?;
                match latest.metadata.lifecycle {
                    CredentialLifecycle::Active => Ok(latest),
                    CredentialLifecycle::ReauthRequired => Err(CredentialError::RefreshFailed {
                        kind: RefreshErrorKind::TokenExpired,
                        retry: RetryAdvice::Never,
                        source: "credential needs re-authentication".into(),
                    }.into()),
                    CredentialLifecycle::Terminal => Err(CredentialError::RefreshFailed {
                        kind: RefreshErrorKind::TokenRevoked,
                        retry: RetryAdvice::Never,
                        source: "credential is terminal".into(),
                    }.into()),
                }
            }
            Err(e) => Err(ResolutionErrorKind::Store(e)),
        }
    }
}
```

### ResolutionStage — where in the pipeline did it fail?

```rust
/// Encodes the failing stage for debugging three-step resolution.
#[derive(Debug, Clone, Copy)]
pub enum ResolutionStage {
    LoadState,
    Decrypt,
    DeserializeState,
    ProjectScheme,
    CoerceToResourceAuth,
    Refresh,
}
```

---

## Error Design

**Two-layer error model:** Credential authors write `CredentialError` (domain errors,
no credential_id). Framework wraps into `ResolutionError` with context (credential_id,
stage). This eliminates boilerplate — authors don't thread IDs through every error.

Boxing uses `Send` only (no `Sync` — some error types may not be Sync).

### CredentialError — author-facing (no credential_id)

```rust
use std::error::Error as StdError;

/// Domain errors returned by credential implementations.
/// No credential_id — framework adds context via ResolutionError.
///
/// Credential authors construct these directly:
///   Err(CredentialError::InvalidInput("missing code".into()))
///   Err(CredentialError::RefreshFailed { kind, retry, source })
#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("validation failed: {0}")]
    ValidationFailed(String),

    /// All refresh failures. RefreshOutcome only has success/expected states.
    #[error("refresh failed: {kind:?}")]
    RefreshFailed {
        kind: RefreshErrorKind,
        retry: RetryAdvice,
        #[source]
        source: Box<dyn StdError + Send + 'static>,
    },

    #[error("revoke failed")]
    RevokeFailed {
        #[source]
        source: Box<dyn StdError + Send + 'static>,
    },

    #[error("not interactive")]
    NotInteractive,

    #[error("credential composition not available")]
    CompositionNotAvailable,

    /// Composed credential (e.g., base AWS credential for Assume Role) failed.
    /// Used when ctx.resolve_credential() returns an error.
    #[error("composition failed")]
    CompositionFailed {
        #[source]
        source: Box<dyn StdError + Send + 'static>,
    },

    #[error("provider error: {0}")]
    Provider(Box<dyn StdError + Send + 'static>),
}

/// Helper: construct Box<dyn StdError + Send> from a string message.
/// Use in RefreshFailed { source }, RevokeFailed { source }, etc.
///
/// ```rust
/// Err(CredentialError::RefreshFailed {
///     kind: RefreshErrorKind::TokenExpired,
///     retry: RetryAdvice::Never,
///     source: error_source("no refresh token available"),
/// })
/// ```
pub fn error_source(msg: impl Into<String>) -> Box<dyn StdError + Send + 'static> {
    Box::new(SimpleError(msg.into()))
}

#[derive(Debug)]
pub struct SimpleError(pub String);
impl fmt::Display for SimpleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{}", self.0) }
}
impl StdError for SimpleError {}

/// Convenience constructors for common error patterns.
impl CredentialError {
    /// Shorthand for RefreshFailed with string source.
    pub fn refresh(
        kind: RefreshErrorKind,
        retry: RetryAdvice,
        msg: impl Into<String>,
    ) -> Self {
        Self::RefreshFailed { kind, retry, source: error_source(msg) }
    }
}
```

### ResolutionError — framework-facing (with context)

```rust
/// Framework error wrapping CredentialError with system context.
/// Constructed by framework, not by credential authors.
#[derive(Debug, thiserror::Error)]
#[error("credential {credential_id} failed at {stage:?}")]
pub struct ResolutionError {
    pub credential_id: CredentialId,
    pub stage: ResolutionStage,
    #[source]
    pub source: ResolutionErrorKind,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolutionErrorKind {
    /// Error from credential implementation.
    #[error(transparent)]
    Credential(#[from] CredentialError),

    /// Scheme mismatch at binding time.
    #[error("scheme mismatch: expected {expected}, got {actual}")]
    SchemeMismatch {
        expected: &'static str,
        actual: String,
    },

    /// Multi-tenant access denied.
    #[error("scope violation: requires {required_scope}")]
    ScopeViolation { required_scope: String },

    /// Composition depth exceeded.
    #[error("composition depth exceeded: depth {depth}, max {max}")]
    CompositionDepthExceeded { depth: usize, max: usize },

    /// Credential not found in store.
    #[error("not found")]
    NotFound,

    /// Storage-level error.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Encodes the failing stage for pipeline debugging.
#[derive(Debug, Clone, Copy)]
pub enum ResolutionStage {
    LoadState,
    Decrypt,
    DeserializeState,
    ProjectScheme,
    CoerceToResourceAuth,
    Resolve,
    ContinueResolve,
    Refresh,
    Revoke,
}

impl ResolutionError {
    pub fn new(id: CredentialId, stage: ResolutionStage, source: impl Into<ResolutionErrorKind>) -> Self {
        Self { credential_id: id, stage, source: source.into() }
    }
}
```

### Retry and refresh error classification

```rust
/// Retry advice attached to refresh failures.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RetryAdvice {
    /// Do not retry. Terminal failure.
    Never,
    /// Retry immediately (transient).
    Immediate,
    /// Retry after specified duration.
    After(Duration),
}

/// Classifies refresh failure for programmatic handling.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RefreshErrorKind {
    TransientNetwork,
    RateLimited,
    ProviderUnavailable,
    PermissionDenied,
    TokenRevoked,
    TokenExpired,
    /// OAuth2 "invalid_grant" — ambiguous. Framework decides based on policy.
    InvalidGrant,
    ProtocolError,
}
```

### StoreError

```rust
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("credential not found: {id}")]
    NotFound { id: CredentialId },

    #[error("I/O error")]
    Io(#[source] std::io::Error),

    #[error("serialization error")]
    Serialization(#[source] serde_json::Error),

    #[error("conflict on {id}: expected version {expected}, actual {actual}")]
    Conflict {
        id: CredentialId,
        expected: u64,
        actual: u64,
    },

    #[error("{backend} backend error")]
    Backend {
        backend: &'static str,
        #[source]
        source: Box<dyn StdError + Send + 'static>,
    },
}
```

### Refresh error handling cheatsheet

```
Credential author returns:
  Ok(Refreshed)                    → framework updates state
  Ok(ReauthRequired)               → framework triggers re-resolve
  Err(CredentialError::RefreshFailed { kind, retry, source }) →
    Framework wraps in ResolutionError, then:
    kind=TransientNetwork, retry=After(30s) → backoff and retry
    kind=RateLimited, retry=After(n)        → respect provider limit
    kind=TokenRevoked, retry=Never          → terminal, lifecycle=Terminal
    kind=TokenExpired, retry=Never          → lifecycle=ReauthRequired
    kind=InvalidGrant, retry=Never          → policy: ReauthRequired or Terminal
    kind=ProtocolError, retry=Never         → terminal, admin intervention
```

**DX improvement:** Credential author writes `Err(CredentialError::InvalidInput("bad code".into()))`.
Framework adds `credential_id` and `stage` automatically. No ID threading in credential code.

---

## New Parameter Type: Credential Picker

For credential composition, a new parameter type allows UI to show a credential
picker filtered by scheme compatibility:

```rust
// In credential parameters:
Parameter::credential("base_credential")
    .label("Base AWS Credential")
    .scheme::<AwsAuth>()         // only show credentials producing AwsAuth
    .required()

// Serialized as JSON:
{
    "type": "credential",
    "key": "base_credential",
    "label": "Base AWS Credential",
    "scheme_kind": "aws",
    "required": true
}
```

Frontend renders a dropdown of compatible credentials. Value stored as
`CredentialId` string.

---

## Security Model

This section documents trust boundaries, threat mitigations, and operational
security requirements discovered through adversarial review.

### Trust boundaries

```
┌─────────────────────────────────────────────────────────────┐
│  TRUSTED ZONE — same process, same trust level as framework │
│                                                             │
│  • Credential implementations (resolve, refresh, revoke)    │
│  • Protocol implementations (OAuth2Flow, StaticProtocol)    │
│  • AuthScheme types (BearerToken, DatabaseAuth, etc.)       │
│                                                             │
│  → These can make arbitrary outbound HTTP calls              │
│  → These can read any secret passed to them                  │
│  → No sandbox — same as framework code                       │
│                                                             │
│  Third-party credential plugins: treat as trusted extensions │
│  only. Require plugin signing + first-party registry for     │
│  untrusted sources. Consider ALLOWS_NETWORK: bool const      │
│  for audit (Phase N).                                        │
└─────────────────────────────────────────────────────────────┘
         │
         │ Framework boundary
         ▼
┌─────────────────────────────────────────────────────────────┐
│  FRAMEWORK ZONE — enforces security invariants               │
│                                                             │
│  • PendingStateStore: 4-dimensional token binding            │
│  • CredentialStore: encryption, scope, audit layers          │
│  • RefreshCoordinator: timeout, scopeguard, circuit breaker  │
│  • Error normalization: CredentialError → ResolutionError    │
│  • Timeout on all credential methods (30s hard limit)        │
└─────────────────────────────────────────────────────────────┘
         │
         │ API / HTTP boundary
         ▼
┌─────────────────────────────────────────────────────────────┐
│  UNTRUSTED ZONE — external callers, UI, API consumers       │
│                                                             │
│  • MUST NOT see ResolutionError details (stage, scheme kind) │
│  • MUST see only: "invalid request" / "access denied" /     │
│    "credential unavailable"                                  │
│  • Timing must be normalized (jitter on failure paths)       │
│  • PendingToken must not appear in URLs                      │
└─────────────────────────────────────────────────────────────┘
```

### Callback security (OAuth2/SAML interactive flows)

**PendingToken MUST NOT appear in URLs.** OAuth2 `state` parameter =
`csrf_state` (random anti-CSRF nonce), NOT the PendingToken. The framework
correlates callback → PendingToken via server-side session (HttpOnly
SameSite=Lax cookie recommended).

**Login CSRF prevention:** The 4-dimensional PendingToken binding
(credential_kind + owner_id + session_id + token_id) prevents session
fixation. If attacker initiates flow in their session and victim completes
it in a different session, `session_id` mismatch → consume fails.

### Error normalization

`ResolutionError` (with credential_id, stage, scheme kind) is **operator-only**.
External callers / UI / API responses receive normalized generic errors:

| Internal error | External response |
|---------------|-------------------|
| `SchemeMismatch { expected: "bearer", actual: "database" }` | `"credential unavailable"` |
| `CompositionDepthExceeded { depth: 4, max: 3 }` | `"credential unavailable"` |
| `ScopeViolation { required_scope: "tenant-42" }` | `"access denied"` |
| `RefreshFailed { kind: TokenRevoked, ... }` | `"credential unavailable"` |
| `Store(NotFound { id: ... })` | `"credential unavailable"` |

**Provider error sanitization:** Credential authors MUST NOT pass raw HTTP
response bodies or URLs with query parameters into `CredentialError::Provider`.
Provider responses may contain secrets (e.g., `"Client secret 'ABC-123' is
invalid"`). Sanitize before wrapping:

```rust
// BAD: raw provider response may contain secrets
Err(CredentialError::Provider(Box::new(reqwest_error)))

// GOOD: sanitized message
Err(CredentialError::Provider(
    format!("OAuth2 token exchange failed: HTTP {}", resp.status()).into()
))
```

### Secret handling guarantees and limitations

**What the framework guarantees:**
- `SecretString` zeroizes on drop (final buffer)
- `SecretGuard` prevents accidental logging (`Display` → `<REDACTED>`)
- `SecretString::ct_eq()` for constant-time comparison (use for secret equality checks)
- Serialization buffers wrapped in `Zeroizing<Vec<u8>>` in EncryptionLayer/PendingStateStore
- Cache stores ciphertext only — no plaintext in cache heap
- Redacted Debug on all built-in AuthScheme types

**What the framework does NOT guarantee (operational responsibility):**
- **Reallocation copies:** `String`/`Vec<u8>` growth leaves old buffer without zeroize.
  Use `secrecy` crate's `SecretString` which uses `Zeroizing<String>`. Pre-allocate
  where possible. This is a fundamental limitation of heap allocators.
- **Swap/pagefile:** Secrets in memory may be swapped to disk. Deploy with encrypted
  swap or `vm.swappiness=0`.
- **Core dumps:** Disable in production (`ulimit -c 0` or `prctl(PR_SET_DUMPABLE, 0)`).
- **Panic payloads:** Don't format secret-bearing types in panic messages.
- **HTTP client instrumentation:** Third-party HTTP clients (reqwest) may log request
  headers/bodies. Disable tracing on credential provider clients.

### AuthScheme compatibility model

`AuthScheme` is **structural** compatibility — type-level shape matching.
A `BearerToken` from OAuth2 and a `BearerToken` from API key are
interchangeable at the type level.

**Semantic** compatibility (audience, scope, issuer) is NOT enforced by
AuthScheme. Resources needing semantic validation must check in `create()`:

```rust
impl Resource for GoogleSheetsApi {
    type Auth = OAuth2Token;
    
    async fn create(&self, config: &Config, auth: &OAuth2Token, ctx: &dyn Ctx)
        -> Result<Client, Error>
    {
        // Semantic validation: check scopes
        if !auth.scopes.iter().any(|s| s.contains("spreadsheets")) {
            return Err(Error::InsufficientScopes);
        }
        // ...
    }
}
```

### Credential composition scope

`ctx.resolve_credential()` passes through `ScopeLayer` on the underlying
`CredentialStore`. Cross-tenant credential composition is blocked by default.
Explicit scope configuration required to allow cross-tenant access.

The credential picker UI parameter MUST filter credentials by owner_id.
Backend MUST validate that submitted credential_id is accessible to the
current user before storing the binding.

---

## Module Layout

```
crates/credential/src/
├── lib.rs                          // Public API re-exports
│
├── scheme/                         // AuthScheme types
│   ├── mod.rs                      // AuthScheme trait
│   ├── bearer.rs                   // BearerToken
│   ├── basic.rs                    // BasicAuth
│   ├── database.rs                 // DatabaseAuth
│   ├── api_key.rs                  // ApiKeyAuth + ApiKeyPlacement
│   ├── header.rs                   // HeaderAuth
│   ├── certificate.rs              // CertificateAuth
│   ├── ssh.rs                      // SshAuth + SshAuthMethod
│   ├── oauth2.rs                   // OAuth2Token
│   ├── aws.rs                      // AwsAuth
│   ├── ldap.rs                     // LdapAuth + LdapBindMethod
│   ├── saml.rs                     // SamlAuth
│   ├── kerberos.rs                 // KerberosAuth
│   └── hmac.rs                     // HmacSecret
│
├── credential.rs                   // Credential trait (unified, const capabilities)
├── state.rs                        // CredentialState trait + identity_state! macro
├── description.rs                  // CredentialDescription, CredentialKey
├── context.rs                      // CredentialContext (pending_store + resolver)
├── handle.rs                       // CredentialHandle<S> (snapshot, not Deref)
│
├── resolve/                        // Resolution types
│   ├── mod.rs                      // ResolveResult, InteractionRequest, UserInput
│   ├── pending.rs                  // PendingState trait, PendingToken (CSPRNG), NoPendingState
│   ├── pending_store.rs            // PendingStateStore trait (put/get/consume/delete)
│   └── display.rs                  // DisplayData
│
├── refresh/                        // Refresh coordination
│   ├── mod.rs                      // RefreshCoordinator
│   ├── policy.rs                   // RefreshPolicy (early_refresh, backoff, jitter)
│   └── outcome.rs                  // RefreshOutcome (Refreshed, NotSupported, ReauthRequired)
│
├── protocol/                       // Reusable protocol building blocks
│   ├── mod.rs                      // StaticProtocol trait
│   ├── api_key.rs                  // ApiKeyProtocol
│   ├── basic_auth.rs               // BasicAuthProtocol
│   ├── database.rs                 // DatabaseProtocol
│   ├── header_auth.rs              // HeaderAuthProtocol
│   ├── ldap.rs                     // LdapProtocol
│   └── oauth2/                     // OAuth2 flow helpers
│       ├── mod.rs                  // OAuth2Flow builder
│       ├── config.rs               // OAuth2Config, AuthStyle, GrantType
│       ├── state.rs                // OAuth2State (internal stored state)
│       └── pkce.rs                 // PKCE challenge generation
│
├── store/                          // Storage abstraction
│   ├── mod.rs                      // CredentialStore trait, StoredCredential, PutMode
│   ├── memory.rs                   // InMemoryStore
│   ├── local.rs                    // LocalFileStore
│   ├── postgres.rs                 // PostgresStore
│   ├── vault.rs                    // VaultStore
│   ├── aws.rs                      // AwsSecretsStore
│   └── k8s.rs                      // K8sSecretsStore
│
├── layer/                          // Composable storage layers
│   ├── mod.rs                      // StoreLayer trait
│   ├── encryption.rs               // EncryptionLayer (decrypt/encrypt boundary)
│   ├── cache.rs                    // CacheLayer (moka — caches ciphertext only)
│   ├── scope.rs                    // ScopeLayer (multi-tenant, outermost)
│   └── audit.rs                    // AuditLayer (redacted metadata only)
│
├── resolver.rs                     // CredentialResolver (composition + cycle detection)
├── registry.rs                     // CredentialRegistry (type → handler mapping)
├── event.rs                        // CredentialRotatedEvent
├── error.rs                        // CredentialError, StoreError, RefreshErrorKind, RetryAdvice, ResolutionStage
│
├── crypto/                         // Encryption primitives
│   ├── mod.rs
│   ├── aes_gcm.rs                  // AES-256-GCM encrypt/decrypt
│   ├── secret_string.rs            // SecretString (zeroize on drop, ct_eq)
│   └── secret_guard.rs             // SecretGuard (redacted Display/Debug)
│
└── testing/                        // Test utilities (feature: test-support)
    ├── mod.rs
    ├── mock_store.rs               // InMemoryStore + error injection
    └── test_credentials.rs         // Pre-built test credential types
```

---

## Implementation Phases

### Phase 1: Core types + AuthScheme + Credential trait
- `AuthScheme` trait + all built-in schemes (15 types, redacted Debug)
- `CredentialState` trait + `identity_state!` macro (no blanket impl)
- `Credential` trait (unified: associated const capabilities, RefreshPolicy)
- `ResolveResult`, `InteractionRequest`, `UserInput`, `RefreshOutcome` (no Terminal), `TestResult`
- `PendingState` trait, `PendingToken` (CSPRNG 32B), `NoPendingState`
- `PendingStateStore` trait (put/get/consume/delete, owner-validated)
- `CredentialDescription`, `CredentialKey`, `CredentialContext` (with pending_store injection)
- `CredentialError` (structured: #[source] Send-only, RetryAdvice, RefreshErrorKind, ResolutionStage)
- `StoreError` (structured: #[source] Send-only, backend identification)
- `CredentialLifecycle` (Active/ReauthRequired/Terminal in CredentialMetadata)
- `SecretString`, AES-256-GCM crypto, `Zeroizing<Vec<u8>>` for serialization buffers
- **Depends on:** nebula-core, nebula-parameter

### Phase 2: Storage + Layers
- `CredentialStore` trait (5 methods, native async, `PutMode`, returns `StoredCredential`)
- `StoredCredential`, `CredentialMetadata` (with `CredentialLifecycle`)
- `InMemoryStore` (test-only), `InMemoryPendingStore` (dev-only)
- `LocalFileStore`
- `StoreLayer` trait
- `EncryptionLayer` (decrypt/encrypt boundary, `Zeroizing<Vec<u8>>` for buffers)
- `CacheLayer` (moka — ciphertext only, invalidation on put/delete)
- Layer ordering: ScopeLayer → AuditLayer → EncryptionLayer → CacheLayer → Backend
- **Depends on:** Phase 1

### Phase 3: Protocols + Derive macros
- `StaticProtocol` trait with `type Input: FromParameters` for typed extraction
- `FromParameters` trait + `#[derive(FromParameters)]` macro
- Built-in protocols: ApiKey, BasicAuth, Database, HeaderAuth, LDAP
- `OAuth2Flow` builder + PKCE + OAuth2State + OAuth2Pending
- `#[derive(Credential)]` macro with `defaults`, capability consts, `REFRESH_POLICY`
- `#[derive(SecretModel)]` macro: auto-generates redacted Debug + ZeroizeOnDrop for AuthScheme types
- `Parameter::credential()` type
- **Depends on:** Phase 1, nebula-macros

### Phase 4: Resolver + Refresh + Composition
- `CredentialResolver` (configurable composition depth, ResolutionError wrapping)
- Framework resolve executor (handles PendingState storage/consumption lifecycle)
- `RefreshCoordinator` (DashMap entry API + CAS via PutMode + lifecycle-aware loser handling)
- `CredentialHandle<S>` with `ArcSwap` + `snapshot()` API
- `CredentialContext::resolve_credential()` for composition
- `CredentialRotatedEvent` + EventBus integration
- `CredentialRegistry` (type dispatch: state_kind → handler, debug_assert capabilities)
- Scheme coercion via `TryFrom` at binding time
- Integration with nebula-resource `Resource::Auth`
- Jitter on refresh window (via REFRESH_POLICY const)
- **Depends on:** Phase 2, Phase 3, nebula-resource, nebula-eventbus

### Phase 5: Scope + Audit + Production storage
- `ScopeLayer` (multi-tenant isolation — outermost layer)
- `AuditLayer` (redacted metadata only — never plaintext)
- `PostgresStore`, `VaultStore`
- `AwsSecretsStore`, `K8sSecretsStore`
- `RedisPendingStore` / `PostgresPendingStore` (HA pending state for multi-node)
- Distributed refresh lease (optional, for multi-node coordination beyond CAS)
- **Depends on:** Phase 4

### Phase 6: Testing infrastructure
- `MockCredentialStore` with error injection
- `MockPendingStore` with error injection
- Pre-built test credentials
- Contract test macro for credential types
- Debug redaction assertion tests for all built-in types
- Capability const vs method consistency tests (debug_assert)
- Integration test harness
- **Depends on:** Phase 5

---

## Migration from v1

### Breaking changes

1. **`CredentialType` trait → `Credential` trait.** One trait replaces
   `CredentialType` + `StaticProtocol` + `FlowProtocol` + `InteractiveCredential`
   + `Refreshable` + `Revocable` + `CredentialResource`.

2. **`CredentialState` → split into `CredentialState` (stored) + `AuthScheme` (exposed).**
   `ApiKeyState` → `BearerToken`. `DatabaseState` → `DatabaseAuth`.
   `OAuth2State` stays but `OAuth2Token` is the new consumer-facing scheme.

3. **`CredentialManager` → `CredentialResolver` + layered store.**
   God object decomposed into single-purpose components.

4. **`StorageProvider` (12 methods) → `CredentialStore` (5 methods).**
   Rotation state and usage metrics removed from storage trait.

5. **`CredentialProvider` → `CredentialAccessor` returns `CredentialHandle<S>`.**
   No more `CredentialSnapshot` with JSON. Typed handles only.

6. **`Resource::Credential` → `Resource::Auth`.**
   Resource declares auth scheme, not credential type.

### Migration path

- Phase 1-3 can coexist with v1 (new types, no trait conflicts)
- Phase 4 replaces runtime integration (breaking for nebula-resource)
- v1 traits deprecated with `#[deprecated]` in Phase 3, removed in Phase 5
