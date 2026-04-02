# nebula-credential v1 — Unified Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement all v1 deliverables for nebula-credential: structural moves, 17 bugfixes, new features, and DX improvements — producing a shippable v1 from HLD v1.5.

**Architecture:** 5 phases in dependency order. Phase 0 (structural) unblocks everything. Phases 1-2 (critical + core bugs) ship safety. Phase 3 (security) hardens secrets. Phase 4 (features) adds designed capabilities. Phase 5 (DX) polishes the developer experience. Within each phase, tasks are parallelizable unless noted.

**Tech Stack:** Rust 2024 edition, `aes-gcm`, `zeroize`, `secrecy`, `tokio`, `scopeguard`, `moka`, `serde`, `nebula-resilience`, `nebula-eventbus`

**Source documents:**
- `docs/plans/nebula-credential-hld-v1.md` (HLD v1.5, 1700+ lines)
- `docs/plans/2026-03-31-credential-bugfixes.md` (detailed bug analysis)
- `.claude/crates/credential.md` (crate context)

---

## Phase 0: Structural Moves

These changes are prerequisites — they move foundational types to the right crates and flatten the module structure.

### Task 0.1: Move SecretString + serde_secret to nebula-core

**Files:**
- Create: `crates/core/src/secret_string.rs`
- Create: `crates/core/src/serde_secret.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/core/Cargo.toml` (add `zeroize` dep)
- Modify: `crates/credential/src/utils/mod.rs`
- Modify: `crates/credential/src/lib.rs`

**Step 1:** Copy `crates/credential/src/utils/secret_string.rs` → `crates/core/src/secret_string.rs`. Fix internal imports.

**Step 2:** Extract `serde_secret` module from `crates/credential/src/utils/mod.rs` → `crates/core/src/serde_secret.rs`. Update `SecretString` import path to `crate::SecretString`.

**Step 3:** In `crates/core/Cargo.toml`, add:
```toml
zeroize = { workspace = true, features = ["zeroize_derive"] }
```

**Step 4:** In `crates/core/src/lib.rs`, add:
```rust
pub mod secret_string;
pub mod serde_secret;
pub use secret_string::SecretString;
```

**Step 5:** In `crates/credential/src/utils/mod.rs`, replace secret_string module with re-export:
```rust
// Secret string moved to nebula-core
pub use nebula_core::SecretString;
pub use nebula_core::serde_secret;
```

**Step 6:** In `crates/credential/src/lib.rs`, update re-export:
```rust
pub use nebula_core::SecretString;
```

**Step 7:** Run `rtk cargo check --workspace`

**Step 8:** Fix any broken imports across workspace (grep for `crate::utils::SecretString`, `crate::utils::serde_secret`).

**Step 9:** Run `rtk cargo nextest run -p nebula-core && rtk cargo nextest run -p nebula-credential`

**Step 10:** Commit: `refactor(core): move SecretString + serde_secret from credential to core`

---

### Task 0.2: Flatten utils/ directory

**Files:**
- Move: `crates/credential/src/utils/crypto.rs` → `crates/credential/src/crypto.rs`
- Move: `crates/credential/src/utils/retry.rs` → `crates/credential/src/retry.rs`
- Delete: `crates/credential/src/utils/mod.rs`
- Delete: `crates/credential/src/utils/` directory
- Modify: `crates/credential/src/lib.rs`

**Step 1:** Move `serde_base64` module from `utils/mod.rs` into `crypto.rs` (append at end of file).

**Step 2:** Move files:
```bash
mv crates/credential/src/utils/crypto.rs crates/credential/src/crypto.rs
mv crates/credential/src/utils/retry.rs crates/credential/src/retry.rs
```

**Step 3:** Delete utils directory:
```bash
rm crates/credential/src/utils/mod.rs
rm crates/credential/src/utils/secret_string.rs
rmdir crates/credential/src/utils
```

