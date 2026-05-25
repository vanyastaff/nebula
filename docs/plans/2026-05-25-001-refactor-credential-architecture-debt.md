---
title: "refactor: credential architecture strategic debt"
type: refactor
status: proposed
date: 2026-05-25
origin: deep architectural review of nebula-credential crate
---

# refactor: credential architecture strategic debt

## Executive Summary

The nebula-credential crate is well-architected for its current scope (OAuth2/API-key/basic-auth ecosystem with enterprise integrations). The trait hierarchy, CP6 capability sub-trait split, and registry design are sound, closing known silent-downgrade security vectors (N1, N3, N5).

Strategic design debt emerges when projecting toward a truly universal credential system supporting arbitrary auth mechanisms (mTLS, Kerberos, SAML federation, blockchain wallets, hardware tokens, biometric delegates). These are design constraints — not bugs — that will require deliberate evolution as scope expands.

---

## Finding 1: Binary Capability Model Limits Universality

| | |
|---|---|
| **Category** | Trait Hierarchy |
| **Severity** | High |

**Current design:** Five sub-traits (`Interactive`, `Refreshable`, `Revocable`, `Testable`, `Dynamic`) enforce compile-time binary membership. A credential either implements a capability or doesn't. Detection at registration via `plugin_capability_report::IsX` const-bool traits.

**Files:** `crates/credential/src/contract/credential.rs`, `refreshable.rs`, `interactive.rs`, `revocable.rs`, `testable.rs`, `dynamic.rs`, `capability_report.rs`

**Why problematic at scale:**

- No conditional capabilities — "refresh only if 2FA is enabled" or "revoke only with certain firmware"
- No negotiated capabilities — SAML dynamically selecting SSO vs device-code at runtime
- No instance-specific capabilities — same credential type, different config = different capabilities
- Plugin authors forced into binary choice: `impl Refreshable` globally or not at all

**Proposed alternative — runtime capability negotiation:**

```rust
pub trait Credential: Send + Sync + 'static {
    // ... existing associated types ...

    /// Runtime capability negotiation — called per-instance, per-context.
    /// Returns Some(handler) if this credential supports refresh in this context.
    fn refresh_capability(
        &self,
        ctx: &CredentialContext,
    ) -> Option<Box<dyn RefreshHandler + '_>> {
        None  // default: no refresh
    }
}
```

**Risk:** High — changing the fundamental trait architecture. Must preserve compile-time dispatch fast path for common cases (OAuth2, API keys).

**Cross-ref:** ADR-0054 (typed capability system), ADR-0081 (M6 resource-credential integration)

---

## Finding 2: ResolveResult Enum Missing Async Protocol Outcomes

| | |
|---|---|
| **Category** | Protocol Design |
| **Severity** | High |

**Current design:** `ResolveResult<S, P>` has three variants — `Complete(S)`, `Pending { state, interaction }`, `Retry { after }`.

**File:** `crates/credential/src/contract/resolve.rs`

**Why problematic at scale:**

- No server-initiated continuation (webhook/push instead of client polling) — forces synthetic polling for protocols that prefer callbacks
- No conditional continuation ("MFA required IF corporate policy changed")
- No multi-stage flows (SAML multi-factor enrollment with sequential stages within single resolve)
- No degraded success ("OAuth2 granted fewer scopes than requested — credential works but read-only")

**Proposed alternative — extended enum:**

```rust
pub enum ResolveResult<S, P: PendingState = NoPendingState> {
    Complete(S),
    Pending { state: P, interaction: InteractionRequest },
    Retry { after: Duration },

    /// Server will callback when ready (webhook/push model)
    ServerContinuation {
        state: P,
        callback_hint: String,
        timeout: Duration,
    },

    /// Credential resolved but with reduced capability surface
    DegradedComplete {
        state: S,
        granted: Capabilities,
        warnings: Vec<String>,
    },
}
```

**Risk:** Medium — additive enum change. Existing match arms need `_ =>` fallback or explicit handling. Framework dispatch code in nebula-engine must be updated.

**Cross-ref:** ADR-0081 (credential resolution protocol)

---

## Finding 3: Store Atomicity Not Expressed in Trait Contract

| | |
|---|---|
| **Category** | State Management |
| **Severity** | Medium-High |

**Current design:** Two separate traits — `CredentialStore` (persistent, encrypted-at-rest state) and `PendingStateStore` (ephemeral, TTL-bounded, single-use interactive flow state). No transactional boundary between them.

**Files:** `crates/credential/src/store.rs`, `crates/credential/src/pending_store.rs`

**Why problematic at scale:**

- Interactive flow has critical invariant: consume pending token + store final state must be atomic
- If `PendingStateStore::consume()` succeeds but `CredentialStore::put()` fails → credential in limbo (pending token gone, final state never persisted)
- Replica crash between step 5 (continue_resolve) and step 6 (store final) → orphaned interaction
- Contract doesn't express this ordering — pushed to implementation discipline
- Plugin authors implementing custom stores may not enforce single-use + atomicity correctly

