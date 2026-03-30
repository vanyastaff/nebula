# nebula-credential v2 Completion Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Complete nebula-credential v2 by migrating v1 working code to v2 interfaces, fixing architectural mismatches, implementing missing subsystems, and cleaning up the public API.

**Architecture:** Incremental migration, NOT big-bang deletion. V1 has ~3,200 LOC of real working logic (OAuth2 flows, 5 storage backends, rotation framework, manager orchestration) that must be preserved. Strategy: adapt v1 implementations to v2 trait contracts, then remove v1 traits/re-exports once everything uses v2.

**Tech Stack:** Rust 1.93, Edition 2024, RPITIT, tokio, AES-256-GCM, moka, zeroize, arc-swap, scopeguard, chrono, serde

**Source of truth:** `crates/credential/plans/credential-hld-v6-final.md` + companion HLDs

**V1 code audit:** `crates/credential/plans/credential-hld-v7-audit.md`

---

## V1 Assets Inventory (must preserve)

Before any work, this is what v1 has that v2 doesn't:

| Asset | Location | LOC | Migration target |
|-------|----------|-----|-----------------|
| OAuth2 token exchange (3 grants, PKCE, device flow) | `protocols/oauth2/flow.rs` | ~350 | v2 OAuth2Credential impl |
| OAuth2 state (expiration, bearer header) | `protocols/oauth2/state.rs` | ~50 | v2 `scheme/oauth2.rs` |
| OAuth2 config builder | `protocols/oauth2/config.rs` | ~100 | Keep as builder |
| API key parameter extraction | `protocols/api_key.rs` | ~40 | Merge into `credentials/api_key.rs` |
| Basic auth Base64 encoding | `protocols/basic_auth.rs` | ~35 | Merge into `credentials/basic_auth.rs` |
| Database credential defaults (port 5432, ssl) | `protocols/database.rs` | ~45 | Merge into `credentials/database.rs` |
| Header auth parameter extraction | `protocols/header_auth.rs` | ~35 | New v2 HeaderAuthCredential |
| LDAP parameter validation (port 389 default) | `protocols/ldap/` | ~60 | New v2 LdapCredential |
| LocalStorageProvider (atomic writes, file locking) | `providers/local.rs` | ~200+ | Adapt to v2 `CredentialStore` |
| PostgresStorageProvider (KV over nebula-storage) | `providers/postgres.rs` | ~200+ | Adapt to v2 `CredentialStore` |
| VaultProvider (KV v2, auth, token renewal) | `providers/vault.rs` | ~200+ | Adapt to v2 `CredentialStore` |
| AwsSecretsProvider (KMS, region auto-detect) | `providers/aws.rs` | ~200+ | Adapt to v2 `CredentialStore` |
| K8sSecretsProvider (namespace isolation, RBAC) | `providers/kubernetes.rs` | ~200+ | Adapt to v2 `CredentialStore` |
| MockStorageProvider (error simulation) | `providers/mock.rs` | ~50 | Merge into v2 test utils |
| CredentialManager (scope, encryption, dispatch) | `manager/manager.rs` | ~200+ | Logic absorbed into CredentialResolver |
| CacheLayer (moka LRU + TTL + stats) | `manager/cache.rs` | ~70 | Adapt to v2 `layer/cache.rs` |
| Credential validation vs rotation policies | `manager/validation.rs` | ~60 | Keep in resolver |
| Protocol registry (dynamic dispatch) | `manager/registry.rs` | ~100+ | Merge into v2 CredentialRegistry |
| 4 rotation policies (Periodic, BeforeExpiry, Scheduled, Manual) | `rotation/policy.rs` | ~150+ | Keep as-is, new module |
| Rotation scheduler (jitter) | `rotation/scheduler.rs` | ~80+ | Keep |
| Blue-green rotation pattern | `rotation/blue_green.rs` | ~100+ | Keep |
| Grace period management | `rotation/grace_period.rs` | ~80+ | Keep |
| Rotation transaction lifecycle | `rotation/transaction.rs` | ~100+ | Keep |
| Rotation backup/restore | `rotation/backup.rs` | ~80+ | Keep |
| Rotation events (EventBus) | `rotation/events.rs` | ~100+ | Keep, wire to v2 |
| RetryPolicy (exponential backoff, jitter) | `utils/retry.rs` | ~120 | Keep as shared util |

**Already shared (no migration needed):**
- `utils/crypto.rs` — AES-256-GCM + Argon2id (used by v2 EncryptionLayer)
- `utils/secret_string.rs` — SecretString with zeroize (used by v2 schemes)
- `core/context.rs` — CredentialContext (used by v2 Credential trait)
- `core/description.rs` — CredentialDescription (used by v2)
- `core/error.rs` — error types (used by v2, will be extended)
- `core/metadata.rs` — CredentialMetadata (used by v2)