**Step 4:** In `crates/credential/src/lib.rs`, replace:
```rust
pub mod utils;
```
with:
```rust
pub mod crypto;
pub mod retry;
// SecretString + serde_secret re-exported from nebula-core
pub use nebula_core::{SecretString, serde_secret};
```

**Step 5:** Fix all `crate::utils::crypto::` → `crate::crypto::` and `crate::utils::retry::` → `crate::retry::` imports across the credential crate.

**Step 6:** Fix all `crate::utils::serde_secret` → `nebula_core::serde_secret` and `crate::utils::serde_base64` → `crate::crypto::serde_base64` references.

**Step 7:** Run `rtk cargo check -p nebula-credential && rtk cargo nextest run -p nebula-credential`

**Step 8:** Commit: `refactor(credential): flatten utils/ — crypto.rs and retry.rs to root`

---

## Phase 1: Critical Bugs (B6, B10)

Ship these first. Both are correctness/security issues.

### Task 1.1: B6 CRITICAL — verify_owner fails open

**Files:**
- Modify: `crates/credential/src/layer/scope.rs:198-215`

**Step 1:** Write failing test in `scope.rs` mod tests:
```rust
#[tokio::test]
async fn unscoped_credential_rejected_for_non_admin() {
    let store = InMemoryStore::new();
    let cred = StoredCredential {
        id: "unscoped".into(),
        data: b"data".to_vec(),
        state_kind: "bearer".into(),
        state_version: 1,
        version: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        expires_at: None,
        metadata: serde_json::Map::new(), // NO owner_id
    };
    store.put(cred, PutMode::CreateOnly).await.unwrap();
    let resolver = Arc::new(FixedOwnerResolver("tenant-a"));
    let scoped = ScopeLayer::new(store, resolver);
    let result = scoped.get("unscoped").await;
    assert!(matches!(result, Err(StoreError::NotFound { .. })));
}
```

**Step 2:** Run `rtk cargo nextest run -p nebula-credential -- unscoped_credential_rejected` — expect FAIL.

**Step 3:** Fix `verify_owner` — replace the fallthrough `Ok(())` with `Err(NotFound)`:
```rust
match credential.metadata.get(OWNER_KEY) {
    Some(Value::String(stored_owner)) if stored_owner == caller_owner => Ok(()),
    Some(Value::String(_)) => Err(StoreError::NotFound { id: id.to_owned() }),
    _ => Err(StoreError::NotFound { id: id.to_owned() }), // fail-closed
}
```

**Step 4:** Run `rtk cargo nextest run -p nebula-credential` — all pass.

**Step 5:** Commit: `fix(credential)!: reject unscoped credentials for non-admin callers [B6]`

---

### Task 1.2: B10 CRITICAL — CAS on missing row creates instead of NotFound

**Files:**
- Modify: `crates/credential/src/store_memory.rs:78-91`

**Step 1:** Write failing test:
```rust
#[tokio::test]
async fn cas_on_missing_credential_returns_not_found() {
    let store = InMemoryStore::new();
    let cred = make_credential("nonexistent");
    let result = store.put(cred, PutMode::CompareAndSwap { expected_version: 0 }).await;
    assert!(matches!(result, Err(StoreError::NotFound { .. })));
}
```

**Step 2:** Run test — expect FAIL (currently creates).

**Step 3:** Fix CAS arm — add early return for missing row:
```rust
PutMode::CompareAndSwap { expected_version } => {
    let Some(existing) = data.get(&credential.id) else {
        return Err(StoreError::NotFound { id: credential.id.clone() });
    };
    if existing.version != expected_version {
        return Err(StoreError::VersionConflict { /* ... */ });
    }
    // proceed with update
}
```

**Step 4:** Run `rtk cargo nextest run -p nebula-credential` — all pass.

**Step 5:** Commit: `fix(credential)!: CAS on missing row returns NotFound [B10]`

---

## Phase 2: Core Fixes (B8, B7, B5, B11, B12, B14)

### Task 2.1: B8 — scopeguard calls complete() on panic

