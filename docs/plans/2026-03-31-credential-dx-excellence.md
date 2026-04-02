# nebula-credential DX Excellence Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make nebula-credential a world-class credential system with typed access for actions, clean public API, and excellent DX — 10x better than n8n's stringly-typed approach.

**Architecture:** The critical gap is the bridge between `CredentialResolver` (type-safe, produces `CredentialHandle<S>`) and `CredentialAccessor` in nebula-action (object-safe, returns `CredentialSnapshot` with raw JSON). We fix this by making `CredentialSnapshot` carry `Box<dyn Any>` — the **projected AuthScheme** — instead of `serde_json::Value`. This enables `credential_typed::<BearerToken>("key")` in actions with zero deserialization at the call site.

**Tech Stack:** Rust 1.93, `std::any::Any`, `arc-swap`, `serde_json`, `nebula-core::AuthScheme`

**Breaking changes:** Allowed. This plan redesigns `CredentialSnapshot` and feature-gates the rotation module.

---

## Current State Summary

| Area | Status |
|------|--------|
| Core credential trait (v2) | ✅ Complete — 7 methods, 6 capability consts |
| AuthScheme (13 types in nebula-core) | ✅ Complete |
| CredentialResolver + RefreshCoordinator | ✅ Production-grade (circuit breaker, scopeguard, 30s timeout, CAS) |
| PendingStateStore + executors | ✅ Complete |
| Storage layers (4: encrypt, cache, audit, scope) | ✅ Complete |
| derive(Credential) macro | ✅ In nebula-sdk-macros |
| **CredentialSnapshot typed bridge** | ❌ Raw `serde_json::Value`, no `downcast()` |
| **Rotation module integration** | ❌ 6.2K LOC disconnected from v2 Credential trait |
| **Doctests** | ❌ 2 failures (stale `StorageError` reference) |
| **Test coverage** | ⚠️ 463 LOC tests / 17.8K LOC code |
| **CredentialContext.resolver** | ❌ Missing (needed for credential composition) |

---

## Phase 1: Typed CredentialSnapshot (THE critical path)

### Why this matters

The action plan (`crates/action/plans/05-context-capabilities.md`) defines:

```rust
// What action authors will write:
let token: BearerToken = ctx.credential_typed::<BearerToken>("api_key").await?;

// What the DX derive generates:
#[derive(ActionDeps)]
struct Deps {
    #[dep(credential = "api_key")]
    api_key: BearerToken,
}
```

Currently `CredentialSnapshot` is `{kind: String, state: serde_json::Value}` — no path from here to `BearerToken` without manual deserialization.

### Task 1.1: Redesign CredentialSnapshot

**Files:**
- Modify: `crates/credential/src/snapshot.rs`
- Modify: `crates/credential/src/lib.rs` (re-exports)

**Design:**

```rust
use std::any::Any;
use crate::metadata::CredentialMetadata;

/// A point-in-time credential snapshot carrying the projected [`AuthScheme`].
///
/// The `projected` field holds the type-erased auth material (e.g., `BearerToken`,
/// `DatabaseAuth`). Use [`project::<S>()`](Self::project) to downcast.
///
/// # Examples
///
/// ```ignore
/// let snapshot = accessor.get("my-cred").await?;
/// let token: &BearerToken = snapshot.project::<BearerToken>()?;
/// ```
#[derive(Debug)]
pub struct CredentialSnapshot {
    /// The credential type key (e.g. `"api_key"`, `"oauth2"`).
    kind: String,
    /// The scheme kind (e.g. `"bearer"`, `"database"`).
    scheme_kind: String,
    /// Associated credential metadata.
    metadata: CredentialMetadata,
    /// Type-erased projected AuthScheme.
    projected: Box<dyn Any + Send + Sync>,
}
```

Key methods:

```rust
impl CredentialSnapshot {
    /// Creates a new snapshot from a resolved credential.
    pub fn new<S: AuthScheme>(
        kind: impl Into<String>,
        metadata: CredentialMetadata,
        scheme: S,
    ) -> Self { ... }

    /// Downcasts the projected auth material to a concrete type.
    ///
    /// # Errors
    ///
    /// Returns error if the stored scheme doesn't match `S`.
    pub fn project<S: AuthScheme>(&self) -> Result<&S, SnapshotError> { ... }

