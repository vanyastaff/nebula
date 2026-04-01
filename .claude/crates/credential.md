# nebula-credential
Credential storage, rotation, v2 trait-based system. Flat module structure.

## Invariants
- Credentials **always encrypted at rest** (AES-256-GCM). `SecretString` zeroizes on drop.
- No direct import between nebula-credential and nebula-resource — use EventBus.
- All `AuthScheme` `Debug` impls redact secrets.
- Error hierarchy: `CredentialError` (author-facing) + `StoreError` (storage) + `SnapshotError` (projection). No `StorageError`, no `ManagerError`.
- `CredentialSnapshot` carries `Box<dyn Any + Send + Sync>` (projected `AuthScheme`), not `serde_json::Value`. Fields are private — use getters + `project::<S>()`/`into_project::<S>()`.
- `CredentialContext` has optional `Arc<dyn CredentialResolverRef>` for credential composition. Use `ctx.resolve_credential::<S>(id)` inside `resolve()`/`refresh()` to depend on another credential.

## Key Decisions
- **Flat structure**: no `core/` directory. All modules at src/ root. Subfolders only for: `scheme/`, `credentials/`, `layer/`, `rotation/`. `utils/` eliminated — `crypto.rs`, `retry.rs` at root. `serde_base64` inline in `crypto.rs`.
- **`SecretString` moved to nebula-core**: `SecretString` + `serde_secret` live in nebula-core (used by log, auth, config, webhook — not just credential). `nebula-credential` re-exports from core.
- **File naming**: `credential.rs` (trait), `state.rs`, `handle.rs`, `key.rs`, `store.rs` (trait), `registry.rs`.
- **Cloud store backends removed**: `store_aws`, `store_vault`, `store_k8s`, `store_postgres`, `store_local` deleted. Will be separate crates. Only `store_memory.rs` (test) + `store.rs` (trait) remain.
- **nebula-resilience integrated**: `RefreshCoordinator` uses per-credential `CircuitBreaker` from nebula-resilience (5 failures, 300s reset). On success CB removed (full reset).
- **derive(Classify)**: `CryptoError`, `ValidationError` use `#[derive(nebula_error::Classify)]`. `CredentialError` keeps manual impl (delegates to inner types).
- `RefreshPolicy.jitter` wired: random jitter applied to early_refresh window to prevent thundering herd.
- `PendingToken` merged into `pending.rs` (was separate file).
- `serde_secret` module lives in nebula-core: transparent SecretString serde for encrypted-at-rest fields. Use `nebula_core::serde_secret` in `#[serde(with = ...)]`.
- `OAuth2State.auth_style` preserves auth style from initial exchange for correct refresh.
- `DevicePollStatus` enum: typed result for device code polling (Ready/Pending/SlowDown/Expired).

## Traps
- Circular dep: peer with nebula-resource, signal via EventBus only.
- v1 prelude deleted — crates using `nebula_credential::prelude::*` must switch to explicit v2 imports.
- `#[derive(Credential)]` requires `identity_state!` invocation for the scheme type separately. Built-in schemes already have it; only custom schemes need it.
- Rotation module is feature-gated behind `rotation` feature flag. Disconnected from v2 Credential trait — uses separate `TestableCredential`/`RotatableCredential` traits. Future v2 integration planned.
- `CredentialSnapshot` is NOT `Serialize`/`Deserialize` — intentionally transient. It IS `Clone` via stored clone function pointer.
- `CredentialSnapshot::into_project::<S>()` consumes self — if type mismatch, snapshot is lost. Use `project::<S>()` (borrow) or check `is::<S>()` first.
- `CredentialHandle::Clone` creates independent `ArcSwap` — cloned handles do NOT see rotation updates. Share via `Arc<CredentialHandle<S>>`.
- `ScopeLayer.list()` filters by owner via N+1 inner `get()` calls — acceptable for in-memory backend, may need backend-level query for large-scale stores.
- `ScopeLayer.exists()` delegates to scoped `get()` — inherits fail-closed behavior from `verify_owner`.
- `verify_owner` is fail-closed: credentials without `owner_id` in metadata are admin-only (fixed B6).
- `CredentialRotationEvent` carries only `credential_id` + `generation` — never credential state (fixed B9).
- Plugin authors need 3 crate deps (`nebula-credential`, `nebula-parameter`, `nebula-core`) — re-export gap.