**Files:**
- Modify: `crates/credential/src/resolver.rs:154-169`
- Modify: `crates/credential/src/refresh.rs` (change `in_flight` to `parking_lot::Mutex`)

**Step 1:** Change `in_flight` from `tokio::sync::Mutex` to `parking_lot::Mutex` in `refresh.rs`. No `.await` is held across this lock.

**Step 2:** Add `complete_sync(&self, id: &str)` method to `RefreshCoordinator`.

**Step 3:** Update Winner arm scopeguard in `resolver.rs` to call both `complete_sync` and `notify_waiters`.

**Step 4:** Write test: coordinator entry cleaned up after simulated panic (drop without complete).

**Step 5:** Run `rtk cargo nextest run -p nebula-credential`

**Step 6:** Commit: `fix(credential): scopeguard calls complete() on winner panic [B8]`

---

### Task 2.2: B7 — CAS retry in perform_refresh

**Files:**
- Modify: `crates/credential/src/resolver.rs:277-285`

**Step 1:** Replace single CAS write with retry loop (max 2 attempts). On `VersionConflict`: re-read stored credential, retry with same refreshed state and new expected_version.

**Step 2:** Write test: concurrent version bump causes retry, second attempt succeeds.

**Step 3:** Run `rtk cargo nextest run -p nebula-credential`

**Step 4:** Commit: `fix(credential): retry CAS write on VersionConflict, reuse token [B7]`

---

### Task 2.3: B5 — ScopeLayer list/exists filtering

**Files:**
- Modify: `crates/credential/src/layer/scope.rs:171-187`

**Depends on:** Task 1.1 (B6 — verify_owner must be fail-closed first)

**Step 1:** Write failing tests for filtered list and scoped exists.

**Step 2:** Implement filtered `list()` — iterate IDs, check ownership via inner `get()`.

**Step 3:** Implement scoped `exists()` — delegate to scoped `get()`.

**Step 4:** Run `rtk cargo nextest run -p nebula-credential`

**Step 5:** Commit: `fix(credential)!: scope-filter list() and exists() in ScopeLayer [B5]`

---

### Task 2.4: B11 — CredentialEvent + resolver emission

**Files:**
- Create: `crates/core/src/credential_event.rs`
- Modify: `crates/core/src/lib.rs`
- Modify: `crates/credential/src/resolver.rs`
- Modify: `crates/credential/src/lib.rs`

**Step 1:** Create `CredentialEvent` in nebula-core (2 variants for now):
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialEvent {
    Refreshed { credential_id: String },
    Revoked { credential_id: String },
}
```

**Step 2:** Add `event_bus: Option<Arc<EventBus<CredentialEvent>>>` to `CredentialResolver`.

**Step 3:** After successful refresh CAS write in `perform_refresh`, emit `Refreshed`.

**Step 4:** Write test: resolver emits event after refresh.

**Step 5:** Run `rtk cargo check -p nebula-core && rtk cargo nextest run -p nebula-credential`

**Step 6:** Commit: `feat(core): add CredentialEvent` then `feat(credential): emit events after refresh [B11]`

---

### Task 2.5: B12 — StoredCredential.credential_key field

**Files:**
- Modify: `crates/credential/src/store.rs`
- Modify: `crates/credential/src/store_memory.rs`
- Modify: all test files constructing `StoredCredential`

**Step 1:** Add `pub credential_key: String` to `StoredCredential`.

**Step 2:** Update `InMemoryStore` and all construction sites.

**Step 3:** Run `rtk cargo nextest run -p nebula-credential`

**Step 4:** Commit: `feat(credential)!: add credential_key to StoredCredential [B12]`

---

### Task 2.6: B14 — Global refresh concurrency limiter

**Files:**
- Modify: `crates/credential/src/refresh.rs`

**Step 1:** Add `refresh_semaphore: Arc<Semaphore>` to `RefreshCoordinator`.

**Step 2:** Add `with_max_concurrent(max: usize)` constructor. Default: 32.

**Step 3:** Winner acquires permit before `Credential::refresh()`.

**Step 4:** Write test: 100 concurrent refreshes with max_concurrent(5).

**Step 5:** Run `rtk cargo nextest run -p nebula-credential`

**Step 6:** Commit: `fix(credential): add refresh concurrency limiter [B14]`

---

## Phase 3: Security Fixes (B1, B2, B9)

**Depends on:** Phase 0 (serde_secret must be in core for correct import paths)

### Task 3.1: B1 — SecretString serde roundtrip for scheme types

**Files:**
- Modify: `crates/credential/src/scheme/bearer.rs`
- Modify: `crates/credential/src/scheme/basic.rs`
- Modify: `crates/credential/src/scheme/database.rs`
- Modify: `crates/credential/src/scheme/header.rs`
- Modify: `crates/credential/src/scheme/api_key.rs`
- Modify: `crates/credential/src/scheme/hmac.rs`
- Modify: any other scheme with SecretString fields

**Step 1:** Write roundtrip test per scheme:
```rust
#[test]
fn bearer_token_serde_roundtrip() {
    let token = BearerToken::new(SecretString::new("my-key".into()));
    let json = serde_json::to_string(&token).unwrap();
    let recovered: BearerToken = serde_json::from_str(&json).unwrap();
    recovered.expose().expose_secret(|s| assert_eq!(s, "my-key"));
}
```

**Step 2:** Run — expect FAIL (json = `{"token":"[REDACTED]"}`).

**Step 3:** Add `#[serde(with = "nebula_core::serde_secret")]` on every `SecretString` field in scheme types.