---

## Phase 0: Critical Safety Fixes (no migration, just fixes)

> Fix bugs that exist today regardless of v1/v2 migration.

### Task 0.1: Add scopeguard to RefreshCoordinator

**Files:**
- Modify: `crates/credential/Cargo.toml` (add `scopeguard = "1"`)
- Modify: `crates/credential/src/refresh.rs`
- Modify: `crates/credential/src/resolver.rs`

**Problem:** If `perform_refresh()` panics, waiters hang forever.

**Step 1:** Add `scopeguard` dependency.

**Step 2:** Change `RefreshAttempt::Winner` to carry `Arc<Notify>`:

```rust
pub enum RefreshAttempt {
    Winner(Arc<Notify>),
    Waiter(Arc<Notify>),
}
```

Update `try_refresh` to return `Arc<Notify>` for Winner.

**Step 3:** In resolver, wrap winner path with scopeguard:

```rust
RefreshAttempt::Winner(notify) => {
    let _guard = scopeguard::guard(notify, |n| n.notify_waiters());
    let result = self.perform_refresh::<C>(credential_id, state, stored, ctx).await;
    self.refresh_coordinator.complete(credential_id).await;
    result
}
```

**Step 4:** Update all tests in `refresh.rs` for new `RefreshAttempt::Winner(notify)` pattern.

**Step 5:** Run `rtk cargo nextest run -p nebula-credential`.

**Step 6:** Commit: `fix(credential): scopeguard on RefreshCoordinator prevents waiter hang`

---

### Task 0.2: Add early refresh window to resolver

**Files:**
- Modify: `crates/credential/src/resolver.rs`

**Problem:** Currently refreshes only after token already expired. Should refresh within `early_refresh` window (default 5 min before expiry).

**Step 1:** Change expiration check from `exp <= now` to:

```rust
let needs_refresh = state.expires_at().is_some_and(|exp| {
    let remaining = exp - chrono::Utc::now();
    remaining <= chrono::Duration::from_std(C::REFRESH_POLICY.early_refresh)
        .unwrap_or(chrono::Duration::zero())
});
```

**Step 2:** Add test with credential expiring in 4 minutes (should trigger refresh since default early_refresh = 5 min).

**Step 3:** Run tests. Commit: `fix(credential): refresh before expiry using RefreshPolicy.early_refresh`

---

## Phase 1: Core Trait Upgrades

> Upgrade AuthScheme, CredentialHandle, CredentialContext, error model.

### Task 1.1: Extend AuthScheme trait in nebula-core

**Files:**
- Modify: `crates/core/src/auth.rs`
- Modify: `crates/core/Cargo.toml` (serde, chrono if needed)
- Modify: all 5 AuthScheme impls in `crates/credential/src/scheme/`

**Step 1:** Add to `AuthScheme`:
- `const KIND: &'static str`
- `fn expires_at(&self) -> Option<DateTime<Utc>> { None }`
- Bounds: `Serialize + DeserializeOwned`

**Step 2:** Update all 5 impls (BearerToken, BasicAuth, DatabaseAuth, ApiKeyAuth, OAuth2Token).

**Step 3:** Fix compilation workspace-wide. Commit.

```
feat(core): add KIND, expires_at, Serialize bounds to AuthScheme
```

---

### Task 1.2: Switch CredentialHandle to ArcSwap

**Files:**
- Modify: `crates/credential/Cargo.toml` (add `arc-swap = "1"`)
- Modify: `crates/credential/src/credential_handle.rs`

**Step 1:** Replace `Arc<S>` with `ArcSwap<S>`. `snapshot()` returns `Arc<S>`. Add `replace()` for hot-swap.

**Step 2:** Update tests. Commit.

```
feat(credential): CredentialHandle uses ArcSwap for transparent refresh
```

---

### Task 1.3: Extend CredentialContext

**Files:**
- Modify: `crates/credential/src/core/context.rs`

Add `callback_url`, `app_url`, `session_id` with builder methods. Additive, non-breaking. Commit.

---

### Task 1.4: Create v2 error types (extend, don't replace)

**Files:**
- Modify: `crates/credential/src/core/error.rs`

**Approach:** Add v2 variants to existing `CredentialError` rather than creating a separate error type. This avoids a split.

**Step 1:** Add new variants:

```rust
pub enum CredentialError {
    // ... existing v1 variants ...

    // v2 additions:
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("refresh failed: {kind:?}")]
    RefreshFailed {
        kind: RefreshErrorKind,
        retry: RetryAdvice,
        #[source]
        source: Box<dyn std::error::Error + Send + 'static>,
    },

    #[error("revoke failed")]
    RevokeFailed {
        #[source]
        source: Box<dyn std::error::Error + Send + 'static>,
    },

    #[error("composition not available")]
    CompositionNotAvailable,

    #[error("composition failed")]
    CompositionFailed {
        #[source]
        source: Box<dyn std::error::Error + Send + 'static>,
    },

    #[error("scheme mismatch: expected {expected}, got {actual}")]
    SchemeMismatch {
        expected: &'static str,
        actual: String,
    },
}
```

**Step 2:** Add `RefreshErrorKind`, `RetryAdvice`, `ResolutionStage` enums to `core/error.rs`.

**Step 3:** Add `Classify` impls for new variants.

**Step 4:** Commit: `feat(credential): add v2 error variants (RefreshFailed, RetryAdvice, etc.)`

---

### Task 1.5: Harden RefreshCoordinator

**Files:**
- Modify: `crates/credential/src/refresh.rs`
- Modify: `crates/credential/src/resolver.rs`

**Step 1:** Add circuit breaker (failure_counts map, 5 failures in 5 min window).

**Step 2:** Add 60s waiter timeout in resolver.

**Step 3:** Add 30s framework timeout on `C::refresh()` calls.

**Step 4:** Tests + commit.

```
feat(credential): RefreshCoordinator circuit breaker + timeouts
```

---

## Phase 2: Storage Backend Migration

> Adapt v1 storage providers to v2 `CredentialStore` trait. Preserve ALL logic.

### Task 2.1: Migrate LocalStorageProvider → LocalFileStore

**Files:**
- Create: `crates/credential/src/store_local.rs`
- Source: `crates/credential/src/providers/local.rs`

**Approach:** Copy `local.rs` logic into a new file that implements `CredentialStore` (v2 trait). Keep atomic writes, file locking, encryption integration. Adapt the method signatures:
- v1 `store()` → v2 `put()` with `PutMode`
- v1 `retrieve()` → v2 `get()`
- v1 `delete()` → v2 `delete()`
- v1 `list()` → v2 `list()`

Feature-gated: `storage-local`.

**Step 1:** Create `store_local.rs` with `CredentialStore` impl.

**Step 2:** Write integration tests using `tempfile`.

**Step 3:** Commit.

---

### Task 2.2: Migrate PostgresStorageProvider → PostgresStore

**Files:**
- Create: `crates/credential/src/store_postgres.rs`
- Source: `crates/credential/src/providers/postgres.rs`

Same migration pattern. Keep KV abstraction, error mapping, metrics. Feature-gated: `storage-postgres`.

---

### Task 2.3: Migrate VaultProvider → VaultStore

**Files:**
- Create: `crates/credential/src/store_vault.rs`
- Source: `crates/credential/src/providers/vault.rs`

Preserve: KV v2, auth methods (Token, AppRole), token renewal, namespace support, TLS config, retry logic. Feature-gated: `storage-vault`.

---

### Task 2.4: Migrate AwsSecretsProvider → AwsSecretsStore

**Files:**
- Create: `crates/credential/src/store_aws.rs`
- Source: `crates/credential/src/providers/aws.rs`

Preserve: KMS encryption, region auto-detect, custom endpoints (LocalStack), size validation. Feature-gated: `storage-aws`.

---

### Task 2.5: Migrate K8sSecretsProvider → K8sSecretsStore

**Files:**
- Create: `crates/credential/src/store_k8s.rs`
- Source: `crates/credential/src/providers/kubernetes.rs`

Preserve: namespace isolation, RBAC, labels/annotations, in-cluster config, size validation. Feature-gated: `storage-k8s`.

---

### Task 2.6: Migrate CacheLayer from manager

**Files:**
- Create: `crates/credential/src/layer/cache.rs`
- Source: `crates/credential/src/manager/cache.rs`

Adapt v1 moka cache to v2 storage layer. Preserve: LRU + TTL, hit/miss counters, invalidation on put/delete. Cache stores ciphertext (v2 invariant: cache sits below EncryptionLayer).

---

## Phase 3: Protocol Migration

> Migrate v1 protocol implementations to v2 Credential trait impls.

### Task 3.1: Migrate OAuth2 flow to v2

**Files:**
- Create: `crates/credential/src/credentials/oauth2.rs`
- Create: `crates/credential/src/credentials/oauth2_flow.rs` (extracted from v1 `protocols/oauth2/flow.rs`)
- Source: `protocols/oauth2/flow.rs`, `protocols/oauth2/config.rs`, `protocols/oauth2/state.rs`

