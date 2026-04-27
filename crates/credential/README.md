---
name: nebula-credential
role: Credential Contract (stored state vs projected auth material; runtime orchestration lives in nebula-engine)
status: frontier
last-reviewed: 2026-04-23
canon-invariants: [L2-12.5, L2-13.2]
related: [nebula-core, nebula-schema, nebula-resource, nebula-action, nebula-plugin]
---

# nebula-credential

## Purpose

In most workflow engines, credentials are blobs of JSON passed directly into node code — the author handles rotation, secret exposure, and multi-step flows ad hoc. `nebula-credential` replaces that pattern with a typed **Credential Contract**: the engine owns the split between **stored state** (what is persisted, possibly encrypted) and **projected auth material** (what action code receives). Runtime orchestration (resolver/executor/refresh coordination) now lives in `nebula-engine::credential`. Action authors bind to a `Credential` type; they never hand-roll token refresh, never hold plaintext secrets longer than necessary, and never see secrets in logs.

## Role

**Credential Contract.** Stored-state vs consumer-facing auth-material split, pending-state contract, secret-handling primitives, and credential metadata/types. Each `Credential` type declares three associated types: `Scheme` (the auth protocol), `State` (what is persisted), and `Pending` (interactive flow state, e.g. OAuth2 PKCE). The engine resolves them; action code receives only the projected material.

**Integration credentials (Plane B):** this crate models **workflow integration** secrets (calls to Slack, cloud APIs, databases, …), not operator login to Nebula. The canonical boundary and rules for adding new auth mechanisms are documented in [`docs/adr/0033-integration-credentials-plane-b.md`](../../docs/adr/0033-integration-credentials-plane-b.md).

Pattern: *Typed credential lifecycle* (Release It! ch "Stability Patterns" — secrets must not leak; rotations must not strand in-flight executions). Implementation follows the canonical separation between domain representation (`CredentialRecord`) and persisted row (`nebula_storage::rows::CredentialRow`).

### Architecture cleanup status

The [credential architecture cleanup design](../../docs/superpowers/specs/2026-04-20-credential-architecture-cleanup-design.md) phased resolver/registry/executor and rotation **orchestration** into `nebula-engine`, persistence layers into `nebula-storage`, and OAuth **HTTP ceremony** into `nebula-api` — see ADR-0028–0031 and [`ADR-0033`](../../docs/adr/0033-integration-credentials-plane-b.md) (Plane B).

**ADR-0032** keeps the `CredentialStore` **trait** in this crate (avoiding a `credential → storage` dependency cycle). Production in-memory stores should use `nebula_storage::credential::InMemoryStore`; `store_memory` remains as a cycle-safe shim.

**HTTP transport status:** `OAuth2Credential::resolve` (authorization URL construction) is pure — no HTTP. `OAuth2Credential::refresh` returns `CredentialError::Provider("OAuth2 HTTP transport has moved: ...")` per ADR-0031 — refresh HTTP lives in `nebula-engine`, token exchange в `nebula-api`. The crate has **no reqwest dependency**.

## Public API

