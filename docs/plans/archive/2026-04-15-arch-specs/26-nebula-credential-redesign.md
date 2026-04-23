# 26 — `nebula-credential` redesign

> **Status:** IMPLEMENTED (2026-04-23 — all spec items completed)
> **Authority:** subordinate to `docs/PRODUCT_CANON.md`. Canon wins on conflict.
> **Parent:** [`./README.md`](./README.md)
> **Scope:** Align `nebula-credential` with specs 22 (credential v3), 23
> (cross-crate foundation), 24 (nebula-core redesign). Migrate CredentialContext
> to BaseContext, unify CredentialAccessor with core, replace SecretString with
> secrecy, three-crate split, delete retry facade.
> **Depends on:** 22 (credential v3 design), 23 (Context/Guard/Dependencies),
> 24 (nebula-core redesign), 25 (resource redesign — HasResourcesExt used by
> CredentialContext for OAuth2 refresh)
> **Consumers:** `nebula-action`, `nebula-engine`, `nebula-testing`, plugin crates

## 1. Problem

`nebula-credential` is the largest business crate (~19K LOC, 65 files) with a
mature credential lifecycle. It predates specs 22/23/24 and has these misalignments:

1. **`CredentialContext`** uses `String` owner_id, `Uuid` trace_id, `Option<ScopeLevel>`
   with String variants — should embed `BaseContext` with typed `Principal`, `Scope`,
   `TraceId`, `CancellationToken`, `Clock`
2. **`CredentialResolverRef`** — custom trait for credential composition (AWS Assume
   Role). Should be replaced by standard `HasCredentials` from core
3. **`CredentialAccessor`** — defined in this crate with `async_trait`. Spec 23
   defines `CredentialAccessor` in `nebula-core`. Two traits, one role.
4. **`CredentialGuard<S>`** — no `Guard`/`TypedGuard` impls from core
5. **`SecretString`** — imported from `nebula-core`. Decision D (spec 24): replace
   with `secrecy` crate, add `RedactedSecret` wrapper here
6. **`ParameterCollection`** — from `nebula-parameter`. Spec 21: replace with
   `nebula-schema::Schema`
7. **`retry.rs`** — 366-line facade wrapping `nebula-resilience`. Spec 22: delete,
   callers use resilience directly
8. **No `DeclaresDependencies`** — credential→resource deps undeclared
9. **Monolith crate** — store, layers, executor, resolver, rotation all in one.
   Spec 22: three-crate split

### 1.1 What stays unchanged

- **`Credential` trait** — unified lifecycle (resolve/project/refresh/test/revoke),
  3 associated types, 5 capability consts. Well-designed, RPITIT, no changes needed
  (except `CredentialContext` type in method signatures and `ParameterCollection` → `Schema`)
- **12 scheme types** — SecretToken, IdentityPassword, OAuth2Token, KeyPair,
  Certificate, SigningKey, SharedKey, ChallengeSecret, ConnectionUri,
  FederatedAssertion, OtpSeed, InstanceBinding. Only `SecretString` → `secrecy` change
- **`CredentialGuard<S: Zeroize>`** — Deref + Zeroize on Drop. Gains Guard/TypedGuard,
  internal structure unchanged
- **`crypto`** module — AES-256-GCM, key derivation, PKCE
- **`CredentialSnapshot`**, **`CredentialState`**, **`CredentialDescription`**,
  **`CredentialMetadata`**, **`CredentialKey`** (moves to `CredentialKey` from core)
- **Error types** — `CredentialError`, `CryptoError`, `ValidationError` with `Classify`
- **Pending flow** — `PendingState`, `PendingToken`, stores
- **Built-in credentials** — ApiKeyCredential, BasicAuthCredential, OAuth2Credential

## 2. Decision

Nine targeted changes, most mechanical:

1. **Unify** `CredentialAccessor` — delete local trait, use `nebula-core::CredentialAccessor`
2. **Rewrite** `CredentialContext` — embed `BaseContext` + `HasResources` + `HasCredentials` + domain fields
3. **Delete** `CredentialResolverRef` — replaced by `HasCredentials` on context
4. **Add** Guard/TypedGuard impls on `CredentialGuard<S>`
5. **Add** `HasCredentialsExt` extension trait — typed `ctx.credential::<C>()`
6. **Replace** `SecretString` with `secrecy` crate + `RedactedSecret` wrapper
7. **Replace** `ParameterCollection` with `nebula_schema::Schema`
8. **Delete** `retry.rs` — callers use `nebula-resilience` directly
9. **Three-crate split** boundaries defined (implementation in separate PRs)