    /// Consumes the snapshot and extracts the projected auth material.
    pub fn into_project<S: AuthScheme>(self) -> Result<S, SnapshotError> { ... }

    /// Returns the credential type key.
    pub fn kind(&self) -> &str { &self.kind }

    /// Returns the scheme kind.
    pub fn scheme_kind(&self) -> &str { &self.scheme_kind }

    /// Returns the credential metadata.
    pub fn metadata(&self) -> &CredentialMetadata { &self.metadata }
}
```

**SnapshotError:**

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SnapshotError {
    #[error("scheme mismatch: expected {expected}, got {actual}")]
    SchemeMismatch {
        expected: &'static str,
        actual: String,
    },
}
```

**Breaking change:** Fields are no longer `pub`. Use getters instead. `state: serde_json::Value` removed — replaced by `projected: Box<dyn Any>`.

**Tests to write:**
- `project_returns_correct_type` — create snapshot with `BearerToken`, project to `BearerToken` succeeds
- `project_wrong_type_returns_error` — create with `BearerToken`, project to `DatabaseAuth` fails with `SchemeMismatch`
- `into_project_consumes` — `into_project::<BearerToken>()` returns owned value
- `kind_and_metadata_accessors` — getters work

**Step 1:** Write tests in `snapshot.rs` `#[cfg(test)]` module
**Step 2:** Implement `CredentialSnapshot` with `Box<dyn Any>`
**Step 3:** Run `rtk cargo check -p nebula-credential` — expect compile errors in dependents
**Step 4:** Fix dependents (action crate's `CredentialAccessor` tests, sdk re-exports)
**Step 5:** Run `rtk cargo nextest run -p nebula-credential`
**Step 6:** Commit: `feat(credential)!: typed CredentialSnapshot with AuthScheme projection`

### Task 1.2: Update CredentialAccessor in nebula-action

**Files:**
- Read: `crates/action/src/capability.rs`
- Modify: `crates/action/src/capability.rs` (CredentialAccessor trait)
- Modify: `crates/action/src/context.rs` (ActionContext.credential_typed)

The `CredentialAccessor` trait currently returns `CredentialSnapshot`. After Task 1.1, the snapshot carries `Box<dyn Any>` instead of `serde_json::Value`, so `credential_typed::<S>()` can use `snapshot.project::<S>()`.

Verify that `ActionContext::credential_typed` (if it exists) or `credential` method works with the new snapshot. The action plan's `credential_typed::<S>` should be implementable as:

```rust
pub async fn credential_typed<S: AuthScheme>(&self, key: &str) -> Result<S, ActionError> {
    self.guard.check()?;
    let snapshot = self.credentials.get(key).await?;
    snapshot.into_project::<S>()
        .map_err(|e| ActionError::fatal(format!("credential '{key}': {e}")))
}
```

**Step 1:** Update `CredentialAccessor` if needed
**Step 2:** Add/update `credential_typed` on `ActionContext`
**Step 3:** Run `rtk cargo check --workspace`
**Step 4:** Run `rtk cargo nextest run -p nebula-action`
**Step 5:** Commit: `feat(action)!: typed credential access via credential_typed<S>()`

### Task 1.3: Wire CredentialResolver output to CredentialSnapshot

**Files:**
- Review: `crates/credential/src/resolver.rs`

The resolver already produces `CredentialHandle<C::Scheme>`. The runtime (which implements `CredentialAccessor`) needs to:

1. Call `resolver.resolve::<C>(id)` or `resolve_with_refresh::<C>(id, ctx)`
2. Get `CredentialHandle<S>` where `S: AuthScheme`
3. Call `handle.snapshot()` → `Arc<S>`
4. Construct `CredentialSnapshot::new::<S>(C::KEY, metadata, (*arc).clone())`

This wiring happens in the **runtime/engine** (outside credential crate), but credential needs to export `CredentialSnapshot::new` which is done in Task 1.1. No additional credential crate changes needed.

Add a doc example in `snapshot.rs` showing the construction pattern.

**Step 1:** Add doc example to `CredentialSnapshot::new`
**Step 2:** Run `rtk cargo test --doc -p nebula-credential`
**Step 3:** Commit: `docs(credential): add CredentialSnapshot construction example`

---

## Phase 2: Fix broken doctests

### Task 2.1: Fix StorageError reference in lib.rs doctests

**Files:**
- Modify: `crates/credential/src/lib.rs` (lines ~56, ~69)

Two doctests reference `nebula_credential::StorageError` which doesn't exist (it's `StoreError`). Fix the imports.