- `Credential` — base trait: `resolve()`, `project()`, plus the three associated types (`Input`, `State`, `Scheme`) and `KEY` const. Capability methods removed from base — see sub-traits below.
- `Interactive`, `Refreshable`, `Revocable`, `Testable`, `Dynamic` — capability sub-traits added at П1 (Tech Spec §15.4). `Interactive` carries the `Pending` associated type; engine dispatchers bind `where C: <Capability>`.
- `CredentialState` — supertrait `ZeroizeOnDrop` is now mandatory (Tech Spec §15.4 amendment); compile-fail probe `compile_fail_state_zeroize` enforces.
- `CredentialMetadata`, `CredentialMetadataBuilder` — static type descriptor: key, name, schema (`ValidSchema`), `AuthPattern`. **`capabilities_enabled` field removed** (Tech Spec §15.8) — capability sets come from sub-trait membership at registration.
- `Capabilities` (bitflags), `compute_capabilities::<C>() -> Capabilities`, `plugin_capability_report::*` — registration-time capability fold (Tech Spec §15.8).
- `CredentialRegistry`, `RegisterError` — `register<C>(instance, registering_crate) -> Result<(), RegisterError>`; duplicates fatal in debug + release (Tech Spec §15.6). `iter_compatible(required: Capabilities)` for slot-picker / discovery code.
- `AuthScheme` (base) + `SensitiveScheme: AuthScheme + ZeroizeOnDrop` + `PublicScheme: AuthScheme` — the П1 sensitivity dichotomy (Tech Spec §15.5). `AuthPattern` unchanged.
- 9 built-in scheme types: `SecretToken`, `IdentityPassword`, `OAuth2Token`, `KeyPair`, `Certificate`, `SigningKey`, `ConnectionUri`, `InstanceBinding`, `SharedKey`. (Pruned 2026-04-24: `FederatedAssertion` → Plane A per ADR-0033; `ChallengeSecret` + `OtpSeed` → integration-internal, not projected.) Each scheme is now `SensitiveScheme` or `PublicScheme` per §15.5.
- `SchemeGuard<'a, C>`, `SchemeFactory<C>` — refresh-hook surface (Tech Spec §15.7). `SchemeGuard` is `!Clone`, lifetime-pinned, and drop-zeroizes through the wrapped scheme's `ZeroizeOnDrop` impl. The refresh-notification hook itself is `Resource::on_credential_refresh` in `nebula-resource` per ADR-0036; the previously-defined parallel `OnCredentialRefresh<C>` trait was a transitional bridge and was removed in nebula-resource П2.
- `CredentialRecord` — runtime operational state (created_at, version, expiry, tags); non-sensitive domain representation. Previously named `Metadata` (ADR 0004).
- `CredentialStore`, `StoredCredential`, `PutMode`, `StoreError` — storage trait with layered composition.
- `InMemoryStore` — in-crate test/development store shim (canonical impl is `nebula_storage::credential::InMemoryStore`).
- `SecretString` — string type with automatic zeroization on drop.
- `CredentialGuard` — secure RAII wrapper with `Deref` + zeroize on drop; implements `Guard` and `TypedGuard` from `nebula-core`.
- `NoPendingState`, `PendingState`, `PendingToken` — pending state for interactive flows (`Pending` lives on `Interactive` per §15.4).
- `PendingStateStore`, `InMemoryPendingStore`, `PendingStoreError` — pending-state contract and in-memory shim.
- `EncryptedData`, `EncryptionKey`, `encrypt`, `decrypt` — AES-256-GCM crypto primitives.
- `#[derive(Credential)]`, `#[derive(AuthScheme)]` (with `sensitive` / `public` argument) — proc-macro derivations (low boilerplate).
- `#[capability]` (in `nebula-credential-macros`) — capability sub-trait declaration with sealed companion + phantom-shim companion per ADR-0035.
- `CredentialRotationEvent`, `RotationError` (feature `rotation`) — rotation event and error types.
- `OAuth2Credential`, `ApiKeyCredential`, `BasicAuthCredential` — built-in credential implementations.
- `NoCredential` — opt-out for resources without an authenticated binding ([ADR-0036](../../docs/adr/0036-resource-credential-adoption-auth-retirement.md)).
- `StaticProtocol` — reusable pattern for static credentials (State = Scheme).
- `ExternalProvider`, `ExternalReference`, `ProviderKind`, `ProviderError` — external provider abstraction for Vault, AWS Secrets Manager, GCP Secret Manager, Azure Key Vault, and other secret managers.
- `CredentialMetrics` — standardized credential operation metric names and label helpers (`resolve_total`, `refresh_total`, `rotations_total`, etc.).
- `prelude` module — convenient re-exports of common credential types.

## Contract