## 3. Changes

### 3.1 CredentialAccessor unification

**Delete:** local `CredentialAccessor` trait from `accessor.rs`.

**Keep in `accessor.rs`:** `NoopCredentialAccessor`, `ScopedCredentialAccessor` —
but as implementations of `nebula_core::CredentialAccessor`, not a local trait.

```rust
// accessor.rs — impls of core trait
use nebula_core::accessor::CredentialAccessor;

pub struct NoopCredentialAccessor;
impl CredentialAccessor for NoopCredentialAccessor { /* ... */ }

pub struct ScopedCredentialAccessor { /* allowed_types, inner */ }
impl CredentialAccessor for ScopedCredentialAccessor { /* ... */ }
```

`async_trait` removed from accessor — core trait uses `Pin<Box<dyn Future>>`
for dyn-safety.

### 3.2 CredentialContext redesign

**Rewrite** `context.rs`:

```rust
use std::sync::Arc;
use nebula_core::{
    context::{Context, BaseContext},
    context::capability::{HasResources, HasCredentials},
    accessor::{ResourceAccessor, CredentialAccessor},
};

/// Domain context for credential lifecycle methods (resolve, refresh, test, revoke).
///
/// Embeds BaseContext for identity/scope/cancellation/clock.
/// HasResources for OAuth2 refresh that needs HttpResource.
/// HasCredentials for credential composition (AWS Assume Role).
/// Domain-specific fields for interactive flows (callback_url, session_id).
pub struct CredentialContext {
    base: BaseContext,
    resources: Arc<dyn ResourceAccessor>,
    credentials: Arc<dyn CredentialAccessor>,
    // Domain-specific:
    callback_url: Option<String>,
    app_url: Option<String>,
    session_id: Option<String>,
}

impl Context for CredentialContext { /* delegate to base */ }
impl HasResources for CredentialContext {
    fn resources(&self) -> &dyn ResourceAccessor { &*self.resources }
}
impl HasCredentials for CredentialContext {
    fn credentials(&self) -> &dyn CredentialAccessor { &*self.credentials }
}

impl CredentialContext {
    pub fn callback_url(&self) -> Option<&str> { self.callback_url.as_deref() }
    pub fn app_url(&self) -> Option<&str> { self.app_url.as_deref() }
    pub fn session_id(&self) -> Option<&str> { self.session_id.as_deref() }
}
```

**Deleted fields:**
- `owner_id: String` → replaced by `BaseContext` → `Principal` (typed)
- `trace_id: Uuid` → replaced by `BaseContext` → `TraceId` (typed)
- `caller_scope: Option<ScopeLevel>` → replaced by `BaseContext` → `Scope` (typed IDs)
- `timestamp: DateTime<Utc>` → replaced by `BaseContext` → `Clock::now()`
- `resolver: Option<Arc<dyn CredentialResolverRef>>` → replaced by `HasCredentials`

**Credential composition (AWS Assume Role) now uses standard API:**

```rust
// Before:
fn refresh(state: &mut Self::State, ctx: &CredentialContext) -> ... {
    let base = ctx.resolve_credential::<BaseAwsCredential>("base-cred").await?;
}

// After:
fn refresh(state: &mut Self::State, ctx: &CredentialContext) -> ... {
    let base = ctx.credential::<BaseAwsCredential>().await?;  // HasCredentialsExt
}
```

### 3.3 Delete CredentialResolverRef

**Delete entirely** from `context.rs`. Its role replaced by
`HasCredentials` + `HasCredentialsExt::credential::<C>()`.

**Note on spec 23 "credential→credential FORBIDDEN":** this refers to
`DeclaresDependencies` static declarations (Tarjan SCC cycle detection).
Runtime composition via `ctx.credential::<C>()` is allowed — engine controls
ordering and prevents cycles at registration/activation time, not compile time.

### 3.4 CredentialGuard + Guard/TypedGuard

```rust
impl<S: Zeroize + Send + Sync + 'static> nebula_core::Guard for CredentialGuard<S> {
    fn guard_kind(&self) -> &'static str { "credential" }
    fn acquired_at(&self) -> Instant { self.acquired_at }
}

impl<S: Zeroize + Send + Sync + 'static> nebula_core::TypedGuard for CredentialGuard<S> {
    type Inner = S;
    fn as_inner(&self) -> &S { self }  // delegates to Deref
}
```