**Step 1:** Read the failing doctest code
**Step 2:** Replace `StorageError` with `StoreError` (or remove if the example is outdated)
**Step 3:** Run `rtk cargo test --doc -p nebula-credential`
**Step 4:** Commit: `fix(credential): fix broken doctests referencing StorageError`

---

## Phase 3: Rotation module — feature gate

### Why

The rotation module is 6.2K LOC defining separate trait hierarchies (`TestableCredential`, `RotatableCredential`) that don't reference the v2 `Credential` trait. It's architecturally sound but disconnected. Rather than delete or rush a redesign, feature-gate it for now.

### Task 3.1: Feature-gate rotation module

**Files:**
- Modify: `crates/credential/Cargo.toml` — add `rotation` feature
- Modify: `crates/credential/src/lib.rs` — `#[cfg(feature = "rotation")]` on rotation module and re-exports

**Step 1:** Add to Cargo.toml:
```toml
[features]
default = []
rotation = []
```

**Step 2:** Gate the rotation module:
```rust
#[cfg(feature = "rotation")]
pub mod rotation;
```

**Step 3:** Gate rotation re-exports in lib.rs
**Step 4:** Run `rtk cargo check -p nebula-credential` (without feature — should compile)
**Step 5:** Run `rtk cargo check -p nebula-credential --features rotation` (with feature — should compile)
**Step 6:** Run `rtk cargo nextest run -p nebula-credential`
**Step 7:** Commit: `refactor(credential)!: feature-gate rotation module behind "rotation" flag`

### Task 3.2: Document rotation redesign plan

**Files:**
- Modify: `crates/credential/src/rotation/mod.rs` — add module-level doc noting future v2 integration

Add doc note at top of rotation module:

```rust
//! # Future: v2 Credential trait integration
//!
//! This module currently defines its own `TestableCredential` and `RotatableCredential`
//! traits. The plan is to integrate with the v2 `Credential` trait:
//! - `TestableCredential::test()` → use `Credential::test()` (TESTABLE const)
//! - `RotatableCredential` → new opt-in extension trait requiring `Credential`
//! - `RotationScheduler` → use `CredentialResolver` for state access
//!
//! Tracked in: rotation v2 redesign task
```

**Step 1:** Add doc comment
**Step 2:** Run `rtk cargo check -p nebula-credential --features rotation`
**Step 3:** Commit: `docs(credential): document rotation v2 integration plan`

---

## Phase 4: Public API audit & polish

### Task 4.1: Audit re-exports for DX

**Files:**
- Modify: `crates/credential/src/lib.rs`

Current: 42+ items exported at root. Review each for necessity:

**Keep (core DX):**
- `Credential`, `CredentialState`, `CredentialKey`, `CredentialId`
- `AnyCredential`, `CredentialDescription`, `CredentialMetadata`, `CredentialSnapshot`
- `CredentialContext`
- All 13 AuthScheme types (they ARE the API surface for consumers)
- All 5 built-in credential impls (`ApiKeyCredential`, etc.)
- `CredentialError`, `CryptoError`, `ValidationError`
- `SecretString`, `EncryptedData`, `EncryptionKey`, `encrypt`, `decrypt`
- `NoPendingState`, `PendingState`, `PendingToken`
- `ResolveResult`, `StaticResolveResult`, `RefreshOutcome`, `RefreshPolicy`
- `InteractionRequest`, `DisplayData`, `UserInput`
- `StaticProtocol`

**Keep (runtime/framework):**
- `CredentialStore`, `StoredCredential`, `PutMode`, `StoreError`, `InMemoryStore`
- `PendingStateStore`, `PendingStoreError`, `InMemoryPendingStore`
- `CredentialRegistry`, `RegistryError`
- `CredentialResolver`, `ResolveError`
- `RefreshCoordinator`, `RefreshAttempt`
- `CredentialHandle`
- `execute_resolve`, `execute_continue`, `ResolveResponse`, `ExecutorError`
- All layer types

**Consider removing from root (too granular):**
- `RetryAdvice`, `RefreshErrorKind`, `ResolutionStage` — these are sub-types of `CredentialError`, access via `CredentialError::*` or `error::*` module
- `SnapshotError` — new type from Phase 1, access via `snapshot::SnapshotError` or include in root

