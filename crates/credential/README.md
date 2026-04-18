---
name: nebula-credential
role: Credential Contract (stored state vs projected auth material; rotation, refresh)
status: frontier
last-reviewed: 2026-04-17
canon-invariants: [L2-12.5, L2-13.2]
related: [nebula-core, nebula-schema, nebula-resource, nebula-action, nebula-plugin]
---

# nebula-credential

## Purpose

In most workflow engines, credentials are blobs of JSON passed directly into node code — the author handles rotation, secret exposure, and multi-step flows ad hoc. `nebula-credential` replaces that pattern with a typed **Credential Contract**: the engine owns the split between **stored state** (what is persisted, possibly encrypted) and **projected auth material** (what action code receives), plus the rotation and refresh lifecycles. Action authors bind to a `Credential` type; they never hand-roll token refresh, never hold plaintext secrets longer than necessary, and never see secrets in logs.

## Role

**Credential Contract.** Stored-state vs consumer-facing auth-material split, engine-owned rotation and refresh, composable layered storage. Each `Credential` type declares three associated types: `Scheme` (the auth protocol), `State` (what is persisted), and `Pending` (interactive flow state, e.g. OAuth2 PKCE). The engine resolves them; action code receives only the projected material.

Pattern: *Typed credential lifecycle* (Release It! ch "Stability Patterns" — secrets must not leak; rotations must not strand in-flight executions). Implementation follows the canonical separation between domain representation (`CredentialRecord`) and persisted row (`nebula_storage::rows::CredentialRow`).

## Public API

- `Credential` — unified trait: `resolve()`, `refresh()`, `test()`, `project()`.
- `CredentialMetadata`, `CredentialMetadataBuilder` — static type descriptor: key, name, schema (`ValidSchema`), `AuthPattern`.
- `CredentialRecord` — runtime operational state (created_at, version, expiry, tags); non-sensitive domain representation. Previously named `Metadata` (ADR 0004).
- `AuthScheme`, `AuthPattern` — from `nebula-core`; the open scheme trait and classification enum.
- 12 built-in scheme types: `SecretToken`, `IdentityPassword`, `OAuth2Token`, `KeyPair`, `Certificate`, `SigningKey`, `FederatedAssertion`, `ChallengeSecret`, `OtpSeed`, `ConnectionUri`, `InstanceBinding`, `SharedKey`.
- `CredentialStore`, `StoredCredential`, `PutMode`, `StoreError` — storage trait with layered composition.
- `EncryptionLayer`, `CacheLayer`, `AuditLayer`, `ScopeLayer` — composable store decorators.
- `CredentialRegistry`, `RegistryError` — type-erased runtime dispatch.
- `CredentialResolver`, `ResolveError` — resolution engine: resolve, refresh, test via registry.
- `RefreshCoordinator`, `RefreshAttempt` — thundering-herd prevention for concurrent refresh.
- `SecretString` — string type with automatic zeroization on drop.
- `CredentialGuard` — secure RAII wrapper with `Deref` + zeroize on drop.
- `NoPendingState`, `PendingState`, `PendingToken` — pending state for interactive flows.
- `EncryptedData`, `EncryptionKey`, `encrypt`, `decrypt` — AES-256-GCM crypto primitives.
- `#[derive(Credential)]`, `#[derive(AuthScheme)]` — proc-macro derivations (low boilerplate).
- `CredentialRotationEvent`, `RotationError` (feature `rotation`) — rotation event and error types.
- `OAuth2Credential`, `ApiKeyCredential`, `BasicAuthCredential` — built-in credential implementations.
- `StaticProtocol` — reusable pattern for static credentials (State = Scheme).

## Contract

- **[L2-§12.5]** Encryption at rest uses authenticated encryption (AES-256-GCM). No bypass for debugging. `SecretString` and `Zeroizing<Vec<u8>>` on all intermediate plaintext buffers. `Debug` impls on credential wrappers redact secret fields. Seam: `crates/credential/src/crypto.rs`. Test: `crates/credential/src/crypto.rs` unit tests.
- **[L2-§13.2]** Credential refresh and rotation must not silently strand or corrupt in-flight executions that hold valid material. Failure is explicit in status or errors if the system cannot reconcile. Seam: `crates/credential/src/resolver.rs` — `CredentialResolver::refresh`.
- **[L1-§3.5]** Engine owns the stored-state vs consumer-facing auth-material split. Action authors never hand-roll refresh or pending OAuth steps. Seam: `Credential::project()`.
- **Rename note** — `CredentialRecord` was `Metadata` and `CredentialMetadata` was `Description` before ADR 0004 (commit `51baa36f`). All references to the old names are stale.

## Non-goals

- Not a secret manager (Vault, AWS Secrets Manager) — this is the domain contract layer, not a storage backend.
- Not responsible for secret storage backends — composable layers (`EncryptionLayer`, etc.) wrap any `CredentialStore`.
- Not an OAuth2 server — PKCE and device-code flows are client-side helpers; the OAuth2 authorization endpoint is external.
- Not the schema system — field definitions use `nebula-schema`; `CredentialMetadata.properties` is a `ValidSchema`.

## Maturity

See `docs/MATURITY.md` row for `nebula-credential`.

- API stability: `frontier` — core trait, 12 scheme types, layered store, and resolver are implemented. Rotation feature (`rotation`) is feature-gated and still evolving.
- `#![forbid(unsafe_code)]` enforced.
- Known gap: `CredentialRecord` placement is tracked for potential movement (see comment in `src/record.rs`); no canon revision required.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5 (integration model — stored-state vs projected auth-material split), §12.5 (secrets + auth invariants), §13.2 (rotation/refresh seam).
- Integration model: `docs/INTEGRATION_MODEL.md` §`nebula-credential`.
- ADR: `docs/adr/0004-rename-credential-metadata-description.md` (Metadata→Record, Description→Metadata).
- Siblings: `nebula-core` (`AuthScheme`, `AuthPattern`, `SecretString`, `CredentialId`), `nebula-schema` (`ValidSchema` for `CredentialMetadata.properties`), `nebula-action` (binds to credential types in `ActionDependencies`).

## Appendix

### Authenticated encryption details (evicted from PRODUCT_CANON.md §12.5)

Credentials at rest are encrypted with **AES-256-GCM** using **Argon2id** as the key derivation function. The credential ID is bound as additional authenticated data (AAD), ensuring ciphertext is tied to the specific credential record — no legacy fallback without AAD. Key rotation is supported via multi-key storage with lazy re-encryption on read.

Specific algorithm/KDF/parameters: see `src/crypto.rs` for the authoritative implementation. These choices are L4 implementation detail — changing the algorithm or parameters requires updating this README and `src/crypto.rs`; no canon revision needed. The L2 invariant ("encryption at rest uses authenticated encryption; do not bypass for debugging") lives in canon §12.5.