Existing `Deref<Target = S>`, `Drop` (zeroize), `Debug` (redacted),
conditional `Clone` — unchanged.

### 3.5 HasCredentialsExt extension trait

**New file:** `ext.rs`

```rust
use nebula_core::context::capability::HasCredentials;
use crate::{Credential, CredentialGuard, error::CredentialError};

/// Typed credential access for any context implementing HasCredentials.
///
/// Primary API: `ctx.credential::<SlackBotToken>().await?`
pub trait HasCredentialsExt: HasCredentials {
    fn credential<C: Credential>(&self)
        -> impl Future<Output = Result<CredentialGuard<C::Scheme>, CredentialError>> + Send
    where Self: Sized;

    fn try_credential<C: Credential>(&self)
        -> impl Future<Output = Result<Option<CredentialGuard<C::Scheme>>, CredentialError>> + Send
    where Self: Sized;
}

impl<Ctx: HasCredentials + ?Sized> HasCredentialsExt for Ctx {
    async fn credential<C: Credential>(&self) -> Result<CredentialGuard<C::Scheme>, CredentialError> {
        let snapshot = self.credentials().resolve_any(&C::KEY.into()).await?;
        // Project scheme from stored state, wrap in guard
        let scheme = C::project(&snapshot.state_as::<C::State>()?);
        Ok(CredentialGuard::new(scheme))
    }

    async fn try_credential<C: Credential>(&self) -> Result<Option<CredentialGuard<C::Scheme>>, CredentialError> {
        match self.credential::<C>().await {
            Ok(guard) => Ok(Some(guard)),
            Err(CredentialError::NotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
```

### 3.6 SecretString → secrecy

All 12 scheme types change `nebula_core::SecretString` → `secrecy::SecretString`.

**New in this crate:** `RedactedSecret<S>` wrapper for serde:

```rust
// secret.rs (new)
use secrecy::{Secret, Zeroize};
use serde::{Serialize, Serializer};

/// Wrapper that serializes as "[REDACTED]" by default.
/// For real persistence, use serde_secret helper module.
pub struct RedactedSecret<S: Zeroize>(pub Secret<S>);

impl<S: Zeroize> Serialize for RedactedSecret<S> {
    fn serialize<Ser: Serializer>(&self, s: Ser) -> Result<Ser::Ok, Ser::Error> {
        s.serialize_str("[REDACTED]")
    }
}
// + Deref to Secret<S>, Debug (redacted), Clone where S: Clone
```

**Moved from nebula-core:** `serde_secret.rs` and `option_serde_secret.rs`
modules relocate here (only consumer).

### 3.7 ParameterCollection → Schema

```rust
// Before:
fn parameters() -> ParameterCollection where Self: Sized;

// After:
fn parameters() -> nebula_schema::Schema where Self: Sized;
```

`nebula-parameter` dependency replaced with `nebula-schema` in `Cargo.toml`.

### 3.8 Delete retry.rs

Remove `retry.rs` (366 lines). Callers switch to `nebula_resilience::retry_with`:

```rust
// Before (retry facade):
crate::retry::with_retry(|| async { resolver.resolve(...) }, config).await

// After (direct resilience):
nebula_resilience::retry_with(config, || async { resolver.resolve(...) }).await
```

### 3.9 Three-crate split boundaries

Per spec 22. Not implemented in this PR set — separate effort.

