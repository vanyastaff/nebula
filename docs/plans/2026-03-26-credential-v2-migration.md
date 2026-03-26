# Credential v2 Migration Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Rewrite nebula-credential from v1 (trait soup + flat storage) to v2 (unified Credential trait, AuthScheme contract, layered storage, RefreshCoordinator) per HLD v6.

**Architecture:** Big-bang rewrite. v1 code (~19K LOC) is mature but architecturally incompatible with v2. Crypto utils and storage backends are reused; traits, manager, and protocols are replaced. AuthScheme moves to nebula-core as a contract type.

**Tech Stack:** Rust 1.93, RPITIT, tokio, AES-256-GCM, moka, secrecy/zeroize

**Source of truth:** `crates/credential/plans/credential-hld-v6-final.md`

---

## What to keep from v1

| Component | Location | Action |
|-----------|----------|--------|
| AES-256-GCM encrypt/decrypt | `utils/crypto.rs` | Keep as-is |
| SecretString + zeroize | `utils/secret_string.rs` | Keep as-is |
| Argon2 key derivation | `utils/crypto.rs` | Keep as-is |
| PKCE helpers | `utils/crypto.rs` | Keep as-is |
| LocalStorageProvider | `providers/local.rs` | Adapt to new `CredentialStore` trait |
| AwsSecretsProvider | `providers/aws.rs` | Adapt to new `CredentialStore` trait |
| VaultProvider | `providers/vault.rs` | Adapt to new `CredentialStore` trait |
| K8sSecretsProvider | `providers/kubernetes.rs` | Adapt to new `CredentialStore` trait |
| PostgresProvider | `providers/postgres.rs` | Adapt to new `CredentialStore` trait |
| MockProvider | `providers/mock.rs` | Adapt to new `CredentialStore` trait |
| RotationPolicy types | `rotation/policy.rs` | Keep, integrate with RefreshPolicy |
| BlueGreenRotation | `rotation/blue_green.rs` | Keep, adapt interface |
| GracePeriodTracker | `rotation/grace_period.rs` | Keep |
| EventBus integration | `manager/manager.rs` | Reuse pattern |

## What to delete from v1

| Component | Location | Replaced by |
|-----------|----------|-------------|
| CredentialType trait | `traits/credential.rs` | `Credential` trait (unified) |
| StaticProtocol trait | `traits/protocol.rs` | `Credential::resolve()` |
| FlowProtocol trait | `traits/protocol.rs` | `Credential::resolve()` + `continue_resolve()` |
| InteractiveCredential | `traits/interactive.rs` | `Credential::continue_resolve()` |
| Refreshable trait | `traits/refreshable.rs` | `Credential::refresh()` |
| Revocable trait | `traits/revocable.rs` | `Credential::revoke()` |
| CredentialResource trait | `traits/resource.rs` | `Resource::Auth` in nebula-resource |
| CredentialManager | `manager/manager.rs` | `CredentialResolver` + layered storage |
| CredentialRef / ErasedRef | `core/reference.rs` | `CredentialHandle<S>` |
| CacheLayer (custom) | `manager/cache.rs` | `CacheLayer` in storage stack |
| ProtocolRegistry | `manager/registry.rs` | `CredentialRegistry` |
| CredentialProvider trait | `core/reference.rs` | `CredentialResolver` |
| ApiKeyProtocol | `protocols/api_key.rs` | Static credential impl |
| BasicAuthProtocol | `protocols/basic_auth.rs` | Static credential impl |
| DatabaseProtocol | `protocols/database.rs` | Static credential impl |

---

## Phase 1: Core Types + AuthScheme (nebula-core + nebula-credential)

**Estimated effort:** 4-6 hours
**Dependencies:** None
**Breaking changes:** `Resource::Credential` → `Resource::Auth` in nebula-resource

### Task 1.1: Add AuthScheme trait to nebula-core

**Files:** `crates/core/src/auth.rs` (create), `crates/core/src/lib.rs` (modify)

AuthScheme is a marker trait for consumer-facing auth material. Lives in core because both credential and resource depend on it.

```rust
/// Marker trait for consumer-facing authentication material.
///
/// Resources declare `type Auth: AuthScheme` to specify what auth
/// material they need (e.g., `BearerToken`, `DatabaseAuth`).
/// Credential types produce auth material via `Credential::project()`.
pub trait AuthScheme: Send + Sync + Clone + 'static {}

/// No authentication required.
impl AuthScheme for () {}
```