## Pre-existing Bugs (from HLD v1.2 review + town hall)
- **B6 CRITICAL**: ~~`verify_owner` fails open for ownerless credentials~~ FIXED
- **B10 CRITICAL**: `InMemoryStore` CAS on missing row creates instead of NotFound
- **B1 HIGH**: `SecretString::Serialize` redacts → round-trip destroys identity-state credentials
- **B2 HIGH**: ~~`OAuth2State` stores `access_token`/`refresh_token` as plain `String` (no zeroize)~~ FIXED
- **B5 HIGH**: ~~`ScopeLayer.list()`/`exists()` pass through without scope filtering~~ FIXED
- **B7 HIGH**: ~~`perform_refresh` doesn't retry CAS on `VersionConflict` — new token lost~~ FIXED
- **B8 HIGH**: ~~`complete()` not called if `perform_refresh` panics — in-flight map poisoned~~ FIXED
- **B9 HIGH**: ~~`CredentialRotationEvent.new_state` leaks credential material~~ FIXED
- **B11 HIGH**: ~~Resolver emits NO events after refresh — resources never learn~~ FIXED
- **B12 HIGH**: ~~`StoredCredential` missing `credential_key` — engine can't dispatch~~ FIXED
- **B3 MEDIUM**: RefreshCoordinator circuit breaker map unbounded
- **B13 MEDIUM**: `CredentialRegistry::project()` returns `Box<dyn Any>` not `CredentialSnapshot`
- **B14 HIGH**: ~~No global refresh concurrency limiter — cascading CB opens at 500+ credentials~~ FIXED
- **B15 HIGH**: `DatabaseAuth` missing `expires_at()` — can't auto-refresh IAM tokens
- **B16 HIGH**: `ActionDependencies::credential()` singular — can't declare 2+ (jump hosts)
- **B17 MEDIUM**: `SshAuthMethod` missing `Certificate` variant

## Architecture (HLD v1.5)
- Full HLD: `docs/plans/nebula-credential-hld-v1.md`
- v1 ship list: 33 items (see HLD Section 0)
- Conf1: SecretStore, ResolveOutcome::Stale, keyed semaphore, refresh_policy_override, CertificateAuth, cache feature
- Conf2: DecryptedCacheLayer, DatabaseAuth.extensions, registry introspection, MockCredentialAccessor, "resolve once snapshot many" docs
- v1.1 deferred: CredentialStore dyn-compatibility, put_batch()
- SOC2 audit: CC6.1 CONDITIONAL (B5+B6), CC6.3 PASS, CC7.2 CONDITIONAL, CC8.1 NOT ASSESSED
- Target: 3-crate family (`nebula-credential`, `nebula-credential-storage`, `nebula-credential-macros`)
- Target types not yet implemented: `CredentialPhase` (7 states), `OwnerId`, `StackBuilder`, 5 new error variants
- Registry target: key on `Credential::KEY` (current: `state_kind`), return `Result` on duplicate

## Relations
- Depends on: nebula-core, nebula-eventbus, nebula-resilience, nebula-parameter, nebula-log. Peer: nebula-resource.
- Built-in credentials: `ApiKeyCredential`, `BasicAuthCredential`, `DatabaseCredential`, `HeaderAuthCredential`, `OAuth2Credential`.
- Rotation module: feature-gated, disconnected from v2 Credential trait.
- `CredentialEvent` lives in nebula-core (both emitter/consumer without peer import). Resolver emits via optional `EventBus<CredentialEvent>`.

<!-- updated: 2026-04-01 — HLD v1.5, 17 bugs, 33 v1 items, 2 conferences (10 external devs), SOC2 audit grades -->
<!-- reviewed: 2026-03-31 — B7 fix: CAS retry loop (3 attempts) reuses refreshed token, prevents OAuth2 single-use token loss -->
<!-- reviewed: 2026-03-31 — B8 fix: in_flight → parking_lot::Mutex, scopeguard calls complete()+notify_waiters() -->
<!-- reviewed: 2026-03-31 — B12 fix: added credential_key field to StoredCredential for type dispatch -->
<!-- reviewed: 2026-03-31 — B5 fix: ScopeLayer.list() filters by owner, exists() delegates to scoped get() -->
<!-- reviewed: 2026-03-31 — B14 fix: global refresh concurrency limiter via tokio::sync::Semaphore (default 32) -->
<!-- reviewed: 2026-03-31 — B11 fix: CredentialEvent in nebula-core, resolver emits Refreshed after successful CAS write -->
<!-- reviewed: 2026-03-31 -- B2 fix: OAuth2State access_token/refresh_token/client_id to SecretString, manual Debug redacts -->