- **[L2-§12.5]** Encryption at rest uses authenticated encryption (AES-256-GCM). No bypass for debugging. `SecretString` and `Zeroizing<Vec<u8>>` on all intermediate plaintext buffers. `Debug` impls on credential wrappers redact secret fields. Seam: `crates/credential/src/crypto.rs`. Test: `crates/credential/src/crypto.rs` unit tests.
- **[L2-§13.2]** Credential refresh and rotation must not silently strand or corrupt in-flight executions that hold valid material. Failure is explicit in status or errors if the system cannot reconcile. Seam: `crates/engine/src/credential/resolver.rs` — `CredentialResolver::resolve_with_refresh`.
- **[L1-§3.5]** Engine owns the stored-state vs consumer-facing auth-material split. Action authors never hand-roll refresh or pending OAuth steps. Seam: `Credential::project()`.
- **Rename note** — `CredentialRecord` was `Metadata` and `CredentialMetadata` was `Description` before ADR 0004 (commit `51baa36f`). All references to the old names are stale.

## Non-goals

- Not a secret manager (Vault, AWS Secrets Manager) — this is the domain contract layer, not a storage backend.
- Not responsible for secret storage backends — composable layers (`EncryptionLayer`, etc.) wrap any `CredentialStore`.
- Not an OAuth2 server — PKCE and device-code flows are client-side helpers; the OAuth2 authorization endpoint is external.
- Not the schema system — field definitions use `nebula-schema`; `CredentialMetadata.properties` is a `ValidSchema`.

## П1 trait shape (2026-04-26)

The credential П1 phase landed the validated CP5/CP6 trait shape per Tech Spec §15.4-§15.8. Key shifts versus the pre-П1 surface:

- **Capability sub-trait split (§15.4).** The 4 capability bools (`INTERACTIVE` / `REFRESHABLE` / `REVOCABLE` / `TESTABLE`) and the production `DYNAMIC` flag are gone. Credentials opt into capabilities by implementing `Interactive`, `Refreshable`, `Revocable`, `Testable`, or `Dynamic`. The `Pending` associated type lives on `Interactive` (was on the base trait). Engine dispatchers bind `where C: Refreshable` rather than reading a const; the silent-downgrade vector ("const says `true` but method defaults to `NotSupported`") is structurally absent. Closes security-lead N1+N3+N5.
- **`AuthScheme` sensitivity dichotomy (§15.5).** `AuthScheme` is now the base; sensitive material implements `SensitiveScheme: AuthScheme + ZeroizeOnDrop`, public material implements `PublicScheme: AuthScheme`. Derive macros `#[auth_scheme(sensitive)]` / `#[auth_scheme(public)]` audit fields at expansion (forbid plain `String` for sensitive, forbid `SecretString` for public, name-based lint on `token` / `secret` / `key` / `password`). `OAuth2Token::bearer_header` returns `SecretString`; `ConnectionUri` exposes structured accessors only. Closes N2+N4+N10.
- **Fatal duplicate-KEY registration (§15.6).** `CredentialRegistry::register<C>(instance, registering_crate)` returns `Result<(), RegisterError>` — duplicates are fatal in **both** debug and release builds. The previous "panic in debug, warn + overwrite in release" pattern is removed. Operators resolve via plugin uninstall, version pin, or namespace fix at startup rather than discovering silent credential takeover at runtime. Closes N7 (interim until signed-manifest infra lands).
- **`SchemeGuard` + `SchemeFactory` refresh hook (§15.7).** Long-lived resources receive `SchemeGuard<'a, C>` (`!Clone`, drop-zeroizes via `SensitiveScheme: ZeroizeOnDrop`, lifetime-pinned by `PhantomData<&'a ()>`) instead of `&Scheme`. `SchemeFactory<C>` is the re-acquisition mechanism for connection pools / daemons that need fresh material per request. The refresh-notification hook itself lives on `nebula_resource::Resource::on_credential_refresh` per ADR-0036 + Tech Spec §15.4 (the previously-defined parallel `OnCredentialRefresh<C>` trait was a transitional bridge and was removed in nebula-resource П2). Closes N8 + tech-lead gap (i).
- **Capability-from-type (§15.8).** `CredentialMetadata::capabilities_enabled` is removed. Capability sets come from `compute_capabilities::<C>()` over the `plugin_capability_report::Is*` constants (set by sub-trait membership) at registration; plugins cannot self-attest false capabilities. `CredentialRegistry::iter_compatible(required: Capabilities) -> impl Iterator<Item = (&str, Capabilities)>` is the discovery surface for slot pickers. Closes N6.
- **ADR-0035 phantom-shim canonical form.** `dyn ServiceCapability` requires a per-capability `mod sealed_caps` + `dyn ServiceCapabilityPhantom` rewrite — see [ADR-0035](../../docs/adr/0035-phantom-shim-capability-pattern.md) (amendments 2026-04-24-B + -C + 2026-04-26 rename). The `#[capability]` proc-macro and `#[action_phantom]` rewriter make this one-line for plugin authors.