### Task 1.2: Built-in AuthScheme types in nebula-credential

**Files:** `crates/credential/src/scheme/` (create directory + files)

Types from HLD: `BearerToken`, `BasicAuth`, `DatabaseAuth`, `ApiKeyAuth`, `HeaderAuth`, `CertificateAuth`, `SshAuth`, `OAuth2Token`, `AwsAuth`, `LdapAuth`, `SamlAuth`, `KerberosAuth`, `HmacSecret`.

All must:
- Implement `AuthScheme`
- Use `SecretString` for secret fields
- Implement `Debug` with redaction (`[REDACTED]`)
- Implement `Clone`, `Serialize`, `Deserialize`

Start with the 5 most common: `BearerToken`, `BasicAuth`, `DatabaseAuth`, `ApiKeyAuth`, `OAuth2Token`.

### Task 1.3: CredentialState trait + identity_state! macro

**Files:** `crates/credential/src/state.rs` (rewrite)

```rust
pub trait CredentialState: Serialize + DeserializeOwned + Send + Sync + 'static {
    const KIND: &'static str;
    const VERSION: u32;
    fn expires_at(&self) -> Option<DateTime<Utc>> { None }
}

/// Opt-in: make an AuthScheme also usable as CredentialState.
macro_rules! identity_state {
    ($ty:ty, $kind:expr, $version:expr) => {
        impl CredentialState for $ty {
            const KIND: &'static str = $kind;
            const VERSION: u32 = $version;
        }
    };
}
```

### Task 1.4: PendingState trait + NoPendingState

**Files:** `crates/credential/src/pending.rs` (create)

### Task 1.5: New unified Credential trait

**Files:** `crates/credential/src/credential.rs` (create)

The big one — single trait with 3 associated types, 5 consts, 7 methods (most with defaults). Copy from HLD v6 lines 337-442.

### Task 1.6: Supporting enums

**Files:** `crates/credential/src/resolve.rs` (create)

- `ResolveResult<S, P>`, `StaticResolveResult<S>`
- `InteractionRequest`, `DisplayData`
- `UserInput`
- `RefreshOutcome`
- `TestResult`
- `RefreshPolicy`

### Task 1.7: CredentialError (new structured error)

**Files:** `crates/credential/src/error.rs` (rewrite)

New error with `RetryAdvice`, `RefreshErrorKind`, `ResolutionStage`. Keep `CryptoError` and `StorageError` from v1.

### Task 1.8: CredentialDescription, CredentialContext

**Files:** `crates/credential/src/description.rs` (adapt from v1), `crates/credential/src/context.rs` (rewrite)

### Task 1.9: Rename Resource::Credential → Resource::Auth

**Files:** `crates/resource/src/resource.rs`, `crates/resource/src/manager.rs`, all tests

Breaking change: `type Credential: Credential` → `type Auth: AuthScheme`. Update all impls across workspace. Delete placeholder `Credential` trait from resource crate.

### Task 1.10: Update lib.rs, tests, verify

Rewire all re-exports, run workspace tests.

---

## Phase 2: Storage + Layers

**Estimated effort:** 4-6 hours
**Dependencies:** Phase 1

### Task 2.1: New CredentialStore trait

**Files:** `crates/credential/src/store.rs` (create)

5 methods (get, put, delete, list, exists), `PutMode` enum (CreateOnly/Overwrite/CompareAndSwap), `StoredCredential` wrapper.

### Task 2.2: StoreLayer trait + EncryptionLayer

**Files:** `crates/credential/src/layer/` (create directory)

Reuse v1 crypto. Encryption wraps serialized bytes → EncryptedData before passing to next layer.

### Task 2.3: CacheLayer (moka, ciphertext-only)

Reuse v1 moka integration. Key invariant: cache stores ciphertext, not plaintext.

### Task 2.4: InMemoryStore + InMemoryPendingStore

Test-only implementations.

### Task 2.5: Adapt LocalFileStore from v1

Adapt v1 `LocalStorageProvider` to new `CredentialStore` trait. Mostly interface change.

### Task 2.6: Layer composition

```
ScopeLayer → AuditLayer → EncryptionLayer → CacheLayer → Backend
```

Wire layers in correct order. ScopeLayer and AuditLayer are stubs (Phase 5).