**Step 1:** Review and decide
**Step 2:** Move niche types to module-level access only (if any)
**Step 3:** Ensure `nebula-sdk` re-exports still work
**Step 4:** Run `rtk cargo check --workspace`
**Step 5:** Commit: `refactor(credential): clean up public re-exports`

### Task 4.2: Verify Debug/Clone/Serialize on all public types

**Files:**
- Various files in `crates/credential/src/`

Per Rust API Guidelines (C-DEBUG, C-COMMON-TRAITS), all public types must implement `Debug`. Config types must implement `Serialize + Deserialize`. Check:

- [ ] All error types: `Debug` ✓ (thiserror provides it)
- [ ] `CredentialSnapshot`: `Debug` needed (with redaction for secrets)
- [ ] `CredentialHandle<S>`: `Debug` ✓ (manual impl)
- [ ] `CredentialContext`: `Debug` ✓ (check)
- [ ] `RefreshPolicy`: `Debug + Clone + Serialize + Deserialize`
- [ ] `PendingToken`: `Debug + Serialize + Deserialize`
- [ ] `StoredCredential`: check all derives
- [ ] Config types (`CacheConfig`, `GracePeriodConfig`): `Serialize + Deserialize`

**Step 1:** Grep for pub structs/enums without Debug
**Step 2:** Add missing derives
**Step 3:** Run `rtk cargo check -p nebula-credential`
**Step 4:** Commit: `fix(credential): add missing Debug/Clone/Serialize derives`

---

## Phase 5: Integration tests

### Task 5.1: End-to-end resolve → snapshot → typed access test

**Files:**
- Create: `crates/credential/tests/integration/typed_access.rs`
- Modify: `crates/credential/tests/mod.rs` (add integration module)

Test the full flow:
1. Create `InMemoryStore` with a stored `ApiKeyCredential` state
2. Create `CredentialResolver`
3. Resolve → get `CredentialHandle<BearerToken>`
4. Create `CredentialSnapshot::new::<BearerToken>(...)`
5. Call `snapshot.project::<BearerToken>()` → success
6. Call `snapshot.project::<DatabaseAuth>()` → `SchemeMismatch` error

```rust
#[tokio::test]
async fn resolve_to_typed_snapshot() {
    let store = Arc::new(InMemoryStore::new());
    // ... store a bearer token credential ...
    let resolver = CredentialResolver::new(store);
    let handle = resolver.resolve::<ApiKeyCredential>("test-cred").await.unwrap();

    let snapshot = CredentialSnapshot::new(
        ApiKeyCredential::KEY,
        CredentialMetadata::default(),
        (*handle.snapshot()).clone(),
    );

    // Typed access works
    let token = snapshot.project::<BearerToken>().unwrap();
    assert_eq!(token.expose().expose_secret(|s| s.to_owned()), "test-key");

    // Wrong type fails cleanly
    let err = snapshot.project::<DatabaseAuth>().unwrap_err();
    assert!(matches!(err, SnapshotError::SchemeMismatch { .. }));
}
```

**Step 1:** Write the test
**Step 2:** Run `rtk cargo nextest run -p nebula-credential`
**Step 3:** Commit: `test(credential): add typed access integration test`

### Task 5.2: RefreshCoordinator thundering herd test

**Files:**
- Add test in `crates/credential/src/resolver.rs` or `tests/integration/`

Spawn 10 concurrent `resolve_with_refresh` calls on an expiring credential. Verify:
- Only 1 actual refresh happens (check via AtomicU32 counter in test credential)
- All 10 callers get the refreshed token

**Step 1:** Write the test
**Step 2:** Run `rtk cargo nextest run -p nebula-credential`
**Step 3:** Commit: `test(credential): add thundering herd integration test`

### Task 5.3: PendingState lifecycle test

**Files:**
- Add test in `crates/credential/tests/integration/`

Test the full interactive flow:
1. Call `execute_resolve::<OAuth2Credential>()` → `ResolveResponse::Pending`
2. Extract `PendingToken`
3. Call `execute_continue::<OAuth2Credential>(token, user_input)` → Complete

This requires mocking the OAuth2 HTTP calls, so use a test credential that simulates the two-step flow.