| Stays in `nebula-credential` | → `nebula-storage::credential` | → `nebula-engine::credential` |
|---|---|---|
| Credential trait | store.rs, store_memory.rs | executor.rs |
| 12 scheme types | layer/* (encryption, cache, scope, audit) | resolver.rs |
| guard, snapshot, state | rotation/* | refresh.rs coordinator |
| context, accessor impls | | |
| crypto, description, metadata | | |
| pending, pending_store | | |
| registry, key, any, error | | |
| ext.rs (new), secret.rs (new) | | |

**Boundary rule:** `nebula-credential` contains traits, types, and crypto —
no storage I/O, no HTTP calls (except in credential impls like OAuth2).
Storage and orchestration move to their respective crates.

## 4. Cargo.toml changes

```toml
[dependencies]
# NEW
secrecy = { version = "0.10", features = ["serde"] }  # replaces nebula-core SecretString
nebula-schema = { path = "../schema" }                  # replaces nebula-parameter

# REMOVED
# nebula-parameter = { path = "../parameter" }  — replaced by nebula-schema

# async_trait removed from accessor (core trait uses Pin<Box<dyn Future>>)
# async-trait stays for now — other internal uses may remain; remove when fully migrated

# Everything else unchanged
```

## 5. DeclaresDependencies for Credential

Credentials can declare resource dependencies (OAuth2 needs HttpResource for
token refresh), but **NOT** credential dependencies.

```rust
#[derive(Credential)]
#[uses_resource(HttpResource, purpose = "OAuth2 token refresh")]
struct GoogleOAuth2Credential;

// Generated:
impl DeclaresDependencies for GoogleOAuth2Credential {
    fn dependencies() -> Dependencies {
        Dependencies::new()
            .resource(ResourceRequirement::of::<HttpResource>()
                .purpose("OAuth2 token refresh"))
        // NO .credential() — compile error from derive macro
    }
}
```

`#[uses_credential(...)]` on a `#[derive(Credential)]` — **compile error**
with clear message: "credential-to-credential static dependencies are
forbidden (spec 23). Use ctx.credential::<C>() for runtime composition."

## 6. Edge cases

### 6.1 Runtime credential composition vs static deps

`DeclaresDependencies` prevents static cycles (Tarjan SCC at registration).
Runtime composition via `ctx.credential::<C>()` is engine-controlled — engine
ensures ordering and detects runtime cycles via call-stack depth limit.

### 6.2 OAuth2 refresh needs HttpResource

`CredentialContext: HasResources` enables:
```rust
fn refresh(state: &mut Self::State, ctx: &CredentialContext) -> ... {
    let http = ctx.resource::<HttpResource>().await?;
    let response = http.post(token_url).form(&refresh_params).send().await?;
    // ...
}
```

HttpResource declared via `#[uses_resource(HttpResource)]` on derive.

### 6.3 SecretString migration scope

12 scheme types + snapshot + state + crypto + store layers — ~40 files touch
`SecretString`. Mechanical find-and-replace with compile verification.

## 7. Testing criteria

- `CredentialAccessor` impls (Noop, Scoped) work with core trait
- `CredentialContext`: Context/HasResources/HasCredentials all delegate correctly
- `HasCredentialsExt`: `ctx.credential::<C>()` resolves and returns CredentialGuard
- `CredentialGuard`: Guard/TypedGuard methods correct, existing tests pass
- `RedactedSecret`: Serialize outputs "[REDACTED]", Deref works, Debug redacted
- All 12 scheme types compile with `secrecy::SecretString`
- `retry.rs` deleted, no compile errors
- Credential composition via `ctx.credential::<C>()` works (replaces CredentialResolverRef)

## 8. Migration path

### PR 1: Guard + accessor unification

1. CredentialGuard gains Guard/TypedGuard impls
2. Delete local CredentialAccessor trait
3. Noop/Scoped implement core CredentialAccessor
4. Add HasCredentialsExt extension trait (ext.rs)

### PR 2: Context redesign

1. Rewrite CredentialContext (BaseContext + HasResources + HasCredentials + domain fields)
2. Delete CredentialResolverRef
3. Update Credential trait method signatures (ctx type)
4. Update all credential impls
5. Fix downstream compile errors

### PR 3: SecretString + retry + ParameterCollection

1. Add secrecy dependency, add RedactedSecret wrapper
2. Move serde_secret/option_serde_secret from core
3. Replace SecretString in all 12 scheme types + ~40 files
4. Delete retry.rs, update callers to nebula-resilience
5. Replace ParameterCollection → Schema (depends on spec 21)

### PR 4: DeclaresDependencies

1. Update #[derive(Credential)] macro
2. Add #[uses_resource] support
3. Forbid #[uses_credential] with compile error message

### PR 5: Three-crate split (separate, large)

1. Move store/layer to nebula-storage::credential
2. Move executor/resolver/refresh to nebula-engine::credential
3. Update all imports across workspace

## 9. Open questions

### 9.1 async_trait full removal timeline

Current crate uses `async_trait` beyond accessor (some internal traits).
Full removal deferred to RPITIT migration pass across workspace.

### 9.2 CredentialKey type

Current: `crate::key::CredentialKey` (CompactString newtype).
Spec 24: `nebula_core::CredentialKey` via `domain_key::Key<CredentialDomain>`.
Migration: alias local to core, eventually delete local.

### 9.3 Three-crate split timing

Split is large (~30 files move). Can be done independently of spec 23/24
alignment. Recommend: alignment PRs first (1-4), split PR last (5).