**Step 4:** Run `rtk cargo nextest run -p nebula-credential` — all pass.

**Step 5:** Commit: `fix(credential)!: use serde_secret for SecretString fields in schemes [B1]`

---

### Task 3.2: B2 — OAuth2State secrets as SecretString

**Files:**
- Modify: `crates/credential/src/credentials/oauth2.rs:44-66`
- Modify: `crates/credential/src/credentials/oauth2_flow.rs` (accessors)

**Step 1:** Write test: `OAuth2State` debug output redacts access_token.

**Step 2:** Convert `access_token: String` → `SecretString`, `refresh_token: Option<String>` → `Option<SecretString>`, `client_id: String` → `SecretString`. All with `#[serde(with = "nebula_core::serde_secret")]`.

**Step 3:** Replace `#[derive(Debug)]` with manual `Debug` impl that redacts all secret fields.

**Step 4:** Fix all call sites (grep `.access_token`, `.refresh_token`, `.client_id`).

**Step 5:** Run `rtk cargo nextest run -p nebula-credential`

**Step 6:** Commit: `fix(credential)!: convert OAuth2State secrets to SecretString [B2]`

---

### Task 3.3: B9 — CredentialRotationEvent leaks state

**Files:**
- Modify: `crates/credential/src/rotation/events.rs:65-72`

**Requires:** `rotation` feature flag.

**Step 1:** Replace `new_state: serde_json::Value` with `generation: u64`.

**Step 2:** Fix all usages of `.new_state` in rotation module.

**Step 3:** Run `rtk cargo nextest run -p nebula-credential --features rotation`

**Step 4:** Commit: `fix(credential)!: remove state from CredentialRotationEvent [B9]`

---

## Phase 4: New Features

### Task 4.1: CredentialPhase state machine

**Files:**
- Create: `crates/credential/src/phase.rs`
- Modify: `crates/credential/src/lib.rs`

**Depends on:** Task 2.5 (StoredCredential needs phase field)

**Step 1:** Create `phase.rs` with `CredentialPhase` enum (7 states), `is_usable()`, `is_terminal()`, `can_transition_to()`, `InvalidTransition` error.

**Step 2:** Add `pub phase: CredentialPhase` to `StoredCredential` (default: `Active` for existing data).

**Step 3:** Write comprehensive transition tests (21 valid transitions + invalid transition rejection).

**Step 4:** Run `rtk cargo nextest run -p nebula-credential`

**Step 5:** Commit: `feat(credential): add CredentialPhase state machine`