**Approach:** Create `OAuth2Credential` implementing v2 `Credential` trait:
- `type Scheme = OAuth2Token`
- `type State = OAuth2State` (with refresh_token internals)
- `type Pending = OAuth2Pending`
- `INTERACTIVE = true`, `REFRESHABLE = true`, `REVOCABLE = true`
- `resolve()` → builds auth URL, returns `ResolveResult::Pending` (authorization code) or `ResolveResult::Complete` (client credentials)
- `continue_resolve()` → exchanges code for tokens (uses flow.rs logic)
- `refresh()` → uses refresh_token (uses flow.rs logic)
- `project()` → extracts OAuth2Token from OAuth2State

**Key:** Preserve all HTTP token exchange logic from `flow.rs`. The v2 trait is the new interface; the OAuth2 HTTP calls are the unchanged internals.

Keep `OAuth2Config` builder as-is — it's config, not protocol-specific.

---

### Task 3.2: Enrich existing v2 credentials with v1 logic

**Files:**
- Modify: `crates/credential/src/credentials/api_key.rs` — add parameter extraction from v1 `protocols/api_key.rs`
- Modify: `crates/credential/src/credentials/basic_auth.rs` — add Base64 encoding from v1 `protocols/basic_auth.rs`
- Modify: `crates/credential/src/credentials/database.rs` — add port/ssl defaults from v1 `protocols/database.rs`

Merge v1 `StaticProtocol::build_state()` logic into v2 `Credential::resolve()`.

---

### Task 3.3: Create HeaderAuthCredential (from v1)

**Files:**
- Create: `crates/credential/src/scheme/header.rs` (HeaderAuth scheme)
- Create: `crates/credential/src/credentials/header_auth.rs`
- Source: `protocols/header_auth.rs`

---

### Task 3.4: Create LdapCredential (from v1)

**Files:**
- Create: `crates/credential/src/scheme/ldap.rs` (LdapAuth scheme)
- Create: `crates/credential/src/credentials/ldap.rs`
- Source: `protocols/ldap/`

Preserve: port 389 default, TLS mode config, bind DN/password extraction.

---

## Phase 4: Rotation Migration

> Move rotation framework to v2 module structure. Preserve ALL logic.

### Task 4.1: Move rotation modules to v2 structure

**Files:**
- Keep: `crates/credential/src/rotation/` directory
- Modify: `crates/credential/src/rotation/mod.rs` — update imports to use v2 types

The rotation framework is largely self-contained. Migration steps:
1. Update imports from v1 traits to v2 types (CredentialState → v2 CredentialState)
2. Update `RotationError` to use v2 error model
3. Wire `CredentialRotationEvent` to v2 EventBus pattern
4. Keep all policy, scheduler, blue-green, grace period, transaction, backup logic untouched

This is mostly an import update, not a rewrite.

---

### Task 4.2: Wire rotation events to v2 resolver

**Files:**
- Create: `crates/credential/src/events.rs`
- Modify: `crates/credential/src/resolver.rs`

Emit `CredentialRotatedEvent` on successful refresh. ResourceManager subscribes to trigger pool eviction.

---

## Phase 5: Missing AuthScheme Types

> Add the remaining 6 scheme types from HLD v6.

### Task 5.1: HmacSecret + CertificateAuth

**Files:**
- Create: `crates/credential/src/scheme/hmac.rs`
- Create: `crates/credential/src/scheme/certificate.rs`

---

### Task 5.2: SshAuth + AwsAuth

**Files:**
- Create: `crates/credential/src/scheme/ssh.rs`
- Create: `crates/credential/src/scheme/aws.rs`

---

### Task 5.3: SamlAuth + KerberosAuth

**Files:**
- Create: `crates/credential/src/scheme/saml.rs`
- Create: `crates/credential/src/scheme/kerberos.rs`

---

### Task 5.4: Scheme coercion (TryFrom impls)

- `OAuth2Token → BearerToken` (From)
- `ApiKeyAuth → BearerToken` (TryFrom, bearer placement only)
- `SamlAuth → BearerToken` (TryFrom, if assertion_b64 present)

---

## Phase 6: PendingStateStore + Framework Executors

> Enable interactive credential flows (OAuth2, SAML, device code).

### Task 6.1: PendingToken + PendingStateStore trait

**Files:**
- Create: `crates/credential/src/pending_token.rs`
- Create: `crates/credential/src/pending_store.rs`