**Step 1:** Write test credential with `INTERACTIVE: true`
**Step 2:** Write the lifecycle test
**Step 3:** Run `rtk cargo nextest run -p nebula-credential`
**Step 4:** Commit: `test(credential): add pending state lifecycle integration test`

---

## Phase 6: CredentialContext composition support

### Task 6.1: Add resolver to CredentialContext

**Files:**
- Modify: `crates/credential/src/context.rs`

Add an optional `resolver` field for credential composition (AWS Assume Role depending on base credential):

```rust
// In CredentialContext:
resolver: Option<Arc<dyn CredentialResolverRef>>,

/// Trait for type-erased credential resolution within credential code.
pub trait CredentialResolverRef: Send + Sync {
    fn resolve_scheme(
        &self,
        credential_id: &str,
        expected_kind: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send + Sync>, ResolveError>> + Send + '_>>;
}
```

With a helper on `CredentialContext`:

```rust
pub async fn resolve_credential<S: AuthScheme>(&self, credential_id: &str) -> Result<S, CredentialError> {
    let resolver = self.resolver.as_ref()
        .ok_or(CredentialError::CompositionNotAvailable)?;
    let boxed = resolver.resolve_scheme(credential_id, S::KIND).await
        .map_err(|e| CredentialError::CompositionFailed { source: Box::new(e) })?;
    *boxed.downcast::<S>()
        .map_err(|_| CredentialError::SchemeMismatch {
            expected: S::KIND.to_string(),
            actual: "unknown".to_string(),
        })
}
```

**Step 1:** Define `CredentialResolverRef` trait
**Step 2:** Add field + builder method to `CredentialContext`
**Step 3:** Add `resolve_credential::<S>()` helper
**Step 4:** Write test for composition flow
**Step 5:** Run `rtk cargo nextest run -p nebula-credential`
**Step 6:** Commit: `feat(credential): add resolver to CredentialContext for credential composition`

---

## Phase 7: Context file updates

### Task 7.1: Update .claude/crates/credential.md

After all phases, update the context file with:
- New CredentialSnapshot design (typed, Box<dyn Any>)
- Rotation module feature-gated
- CredentialContext now has resolver for composition
- SnapshotError as new error type

### Task 7.2: Update .claude/active-work.md

Update "In Progress" / "Recently Completed" sections.

---

## Execution Order & Dependencies

```
Phase 1 (Tasks 1.1 → 1.2 → 1.3)  ← CRITICAL PATH, do first
    ↓
Phase 2 (Task 2.1)                ← Quick fix, can parallel with Phase 3
    ↓
Phase 3 (Tasks 3.1 → 3.2)        ← Rotation feature gate
    ↓
Phase 4 (Tasks 4.1, 4.2)         ← API polish (can parallel)
    ↓
Phase 5 (Tasks 5.1 → 5.2 → 5.3) ← Integration tests
    ↓
Phase 6 (Task 6.1)               ← Composition support
    ↓
Phase 7 (Tasks 7.1, 7.2)         ← Context updates (always last)
```

**Parallelizable:** Phase 2 + Phase 3 can run in parallel. Tasks 4.1 + 4.2 can run in parallel.

---

## What this achieves (vs n8n)

| Capability | n8n | Nebula (after this plan) |
|-----------|-----|------------------------|
| Credential type safety | ❌ String names, runtime errors | ✅ Compile-time typed `credential_typed::<BearerToken>()` |
| Token refresh | ❌ Only on HTTP 401 | ✅ Proactive with early_refresh + jitter + circuit breaker |
| Credential testing | ✅ Declarative HTTP test | ✅ `Credential::test()` (more flexible, not HTTP-only) |
| Rotation | ❌ None | ✅ Full framework (feature-gated, redesign pending) |
| Multi-tenant isolation | ❌ Basic user scoping | ✅ ScopeLayer with fail-fast |
| Audit trail | ❌ None | ✅ AuditLayer with redacted metadata |
| Interactive flows | ✅ OAuth2 via UI | ✅ Generic PendingState model (OAuth2, SAML, Device Code, WebAuthn) |
| Credential composition | ❌ None | ✅ `ctx.resolve_credential::<BearerToken>(base_id)` |
| Thundering herd prevention | ❌ None | ✅ RefreshCoordinator with CAS + scopeguard |
| DX macros | ❌ Manual class per credential | ✅ `#[derive(Credential)]` generates boilerplate |