---

### Task 4.2: CredentialHandle::Clone shares Arc<ArcSwap>

**Files:**
- Modify: `crates/credential/src/handle.rs`

**Step 1:** Change internal structure:
```rust
pub struct CredentialHandle<S: AuthScheme> {
    inner: Arc<ArcSwap<S>>,  // was: ArcSwap<S>
    credential_id: String,
}
```

**Step 2:** Update `Clone` — now shares Arc (not creates new ArcSwap).

**Step 3:** Update `snapshot()`, `replace()` to go through `self.inner`.

**Step 4:** Write test: rotate on original → clone sees new value.

**Step 5:** Run `rtk cargo nextest run -p nebula-credential`

**Step 6:** Commit: `feat(credential): CredentialHandle Clone shares ArcSwap`

---

### Task 4.3: ListFilter / ListPage pagination

**Files:**
- Modify: `crates/credential/src/store.rs`
- Modify: `crates/credential/src/store_memory.rs`
- Modify: `crates/credential/src/layer/scope.rs`
- Modify: `crates/credential/src/layer/cache.rs`
- Modify: `crates/credential/src/layer/audit.rs`
- Modify: `crates/credential/src/layer/encryption.rs`

**Depends on:** Task 2.3 (B5 — scope filtering must work first)

**Step 1:** Define types in `store.rs`:
```rust
pub struct ListFilter {
    pub state_kind: Option<String>,
    pub credential_key: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}
pub struct ListPage {
    pub ids: Vec<String>,
    pub cursor: Option<String>,
}
```

**Step 2:** Change `CredentialStore::list()` signature. Update all layer impls + InMemoryStore.

**Step 3:** Run `rtk cargo nextest run -p nebula-credential`

**Step 4:** Commit: `feat(credential)!: add ListFilter/ListPage pagination`

---

### Task 4.4: DatabaseAuth.expires_at (B15) + extensions (v1.5)

**Files:**
- Modify: `crates/credential/src/scheme/database.rs`

**Step 1:** Add fields:
```rust
pub expires_at: Option<DateTime<Utc>>,
#[serde(default)]
pub extensions: serde_json::Map<String, serde_json::Value>,
```

**Step 2:** Implement `AuthScheme::expires_at()`.

**Step 3:** Add builder methods: `with_extension(key, value)`, `extension(key)`.

**Step 4:** Run `rtk cargo nextest run -p nebula-credential`

**Step 5:** Commit: `feat(credential): add expires_at + extensions to DatabaseAuth [B15]`

---

### Task 4.5: ActionDependencies::credential_slots (B16)

**Files:**
- Modify: `crates/action/src/dependency.rs`

**Step 1:** Add `CredentialSlot` struct and `credential_slots()` method with default impl.

**Step 2:** Existing `credential()` remains for backward compat.

**Step 3:** Run `rtk cargo nextest run -p nebula-action`

**Step 4:** Commit: `feat(action): add credential_slots() for multi-credential actions [B16]`

---

### Task 4.6: SshAuthMethod::Certificate (B17)

**Files:**
- Modify: `crates/credential/src/scheme/ssh.rs`

**Step 1:** Add `Certificate` variant to `#[non_exhaustive]` enum.

**Step 2:** Add `SshAuth::with_certificate()` constructor. Update Debug redaction.

**Step 3:** Run `rtk cargo nextest run -p nebula-credential`

**Step 4:** Commit: `feat(credential): add Certificate variant to SshAuthMethod [B17]`

---

### Task 4.7: CredentialRegistry returns CredentialSnapshot (B13)

**Files:**
- Modify: `crates/credential/src/registry.rs`

**Step 1:** Change `register::<C>()` to capture `CredentialSnapshot::new()` in the handler closure.

**Step 2:** Change `project()` return type to `Result<CredentialSnapshot, RegistryError>`.

**Step 3:** Update resolver call sites.

**Step 4:** Add `kinds()` iterator and `description(key)` introspection methods.

**Step 5:** Run `rtk cargo nextest run -p nebula-credential`