**Proposed alternative — explicit transaction trait:**

```rust
pub trait CredentialStoreTransaction: Send + Sync {
    /// Consume the pending state within the transaction.
    fn consume_pending(
        &mut self,
        token: &PendingToken,
    ) -> impl Future<Output = Result<Vec<u8>, StoreError>> + Send;

    /// Store the final credential within the transaction.
    fn store_final(
        &mut self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;

    /// Commit both changes atomically. Rollback on drop if not committed.
    fn commit(self) -> impl Future<Output = Result<(), StoreError>> + Send;
}
```

**Risk:** Medium — new trait alongside existing traits. Backward-compatible if transaction trait is optional with a default non-atomic fallback.

**Cross-ref:** ADR-0072 (storage spec-16 port-adapter-tenancy)

---

## Finding 4: No Bidirectional Credential Support

| | |
|---|---|
| **Category** | Protocol Design |
| **Severity** | Medium |

**Current design:** Credentials are unidirectional — they carry auth material for outgoing requests via `AuthScheme`. No support for mutual authentication where the external system also authenticates itself to the caller.

**Files:** `crates/credential/src/scheme/` (all scheme types), `crates/credential/src/contract/credential.rs` (project method)

**Why problematic at scale:**

- mTLS requires both client and server certificates — current `Certificate` scheme models only client-side
- Webhook signature verification (external → Nebula) must live outside credential system
- OAuth2 Mutual-TLS Client Authentication (RFC 8705) binds tokens to certificate fingerprints — credential model doesn't support reverse lookup
- SAML signed assertions require verifying the IdP's signing key — not expressible in current scheme model

**Proposed alternative — IncomingVerifier on AuthScheme:**

```rust
pub trait AuthScheme: Send + Sync + 'static {
    fn pattern() -> AuthPattern;

    /// Optional: provide verification material for incoming requests/assertions.
    fn as_incoming_verifier(&self) -> Option<&dyn IncomingVerifier> {
        None
    }
}

pub trait IncomingVerifier: Send + Sync {
    /// Verify an incoming request, webhook signature, or assertion.
    fn verify(&self, data: &[u8], signature: &[u8]) -> Result<(), VerificationError>;
}
```

**Risk:** Low — additive trait method with default impl. Existing schemes unaffected.

---

## Finding 5: Registry Flat String Keys Without Namespace Isolation

| | |
|---|---|
| **Category** | Architecture |
| **Severity** | Medium |

**Current design:** `CredentialRegistry` keyed by `Credential::KEY` (`&'static str`). Duplicate keys cause fatal `RegisterError::DuplicateKey`.

**File:** `crates/credential/src/contract/registry.rs`

**Why problematic at scale:**

- Supply-chain collision: third-party plugin can accidentally or intentionally use same KEY
- No credential versioning path — migrating `oauth2_v1` to `oauth2_v2` requires either same KEY (shadow) or different KEY (breaks resource bindings)
- No tenant isolation for multi-tenant plugin marketplace
- No alias/forwarding for renamed credentials

**Proposed alternative — namespace-aware registry:**

```rust
pub struct CredentialRegistry {
    flat_entries: AHashMap<Arc<str>, RegistryEntry>,
    namespaced_entries: AHashMap<(Arc<str>, Arc<str>), RegistryEntry>,
}

impl CredentialRegistry {
    pub fn register_namespaced<C: Credential>(
        &mut self,
        namespace: &str,
        crate_name: &'static str,
    ) -> Result<(), RegisterError> { /* ... */ }

    /// Lookup: tries namespaced first, then flat (backward compat)
    pub fn resolve(&self, namespace: Option<&str>, key: &str) -> Option<&RegistryEntry> {
        /* ... */
    }
}
```

**Risk:** Medium — backward-compatible addition. Flat keys continue working. Namespace support is opt-in.

---

## Finding 6: Macro Capability-Report Fragility

| | |
|---|---|
| **Category** | Macros |
| **Severity** | Medium |

**Current design:** `#[derive(Credential)]` generates five `plugin_capability_report::IsX` impls with `const VALUE: bool`. Hand-rolled credentials can set `IsRefreshable::VALUE = false` while implementing `impl Refreshable` — no compile-time diagnostic catches this mismatch.

**Files:** `crates/credential/macros/src/credential.rs`, `crates/credential/src/contract/capability_report.rs`

**Why problematic at scale:**

- Mismatch window for hand-rolled credentials: registry says "no refresh" but trait impl exists
- No lint or compiler warning for const-bool vs actual impl divergence
- Future Rust editions (specialization, const generics in trait bounds) may require macro rewrites
- `compute_capabilities::<C>()` can diverge from actual trait membership

**Proposed alternative — registration-time validation:**