32-byte CSPRNG token + trait with 4-dimensional binding (kind, owner, session, token).

---

### Task 6.2: InMemoryPendingStore

**Files:**
- Create: `crates/credential/src/pending_store_memory.rs`

Test-only impl with TTL enforcement, single-use consume, 4D validation.

---

### Task 6.3: Framework resolve/continue executors

**Files:**
- Create: `crates/credential/src/executor.rs`

`execute_resolve()` and `execute_continue()` per HLD v6. 30s timeout on all credential methods. Framework handles PendingState lifecycle.

---

## Phase 7: Storage Layer Additions

### Task 7.1: ScopeLayer (multi-tenant isolation)

### Task 7.2: AuditLayer (redacted metadata)

### Task 7.3: EncryptionLayer — add AAD binding (credential_id as AAD)

---

## Phase 8: Cleanup — Remove v1 Interfaces

> Only AFTER all logic is migrated. This is the final step.

### Task 8.1: Remove v1 trait definitions

**Files:**
- Delete: `crates/credential/src/traits/` — all v1 traits replaced by v2 `Credential` + `CredentialStore`

### Task 8.2: Remove v1 protocol modules

**Files:**
- Delete: `crates/credential/src/protocols/` — logic migrated to `credentials/` in Phase 3

### Task 8.3: Remove v1 provider modules

**Files:**
- Delete: `crates/credential/src/providers/` — logic migrated to `store_*.rs` in Phase 2

### Task 8.4: Remove v1 manager module

**Files:**
- Delete: `crates/credential/src/manager/` — logic absorbed into v2 resolver + layers

### Task 8.5: Clean up lib.rs and re-exports

Remove all v1 re-exports, prelude v1 types. Final `lib.rs` exports only v2.

### Task 8.6: Clean up core/mod.rs

Remove v1-only core types: `CredentialState` (v1), `CredentialFilter`, `CredentialStatus`, `CredentialRef`, `CredentialProvider`, `ErasedCredentialRef`, `CreateResult`, `InitializeResult`, `CredentialSnapshot`, adapter.

### Task 8.7: Fix downstream crates

Update `crates/action/`, `crates/sdk/`, `crates/sdk/macros/` to use v2 types only.

---

## Phase 9: DX Improvements

### Task 9.1: CredentialKey newtype + credential_key! macro

### Task 9.2: #[derive(Credential)] macro for static credentials

### Task 9.3: StaticProtocol v2 trait for reusable protocol patterns

---

## Dependency Graph

```
Phase 0 (safety fixes) ─── can start immediately
    │
    ├── Phase 1 (core traits) ─── depends on Phase 0
    │       │
    │       ├── Phase 2 (storage migration) ─── depends on Phase 1
    │       │
    │       ├── Phase 3 (protocol migration) ─── depends on Phase 1
    │       │
    │       ├── Phase 4 (rotation migration) ─── depends on Phase 1
    │       │
    │       ├── Phase 5 (AuthScheme types) ─── depends on Phase 1.1
    │       │
    │       ├── Phase 6 (PendingStateStore) ─── depends on Phase 1.4
    │       │
    │       └── Phase 7 (storage layers) ─── depends on Phase 1
    │
    └── Phase 8 (v1 cleanup) ─── depends on ALL of Phases 2-7
            │
            └── Phase 9 (DX) ─── depends on Phase 8
```

Phases 2-7 can be parallelized after Phase 1. Phase 8 (cleanup) is the LAST step — only after all logic is migrated.

---

## Key Principle

**Never delete code that has no v2 replacement.** Every line of v1 logic must either:
1. Be migrated to a v2 interface (new trait impl, new module)
2. Be explicitly marked as deprecated with a v2 replacement timeline
3. Stay as-is if it's shared infrastructure (crypto, errors, context)

The v1 *interfaces* (traits, re-exports) get removed in Phase 8. The v1 *logic* lives on inside v2 modules.

---

## Verification Commands

```bash
# Per-crate iteration
rtk cargo check -p nebula-credential && rtk cargo nextest run -p nebula-credential

# Full workspace
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace

# Pre-PR
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace && rtk cargo test --workspace --doc && rtk cargo deny check
```

## Context File Updates

After each phase, update `.claude/crates/credential.md`:
- Phase 0: Note safety fixes
- Phase 1: Update trait signatures, note AuthScheme KIND
- Phase 2: List migrated storage backends
- Phase 3: List migrated protocol impls
- Phase 4: Note rotation framework preserved
- Phase 5: List new AuthScheme types
- Phase 6: Add PendingStateStore to key types
- Phase 7: List new storage layers
- Phase 8: Remove all v1 references