---

## Phase 3: Protocols + Derive Macros

**Estimated effort:** 6-8 hours
**Dependencies:** Phase 1

### Task 3.1: Migrate ApiKey → static Credential impl

Convert v1 `ApiKeyProtocol` (StaticProtocol) → v2 `ApiKeyCredential` (impl Credential).

### Task 3.2: Migrate BasicAuth, Database, HeaderAuth

Same pattern as 3.1. These are the simplest credentials.

### Task 3.3: OAuth2Flow builder + OAuth2State + OAuth2Pending

Adapt v1 OAuth2 code to v2 Credential trait with `resolve()` / `continue_resolve()` / `refresh()` / `revoke()`.

### Task 3.4: #[derive(Credential)] macro

In `nebula-credential-macros` (or expand `nebula-resource-macros`). Generates defaults for static credentials.

### Task 3.5: #[derive(FromParameters)] macro

Type-safe parameter extraction from `ParameterValues`.

---

## Phase 4: Resolver + Refresh + Composition

**Estimated effort:** 8-10 hours
**Dependencies:** Phase 2, Phase 3

### Task 4.1: CredentialResolver

Runtime resolution: load State → project to Scheme → cache → return.

### Task 4.2: RefreshCoordinator

CAS-based refresh with DashMap, Notify, scopeguard. Prevents thundering herd on refresh.

### Task 4.3: CredentialHandle<S> with ArcSwap + snapshot()

Typed handle returned to callers. `snapshot()` returns `Arc<S>` (immutable).

### Task 4.4: Framework resolve executor

Manages PendingState lifecycle, timeout wrapping. Pure orchestration — credential code stays pure.

### Task 4.5: CredentialRegistry

Type dispatch: `state_kind` string → handler. debug_assert capability validation.

### Task 4.6: CredentialRotatedEvent + EventBus

Integration point with nebula-resource. Manager subscribes, triggers pool eviction.

### Task 4.7: Integration with nebula-resource

Wire `CredentialResolver` into engine flow:
1. Engine resolves credential → `Arc<AuthScheme>`
2. Passes to `Manager::acquire_*(&auth, &ctx, &opts)`
3. Resource creates runtime with auth material

---

## Phase 5: Scope + Audit + Production Storage

**Estimated effort:** 6-8 hours
**Dependencies:** Phase 4

### Task 5.1: ScopeLayer (multi-tenant isolation)

Outermost layer — fail fast on scope mismatch.

### Task 5.2: AuditLayer (redacted metadata only)

Never receives plaintext. Logs access patterns for compliance.

### Task 5.3: Adapt PostgresStore from v1

### Task 5.4: Adapt VaultStore from v1

### Task 5.5: Adapt AwsSecretsStore from v1

### Task 5.6: Adapt K8sSecretsStore from v1

---

## Phase 6: Testing Infrastructure

**Estimated effort:** 4-6 hours
**Dependencies:** Phase 5

### Task 6.1: MockCredentialStore with error injection

### Task 6.2: MockPendingStore with error injection

### Task 6.3: Pre-built test credentials (Telegram, GitHub)

### Task 6.4: Contract test macro for credential types

### Task 6.5: Debug redaction assertion tests

### Task 6.6: Capability const consistency tests

---

## Migration Sequence

```
Week 1: Phase 1 (core types) + Phase 2 (storage layers)
Week 2: Phase 3 (protocols + macros)
Week 3: Phase 4 (resolver + refresh + integration)
Week 4: Phase 5 (production storage) + Phase 6 (testing)
```

## Risk Mitigation

| Risk | Mitigation |
|------|-----------|
| Breaking Resource::Auth rename | Do atomically in Phase 1, all downstream in same commit |
| Losing v1 storage provider code | Git history preserves everything; copy to new trait before delete |
| OAuth2 flow regression | Keep v1 OAuth2 tests, adapt to new interface |
| Crypto compatibility | Utils module unchanged — same encrypt/decrypt |

## Success Criteria

- All 15 AuthScheme types implemented with redacted Debug
- Credential trait works for static (Telegram) and interactive (OAuth2) cases
- CredentialResolver resolves + auto-refreshes + caches
- nebula-resource uses `Resource::Auth` instead of `Resource::Credential`
- All v1 storage backends adapted to new interface
- Zero plaintext credentials in logs, errors, or Debug output
