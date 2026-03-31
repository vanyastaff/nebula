# nebula-credential
Credential storage, rotation, v2 trait-based system. Flat module structure.

## Invariants
- Credentials **always encrypted at rest** (AES-256-GCM). `SecretString` zeroizes on drop.
- No direct import between nebula-credential and nebula-resource — use EventBus.
- All `AuthScheme` `Debug` impls redact secrets.
- Error hierarchy: `CredentialError` (author-facing) + `StoreError` (storage). No `StorageError`, no `ManagerError`.

## Key Decisions
- **Flat structure**: no `core/` directory. All modules at src/ root. Subfolders only for: `scheme/`, `credentials/`, `layer/`, `rotation/`, `utils/`.
- **File naming**: `credential.rs` (trait), `state.rs`, `handle.rs`, `key.rs`, `store.rs` (trait), `registry.rs`.
- **Cloud store backends removed**: `store_aws`, `store_vault`, `store_k8s`, `store_postgres`, `store_local` deleted. Will be separate crates. Only `store_memory.rs` (test) + `store.rs` (trait) remain.
- **nebula-resilience integrated**: `RefreshCoordinator` uses per-credential `CircuitBreaker` from nebula-resilience (5 failures, 300s reset). On success CB removed (full reset).
- **derive(Classify)**: `CryptoError`, `ValidationError` use `#[derive(nebula_error::Classify)]`. `CredentialError` keeps manual impl (delegates to inner types).
- `RefreshPolicy.jitter` wired: random jitter applied to early_refresh window to prevent thundering herd.
- `PendingToken` merged into `pending.rs` (was separate file).
- `serde_secret` module (inline in `utils/mod.rs`): transparent SecretString serde for encrypted-at-rest fields.
- `OAuth2State.auth_style` preserves auth style from initial exchange for correct refresh.
- `DevicePollStatus` enum: typed result for device code polling (Ready/Pending/SlowDown/Expired).

## Traps
- Circular dep: peer with nebula-resource, signal via EventBus only.
- v1 prelude deleted — crates using `nebula_credential::prelude::*` must switch to explicit v2 imports.
- `#[derive(Credential)]` requires `identity_state!` invocation for the scheme type separately.
- Rotation module is disconnected from v2 Credential trait (separate trait hierarchy, needs redesign).

## Relations
- Depends on: nebula-core, nebula-eventbus, nebula-resilience. Peer: nebula-resource.
- Built-in credentials: `ApiKeyCredential`, `BasicAuthCredential`, `DatabaseCredential`, `HeaderAuthCredential`, `OAuth2Credential`.
- Rotation module: policy, transaction, blue-green, validation, retry, backup, events, metrics.

<!-- updated: 2026-03-31 — beta refactor: flat structure, resilience integration, error consolidation, store backends removed -->
