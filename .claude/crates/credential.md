# nebula-credential
Credential storage, manager, rotation, protocols. v2 rewrite in progress alongside v1.

## Invariants
- Credentials **always encrypted at rest** (AES-256-GCM). `SecretString` zeroizes on drop.
- No direct import between nebula-credential and nebula-resource — use EventBus.
- All `AuthScheme` `Debug` impls redact secrets.

## Key Decisions
- `CredentialProvider` = DI for actions; never inject `CredentialManager` directly.
- v2 modules coexist with v1 (v1 deleted later):
  - `scheme/` — 5 `AuthScheme` types (BearerToken, BasicAuth, DatabaseAuth, ApiKeyAuth, OAuth2Token)
  - `credential_state` — `CredentialStateV2` (avoids v1 name conflict). `identity_state!` macro.

## Traps
- Circular dep: peer with nebula-resource, signal via EventBus only.
- Storage providers feature-gated: `storage-local`, `-aws`, `-postgres`, `-vault`, `-k8s`.
- `CredentialId` is a `nebula_core::CredentialId` re-export.

## Relations
- Depends on: nebula-core, nebula-eventbus. Peer: nebula-resource.

<!-- reviewed: 2026-03-25 -->