Plugin authors: see [`crates/credential-builtin/`](../credential-builtin/) for canonical capability sub-trait impls and the `mod sealed_caps` convention. The 10 landing-gate compile-fail probes in `tests/compile_fail_*.rs` document every invariant — read those first when a credential change feels load-bearing.

## Maturity

See `docs/MATURITY.md` row for `nebula-credential`.

- API stability: `frontier` — П1 trait scaffolding landed (sub-trait capability split, sensitivity dichotomy, fatal duplicate-KEY registration, `SchemeGuard` / `SchemeFactory` refresh hook, capability-from-type). 9 scheme types, store contract, and secret primitives implemented. Runtime resolver/registry/executor in `nebula-engine::credential`. `CredentialContext` embeds `BaseContext` and implements `Context` trait from `nebula-core`. Former `accessor/` and `metadata/` directories flattened to root-level modules. Rotation feature (`rotation`) is feature-gated and still evolving. nebula-resource П2 wired Manager-side `Resource::on_credential_refresh` fan-out (Tasks 4-5) and removed the deprecated parallel `OnCredentialRefresh<C>` trait (Task 12); engine `iter_compatible` consumer wiring tracked as a post-П1 follow-up (`stage7-followup-engine-discovery` in the credential concerns register).
- `#![forbid(unsafe_code)]` enforced.
- Known gap: `CredentialRecord` placement is tracked for potential movement (see comment in `src/record.rs`); no canon revision required.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5 (integration model — stored-state vs projected auth-material split), §12.5 (secrets + auth invariants), §13.2 (rotation/refresh seam).
- Integration model: `docs/INTEGRATION_MODEL.md` §`nebula-credential`.
- ADR: `docs/adr/0004-rename-credential-metadata-description.md` (Metadata→Record, Description→Metadata).
- Siblings: `nebula-core` (cross-cutting IDs/scopes), `nebula-schema` (`ValidSchema` for `CredentialMetadata.properties`), `nebula-action` (binds to credential types in `ActionDependencies`), `nebula-engine` (`credential` module owns runtime resolution/orchestration), `nebula-storage` (`credential` module owns store impls/layers).

## Appendix

### Authenticated encryption details (evicted from PRODUCT_CANON.md §12.5)

Credentials at rest are encrypted with **AES-256-GCM** using **Argon2id** as the key derivation function. The credential ID is bound as additional authenticated data (AAD), ensuring ciphertext is tied to the specific credential record — no legacy fallback without AAD. Key rotation is supported via multi-key storage with lazy re-encryption on read.

Specific algorithm/KDF/parameters: see `src/crypto.rs` for the authoritative implementation. These choices are L4 implementation detail — changing the algorithm or parameters requires updating this README and `src/crypto.rs`; no canon revision needed. The L2 invariant ("encryption at rest uses authenticated encryption; do not bypass for debugging") lives in canon §12.5.
