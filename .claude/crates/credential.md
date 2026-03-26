# nebula-credential
Credential storage, manager, rotation, protocols. v2 rewrite in progress alongside v1.

## Invariants
- Credentials **always encrypted at rest** (AES-256-GCM). `SecretString` zeroizes on drop.
- No direct import between nebula-credential and nebula-resource — use EventBus.
- All `AuthScheme` `Debug` impls redact secrets.

## Key Decisions
- `CredentialProvider` = DI for actions; never inject `CredentialManager` directly.
- v2 modules coexist with v1 (v1 deleted later). RPITIT, no `#[async_trait]`.
- `PendingState` uses `Zeroize` (not `ZeroizeOnDrop`).
- `EncryptionLayer<S>` serializes `EncryptedData` as JSON bytes in `data` field.
- `CredentialRegistryV2`: type-erased dispatch — `register::<C>()` captures deserialize+project closure keyed by `state_kind`.
- `CredentialResolver` verifies `state_kind` match before deserialize; returns `CredentialHandle<S>` (Arc-wrapped).
- `SecretString` serializes as `"[REDACTED]"` — tests must construct raw JSON for round-trip.

## Traps
- Circular dep: peer with nebula-resource, signal via EventBus only.
- Storage providers feature-gated: `storage-local`, `-aws`, `-postgres`, `-vault`, `-k8s`.
- `CredentialId` is a `nebula_core::CredentialId` re-export.

## Relations
- Depends on: nebula-core, nebula-eventbus. Peer: nebula-resource.

<!-- reviewed: 2026-03-25 -->
<!-- updated: 2026-03-25 — Phase 3: credential impls -->