```rust
/// Sealed helper trait: exists for every Refreshable implementor
trait RefreshableWitness {}
impl<T: Refreshable> RefreshableWitness for T {}

impl CredentialRegistry {
    pub fn register<C: Credential>(&mut self, ...) -> Result<(), RegisterError> {
        let mut caps = Capabilities::empty();
        // Type-system witness check: if C: Refreshable, this compiles
        if <C as HasCapability<Refreshable>>::HAS {
            caps |= Capabilities::REFRESHABLE;
        }
        // ...
    }
}
```

**Risk:** Low — internal registry change. No public API impact.

---

## Finding 7: StaticProtocol Vestigial Indirection

| | |
|---|---|
| **Category** | Macros |
| **Severity** | Low |

**Current design:** `StaticProtocol` trait provides a `parameters()` schema and `build()` method. The `#[derive(Credential)]` macro generates `Credential::resolve()` delegating to `StaticProtocol::build()`.

**File:** `crates/credential/src/contract/static_protocol.rs`

**Why problematic at scale:**

- Unnecessary layer of indirection in Rust 1.95+ with native async traits and RPITIT
- Confuses the trait hierarchy — plugin authors see `Credential` + `StaticProtocol` + optional extensions
- Cannot express "static + refresh" (rotated API keys) — forces manual impl fallback
- Maintenance burden for the macro: extra trait layer adds complexity on every macro update

**Proposed alternative — deprecate in favor of inline properties mode:**

```rust
// Current (indirect):
#[derive(Credential)]
#[credential(protocol = ApiKeyProtocol, ...)]
pub struct ApiKeyCredential;

// Proposed (direct):
#[derive(Credential)]
#[credential(key = "api_key", properties = ApiKeyProperties, scheme = SecretToken, ...)]
pub struct ApiKeyCredential;

impl ApiKeyCredential {
    // Macro generates Credential::resolve() calling this directly
    pub async fn resolve(values: &FieldValues, ctx: &CredentialContext) -> Result<...> {
        // Direct implementation, no StaticProtocol layer
    }
}
```

**Risk:** Low — `properties` mode already exists. Deprecation is additive (mark `StaticProtocol` `#[deprecated]`, provide migration guide).

---

## Finding 8: CredentialHandle Assumes Scheme Type Invariance

| | |
|---|---|
| **Category** | Generics |
| **Severity** | Low |

**Current design:** `CredentialHandle<S: AuthScheme>` wraps `ArcSwap<S>`. The scheme type `S` is fixed for the credential's lifetime. Refresh swaps the value but not the type.

**File:** `crates/credential/src/handle.rs`

**Why problematic at scale:**

- Protocol renegotiation on refresh (SAML metadata changes, firmware update switching TOTP → WebAuthn) cannot change scheme type
- Consumers holding old snapshots don't know scheme was renegotiated — staleness is invisible
- No pre/post-refresh hooks on the handle for cleanup or preparation

**Proposed alternative — scheme versioning:**

```rust
pub struct CredentialHandle<S: AuthScheme> {
    scheme: ArcSwap<S>,
    version: AtomicU64,
}

impl<S: AuthScheme> CredentialHandle<S> {
    pub fn snapshot_versioned(&self) -> (Arc<S>, u64) {
        let s = self.scheme.load_full();
        let v = self.version.load(Ordering::Acquire);
        (s, v)
    }

    pub fn replace(&self, new_scheme: S) {
        self.scheme.store(Arc::new(new_scheme));
        self.version.fetch_add(1, Ordering::Release);
    }
}
```

**Risk:** Low — backward-compatible addition. Existing code ignores the version. New code can detect stale snapshots.

---

## Phased Resolution Timeline

| Phase | Trigger | Findings | Effort |
|-------|---------|----------|--------|
| Phase 1 (now) | — | None — current design is sound for OAuth2/API-key scope | No changes |
| Phase 2 (federation) | SAML/Kerberos/OIDC federation work begins | F2 (ResolveResult), F3 (store atomicity) | Medium — additive enum + new trait |
| Phase 3 (1-2 years) | Maturity L4+ / advanced auth mechanisms | F7 (StaticProtocol deprecation), F1 (capability negotiation), F6 (macro validation) | High — trait architecture evolution |
| Phase 4 (multi-tenant plugins) | Plugin marketplace / open ecosystem | F5 (registry namespacing), F4 (bidirectional auth), F8 (handle versioning) | Medium — backward-compatible additions |

---

## Risk Assessment

- **Low risk** (backward-compatible additions): F4 (IncomingVerifier), F7 (StaticProtocol deprecation), F8 (handle versioning)
- **Medium risk** (new traits alongside existing): F2 (ResolveResult extension), F3 (transaction trait), F5 (registry namespace), F6 (registration validation)
- **High risk** (trait architecture changes): F1 (capability negotiation model)

---

None of these findings block immediate evolution. All should be reviewed at 18-month architectural roadmap intervals as credential scope expands beyond the current OAuth2/API-key/basic-auth ecosystem.