**Step 6:** Commit: `refactor(credential)!: registry returns CredentialSnapshot + introspection [B13]`

---

## Phase 5: DX & Cleanup

### Task 5.1: B3 — RefreshCoordinator remove_credential

**Files:**
- Modify: `crates/credential/src/refresh.rs`

**Step 1:** Add `remove_credential(id)` method. Write test.

**Step 2:** Commit: `fix(credential): add remove_credential to RefreshCoordinator [B3]`

---

### Task 5.2: B4 — Cache invalidation documentation test

**Files:**
- Modify: `crates/credential/src/layer/cache.rs`

**Step 1:** Write test documenting correct invalidate-then-populate behavior. Not a bug.

**Step 2:** Commit: `test(credential): document cache invalidation behavior [B4]`

---

### Task 5.3: Re-exports for plugin authors

**Files:**
- Modify: `crates/credential/src/lib.rs`

**Step 1:** Add:
```rust
pub use nebula_core::AuthScheme;
pub use nebula_parameter::{Parameter, ParameterCollection, ParameterValues};
```

**Step 2:** Run `rtk cargo check --workspace`

**Step 3:** Commit: `refactor(credential): re-export parameter and core types for plugins`

---

### Task 5.4: SecretStore minimal API

**Files:**
- Create: `crates/credential/src/secret_store.rs`
- Modify: `crates/credential/src/lib.rs`

**Step 1:** Implement `SecretStore<B>` with put/get/delete/list — thin wrapper over `EncryptionLayer<B>`.

**Step 2:** Write usage tests.

**Step 3:** Commit: `feat(credential): add SecretStore minimal API`

---

### Task 5.5: StoredCredential PartialEq

**Files:**
- Modify: `crates/credential/src/store.rs`

**Step 1:** Add `#[derive(PartialEq)]` to `StoredCredential`.

**Step 2:** Commit: `refactor(credential): derive PartialEq on StoredCredential`

---

### Task 5.6: health_check on CredentialStore

**Files:**
- Modify: `crates/credential/src/store.rs`
- Modify: `crates/credential/src/store_memory.rs`

**Step 1:** Add `health_check()` with default impl `Ok(())`.

**Step 2:** Add `StoreError::Unavailable { reason: String }` variant.

**Step 3:** Commit: `feat(credential): add health_check() to CredentialStore`

---

## Execution Order

```
Phase 0 (structural — do first):
  0.1  SecretString → core
  0.2  Flatten utils/

Phase 1 (critical — immediately after):
  1.1  B6  verify_owner fail-closed
  1.2  B10 CAS on missing row

Phase 2 (core — week 1, parallel groups):
  Group A: 2.1 B8 (scopeguard) + 2.2 B7 (CAS retry)
  Group B: 2.3 B5 (list/exists filter) ← depends on 1.1
  Group C: 2.4 B11 (CredentialEvent) + 2.5 B12 (credential_key)
  Group D: 2.6 B14 (refresh semaphore)

Phase 3 (security — week 1, after Phase 0):
  3.1  B1  serde_secret on schemes
  3.2  B2  OAuth2State SecretString
  3.3  B9  rotation event leak

Phase 4 (features — week 2):
  4.1  CredentialPhase ← depends on 2.5
  4.2  Handle::Clone ← independent
  4.3  ListFilter/ListPage ← depends on 2.3
  4.4  DatabaseAuth extensions ← independent
  4.5  ActionDependencies slots ← independent
  4.6  SshAuthMethod::Certificate ← independent
  4.7  Registry returns Snapshot ← depends on 2.5

Phase 5 (DX — week 2-3):
  5.1-5.6 all independent, parallel
```

## Verification

After all phases:

```bash
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run -p nebula-credential && rtk cargo nextest run -p nebula-credential --features rotation && rtk cargo nextest run -p nebula-core && rtk cargo nextest run -p nebula-action && rtk cargo test --doc -p nebula-credential && rtk cargo test --doc -p nebula-core
```
