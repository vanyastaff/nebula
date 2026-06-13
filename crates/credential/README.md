---
name: nebula-credential
role: Credential subsystem — Contract (stored state vs projected auth material) + Runtime (resolve/refresh/lease/rotation-state) + Management facade, consolidated into one crate per ADR-0092
status: stable
last-reviewed: 2026-06-12
canon-invariants: [L2-12.5, L2-13.2]
related: [nebula-core, nebula-schema, nebula-resource, nebula-action, nebula-plugin]
---

# nebula-credential

## Purpose

In most workflow engines, credentials are blobs of JSON passed directly into node code — the author handles rotation, secret exposure, and multi-step flows ad hoc. `nebula-credential` replaces that pattern with a typed **Credential Contract**: the crate owns the split between **stored state** (what is persisted, possibly encrypted) and **projected auth material** (what action code receives). Per **ADR-0092** the runtime (resolver / executor / refresh coordinator / lease / rotation-state machines) and the `CredentialService` management facade were **consolidated into this crate** — they previously squatted in `nebula-engine` / `nebula-credential-runtime` (both now removed). Action authors bind to a credential output scheme via `#[credential]` slot fields; they never hand-roll token refresh, never hold plaintext secrets longer than necessary, and never see secrets in logs.

## Role

**Credential subsystem (one crate, three bounded contexts).** (1) *TypeSystem* — the `Credential` contract, capability sub-traits, schemes, and `CredentialRegistry`. Each `Credential` type declares three associated types: `Scheme` (the auth protocol the consumer receives), `State` (what is persisted), and `Properties` (the typed setup-form fields, replaces the pre-Phase-5 `Input`). (2) *Runtime* (`runtime/`) — `CredentialResolver`, `RefreshCoordinator`, lease, executor, dispatchers, rotation-state machines (relocated from `nebula-engine` per ADR-0092). (3) *Management* (`service/`) — the `CredentialService` facade for tenant CRUD and `resolve_for_slot` bind-population. Heavy I/O (HTTP token exchange, durable stores, crypto) is **injected via ports** — see Layering below; the crate links no `reqwest` and no `sqlx`. Action code receives only the projected material.

**Integration credentials (Plane B):** this crate models **workflow integration** secrets (calls to Slack, cloud APIs, databases, …), not operator login to Nebula. The canonical boundary and rules for adding new auth mechanisms are documented in [`ADR-0033 (integration credentials, Plane B)`](../../docs/adr/HISTORICAL.md).

Pattern: *Typed credential lifecycle* (Release It! ch "Stability Patterns" — secrets must not leak; rotations must not strand in-flight executions). Implementation follows the canonical separation between domain representation (`CredentialRecord`) and persisted state (`StoredCredential` + `EncryptionLayer` in `nebula-storage`). (The dead parallel `CredentialRow` SQL row-model flagged by ADR-0088 is slated for deletion — do not build against it.)

### Layering (post-ADR-0092)

Resolver / registry / executor / refresh-coordinator / lease / rotation-**state**
live **here** (`runtime/`), relocated from `nebula-engine` by ADR-0092. The engine
keeps only the `credential_accessor` / `resource_accessor` **bridges** (it consumes
this crate's facade) plus `default_in_memory_coordinator()` for tests. The
per-slot rotation **fan-out** (dispatch to live resources) moved to
`nebula-resource`. Crypto (AES-256-GCM / Argon2id) lives in **`nebula-crypto`**,
injected as `Arc<dyn Cipher>` / `Arc<dyn Kdf>`. Durable stores + Encryption /
Cache / Audit decorators stay in `nebula-storage`. See
[`ADR-0092`](../../docs/adr/0092-credential-subsystem-consolidation.md) and Plane B
mechanics in [`ADR-0033`](../../docs/adr/HISTORICAL.md).

**ADR-0032** keeps the `CredentialStore` **trait** in this crate (the store *port*
lives in `nebula-storage-port`). All concrete SQLite / Postgres stores live in
`nebula_storage::credential`; this crate ships no store impl and links no `sqlx`.

**HTTP transport status:** the crate mounts **zero HTTP routes** and links **no
`reqwest`**. OAuth2 authorization-URL construction, PKCE, signed-state, and pending
logic are pure (`OAuth2State`, `secrets::crypto`). The bare token-exchange / refresh
HTTP POST is inverted behind the **`RefreshTransport`** port (ADR-0092); the
composition root (`nebula-api`) injects the hardened `reqwest` client
(`ReqwestRefreshTransport::hardened()`), while SSRF host/IP validation, bounded
reads, and `OAuth2State` mutation stay **inside** this crate. The browser-redirect
ceremony (auth/callback routes) lives only in `nebula-api` (`transport/oauth`) — see
the **OAuth Plane Law** in [`docs/DESIGN.md`](docs/DESIGN.md).

## Public API (v4 — M6 / Phase 5, 2026-04-29)

The v4 surface lands per Phase 5 of the M6 dependency redesign. The pre-Phase-5 `type Input` was renamed to `type Properties` to mirror `Action::Input` / `Resource::Config`; the schema lives on the typed companion struct rather than baked into instance metadata.

### `Credential` trait — typed Properties / State / Scheme

```rust
pub trait Credential: Send + Sync + 'static {
    type Properties: HasSchema;     // typed setup-form fields (replaces Input)
    type State:      CredentialState;
    type Scheme:     AuthScheme;

    const KEY: &'static str;

    fn metadata() -> CredentialMetadata;
    // No schema method — `Properties: HasSchema` is the single source of
    // truth; read it via `nebula_schema::schema_of::<Self::Properties>()`
    // (ADR-0052 P3).

    fn project(state: &Self::State) -> Self::Scheme;

    async fn resolve(values: &FieldValues, ctx: &CredentialContext<'_>)
        -> Result<ResolveResult<Self::State, /* Pending = */ ()>, CredentialError>;
}
```

Capability methods (`continue_resolve`, `refresh`, `revoke`, `test`, `release`) live on dedicated **sub-traits** per Tech Spec §15.4 — see "Capability sub-traits" below.

### Properties companion struct — `#[derive(Schema, Deserialize)]`

Authoring pattern: declare the setup form as a separate struct and reference it from the credential via `#[credential(properties = …)]`.

```rust
use nebula_credential::Credential;
use nebula_schema::Schema;
use serde::Deserialize;

#[derive(Schema, Deserialize)]
pub struct SlackBotProperties {
    #[field(secret, label = "Bot token")]
    #[validate(required)]
    pub bot_token: String,

    #[field(label = "Refresh URL")]
    pub refresh_url: Option<String>,
}

#[derive(Credential)]
#[credential(
    key      = "slack_bot",
    name     = "Slack bot",
    scheme   = SecretToken,
    properties = SlackBotProperties,
)]
pub struct SlackBotToken;

// Implementor supplies project + resolve in a separate impl block when
// using the `properties = ...` mode (the derive cannot synthesize them
// without a StaticProtocol).
```

**Two derive modes:**

- `properties = TypePath` — companion struct ownership of the schema. User implements `resolve` (and `project` when scheme ≠ state).
- `protocol = TypePath` — for static credentials backed by a reusable `StaticProtocol`. The derive emits a `resolve` body that delegates to `<protocol as StaticProtocol>::build(values)`. `type Properties` is set to `<protocol as StaticProtocol>::Properties`.

The two attributes are mutually exclusive.

### Credential expressions are NOT allowed in property values

Per canon §12.5 / Phase 9, credential property JSON flows through `<C::Properties as HasSchema>::schema().validate(...)` and then directly into `serde_json::from_value::<C::Properties>(...)` — the credential pipeline deliberately omits the `ValidValues::resolve(&dyn ExpressionContext)` step that the action input pipeline runs. Rationale: secrets must not depend on runtime workflow state. Defense in depth: even if a `{{ … }}` template survives validation as `FieldValue::Expression`, `serde_json::from_value` refuses to deserialize the `{"$expr": "..."}` envelope into the typed property field. Seam: `crates/credential/tests/properties_pipeline.rs`.

### Capability sub-traits (Tech Spec §15.4)

Capabilities are not const flags — they are **sub-traits**. A credential opts into a capability by `impl <Capability> for <Cred>`, and an engine dispatcher binds `where C: <Capability>`. Plugins cannot self-attest false capabilities (closes security-lead findings N1+N3+N5).

| Capability | Sub-trait | Notes |
|---|---|---|
| Multi-step interactive resolve | `Interactive` | Carries `type Pending` (was on base trait pre-§15.4) |
| Token refresh | `Refreshable` | |
| Provider-side revocation | `Revocable` | |
| Live health probe | `Testable` | |
| Per-execution ephemeral lease | `Dynamic` | |

`Capabilities` (bitflags), `compute_capabilities::<C>() -> Capabilities`, `plugin_capability_report::*` — registration-time capability fold (Tech Spec §15.8). The `#[derive(Credential)]` macro's `capabilities(...)` argument emits one `plugin_capability_report::IsX` impl per declared capability and a parity assertion that consumes the actual sub-trait bound, so a missing `impl Refreshable for X` fails to compile.

### Slot field type for Action / Resource

`#[credential(key = "…")]` slot fields on `#[derive(Action)]` and `#[derive(Resource)]` structs hold `CredentialGuard<C::Scheme>` (the projected auth scheme), not `CredentialGuard<C>` (the credential type). The framework projects `C::State` → `C::Scheme` before populating the slot.

### Other public API

- `Interactive`, `Refreshable`, `Revocable`, `Testable`, `Dynamic` — capability sub-traits (Tech Spec §15.4). `Interactive` carries the `Pending` associated type.
- `CredentialState` — supertrait `ZeroizeOnDrop` is mandatory (Tech Spec §15.4 amendment); compile-fail probe `compile_fail_state_zeroize` enforces.
- `CredentialMetadata`, `CredentialMetadataBuilder` — static type descriptor: key, name, schema (`ValidSchema`), `AuthPattern`. `capabilities_enabled` field removed in §15.8 — capability sets come from sub-trait membership at registration.
- `CredentialRegistry`, `RegisterError` — `register<C>(instance, registering_crate) -> Result<(), RegisterError>`; duplicates fatal in debug + release (Tech Spec §15.6). `iter_compatible(required: Capabilities)` for slot-picker / discovery code.
- `AuthScheme` (base) + `SensitiveScheme: AuthScheme + ZeroizeOnDrop` + `PublicScheme: AuthScheme` — the П1 sensitivity dichotomy (Tech Spec §15.5).
- 9 built-in scheme types: `SecretToken`, `IdentityPassword`, `OAuth2Token`, `KeyPair`, `Certificate`, `SigningKey`, `ConnectionUri`, `InstanceBinding`, `SharedKey`. Each is `SensitiveScheme` or `PublicScheme` per §15.5.
- `SchemeGuard<'a, C>`, `SchemeFactory<C>` — refresh-hook surface (Tech Spec §15.7). `SchemeGuard` is `!Clone`, lifetime-pinned, drop-zeroizes through the wrapped scheme's `ZeroizeOnDrop` impl.
- `CredentialRecord` — runtime operational state (created_at, version, expiry, tags); non-sensitive domain representation. Previously named `Metadata` (ADR 0004).
- `CredentialStore`, `StoredCredential`, `PutMode`, `StoreError` — storage trait with layered composition; concrete impls (in-memory / SQLite / Postgres) live in `nebula_storage::credential`.
- `SecretString` — string type with automatic zeroization on drop.
- `CredentialGuard` — secure RAII wrapper with `Deref` + zeroize on drop; implements `Guard` and `TypedGuard` from `nebula-core`.
- `CredentialRef<C>` — lazy reference type (`id: String` + `PhantomData<fn() -> C>`). New in Phase 1. Resolves to `CredentialGuard<C::Scheme>` via `.resolve(ctx).await`.
- `NoPendingState`, `PendingState`, `PendingToken` — pending state for interactive flows (`Pending` lives on `Interactive` per §15.4).
- `PendingStateStore`, `InMemoryPendingStore`, `PendingStoreError` — pending-state contract and in-memory shim.
- `EncryptedData`, `EncryptionKey`, `encrypt_with_aad`, `encrypt_with_key_id`, `decrypt`, `decrypt_with_aad` — AES-256-GCM crypto primitives. The AAD-free `encrypt` path is intentionally not exposed (SEC-11 hardening 2026-04-27).
- `#[derive(Credential)]`, `#[derive(AuthScheme)]` (with `sensitive` / `public` / `external` argument) — proc-macro derivations.
- `#[capability]` (in `nebula-credential-macros`) — capability sub-trait declaration with sealed companion + phantom-shim companion per ADR-0035.
- `CredentialRotationEvent`, `RotationError` (feature `rotation`) — rotation event and error types.
- `OAuth2Credential`, `ApiKeyCredential`, `BasicAuthCredential` — built-in credential implementations.
- `StaticProtocol` — reusable pattern for static credentials (State = Scheme).
- `ExternalProvider`, `ExternalProviderChain`, `ExternalReference`, `ProviderFuture`, `ProviderResolution`, `LeaseHandle`, `LeasedProvider`, `ProviderKind`, `ProviderError` — external provider abstraction (per ADR-0051) for Vault, AWS Secrets Manager, GCP Secret Manager, Azure Key Vault, and other secret managers. Trait is dyn-safe via the `ProviderFuture<'a>` newtype (AWS `NowOrLater` pattern); resolutions return a `ProviderResolution` envelope (secret + optional lease + optional TTL); `ExternalProviderChain` composes providers with error-discriminated fallback (only `ProviderError::NotFound` falls through to the next provider, all other errors short-circuit). Lease-aware backends (Vault dynamic secrets, AWS STS) implement the `LeasedProvider` sub-trait for `renew` / `revoke`; capability is discovered without runtime downcasts via `ExternalProvider::lease_renewal()`, which the chain and the `ProviderCacheLayer` in `nebula-storage` forward to their inner.
- `CredentialMetrics` — standardized credential operation metric names and label helpers (`resolve_total`, `refresh_total`, `rotations_total`, etc.).
- `prelude` module — convenient re-exports of common credential types.

## Migration recipe (pre-v4 → v4)

The Phase 5 break renames `type Input` → `type Properties` and shifts schema ownership from instance metadata to a typed companion struct. Migration:

1. **Extract a `<Name>Properties` struct** from the previous in-metadata schema definition. Annotate with `#[derive(Schema, Deserialize)]` and per-field `#[field(...)]` / `#[validate(...)]` attributes (Phase 2 namespace).
2. **Rename `type Input = …` → `type Properties = …`** on the `Credential` impl. The schema is read via `nebula_schema::schema_of::<Self::Properties>()` (ADR-0052 P3 — no per-trait schema method; the `Properties: HasSchema` bound is the single source of truth).
3. **Drop `CredentialMetadata.properties: ValidSchema`** in builder calls; the schema is now derived from the type, not baked into the metadata struct.
4. **For capability traits**, ensure each declared capability has a matching `impl <Capability> for <Cred>`. Pre-§15.4 const flags are gone — declared-but-not-implemented capability is a compile error now.
5. **For `#[derive(Credential)]`** (new), parse `#[credential(key, name, scheme, properties|protocol, capabilities(...))]`. Two modes: `properties = TypePath` (user supplies `resolve` + `project`) or `protocol = TypePath` (derive auto-emits `resolve` delegating to `StaticProtocol`).
6. **For action / resource consumers**, update slot fields to `CredentialGuard<C::Scheme>` (not `CredentialGuard<C>`). The framework projects state→scheme before populating the slot.

## Runnable examples

- `cargo run -p nebula-examples --example resource_resident_http` — credential refresh hook on a Resident-topology HTTP client (OAuth2-style)

## Contract

- **[L2-§12.5]** Encryption at rest uses authenticated encryption (AES-256-GCM). No bypass for debugging. `SecretString` and `Zeroizing<Vec<u8>>` on all intermediate plaintext buffers. `Debug` impls on credential wrappers redact secret fields. Seam: `crates/crypto/src` (`Cipher` / `Kdf`, extracted per ADR-0088/0092) consumed here via injected `Arc<dyn Cipher>`; `EncryptionLayer` wiring lives in `nebula-storage`.
- **[L2-§13.2]** Credential refresh and rotation must not silently strand or corrupt in-flight executions that hold valid material. Failure is explicit in status or errors if the system cannot reconcile. Seam: `crates/credential/src/runtime/resolver.rs` — `CredentialResolver::resolve_with_refresh` (relocated from `nebula-engine` per ADR-0092).
- **[L1-§3.5]** The credential subsystem owns the stored-state vs consumer-facing auth-material split. Action authors never hand-roll refresh or pending OAuth steps. Seam: `Credential::project()`.
- **[L2-§12.5 / Phase 9]** **Expressions are NOT allowed in credential property values.** Credential property JSON flows through `<C::Properties as HasSchema>::schema().validate(...)` and then directly into `serde_json::from_value::<C::Properties>(...)` — the credential pipeline deliberately omits the `ValidValues::resolve(&dyn ExpressionContext)` step that the action input pipeline runs. Rationale: secrets must not depend on runtime workflow state. A property value resolved via `{{ … }}` would couple credential storage to per-execution variables, breaking encapsulation and making secret rotation reason about workflow context. Defense in depth: even if a `{{ … }}` template survives validation as `FieldValue::Expression`, `serde_json::from_value` refuses to deserialize the `{"$expr": "..."}` envelope into the typed property field. Seam: `crates/credential/tests/properties_pipeline.rs`. Action input properties retain expression support; only credential properties are frozen JSON.
- **Rename note** — `CredentialRecord` was `Metadata` and `CredentialMetadata` was `Description` before ADR 0004 (commit `51baa36f`). All references to the old names are stale.

## Non-goals

- Not a secret manager (Vault, AWS Secrets Manager) — this is the domain contract layer, not a storage backend.
- Not responsible for secret storage backends — composable layers (`EncryptionLayer`, etc.) wrap any `CredentialStore`.
- Not an OAuth2 server — PKCE and device-code flows are client-side helpers; the OAuth2 authorization endpoint is external.
- Not the schema system — field definitions use `nebula-schema`. Phase 5: schema lives on `Self::Properties` (a `#[derive(Schema)]` companion struct) rather than baked into `CredentialMetadata`; the engine reads it via `nebula_schema::schema_of::<C::Properties>()` (ADR-0052 P3).

## П1 trait shape (2026-04-26)

The credential П1 phase landed the validated CP5/CP6 trait shape per Tech Spec §15.4-§15.8. Key shifts versus the pre-П1 surface:

- **Capability sub-trait split (§15.4).** The 4 capability bools (`INTERACTIVE` / `REFRESHABLE` / `REVOCABLE` / `TESTABLE`) and the production `DYNAMIC` flag are gone. Credentials opt into capabilities by implementing `Interactive`, `Refreshable`, `Revocable`, `Testable`, or `Dynamic`. The `Pending` associated type lives on `Interactive` (was on the base trait). Engine dispatchers bind `where C: Refreshable` rather than reading a const; the silent-downgrade vector ("const says `true` but method defaults to `NotSupported`") is structurally absent. Closes security-lead N1+N3+N5.
- **`AuthScheme` sensitivity classification (§15.5).** `AuthScheme` is the base; sensitive material implements `SensitiveScheme: AuthScheme + ZeroizeOnDrop`, public (no-secret) material implements `PublicScheme: AuthScheme`, and material held in an out-of-process signer (HSM / KMS / FIDO) implements `ExternalScheme: AuthScheme` — no `ZeroizeOnDrop` (no in-process bytes), but the handle is a signing capability so it is not harmless-public either. Derive macros `#[auth_scheme(sensitive)]` / `#[auth_scheme(public)]` / `#[auth_scheme(external)]` audit fields at expansion (forbid plain `String` for sensitive; forbid `SecretString`/`SecretBytes` for public **and** external — an in-process secret means the scheme is `sensitive`, so `external` cannot be a zeroize-bypass channel; name-based lint on `token` / `secret` / `key` / `password`). `OAuth2Token::bearer_header` returns `SecretString`; `ConnectionUri` exposes structured accessors only. Closes N2+N4+N10.
- **Fatal duplicate-KEY registration (§15.6).** `CredentialRegistry::register<C>(instance, registering_crate)` returns `Result<(), RegisterError>` — duplicates are fatal in **both** debug and release builds. The previous "panic in debug, warn + overwrite in release" pattern is removed. Operators resolve via plugin uninstall, version pin, or namespace fix at startup rather than discovering silent credential takeover at runtime. Closes N7 (interim until signed-manifest infra lands).
- **`SchemeGuard` + `SchemeFactory` refresh hook (§15.7).** Long-lived resources receive `SchemeGuard<'a, C>` (`!Clone`, drop-zeroizes via `SensitiveScheme: ZeroizeOnDrop`, lifetime-pinned by `PhantomData<&'a ()>`) instead of `&Scheme`. `SchemeFactory<C>` is the re-acquisition mechanism for connection pools / daemons that need fresh material per request. The refresh-notification hook itself lives on `nebula_resource::Resource::on_credential_refresh` per ADR-0044 (which supersedes ADR-0036 — slot-binding lands the per-slot rotation hook with `&mut self` + slot_name).
- **Capability-from-type (§15.8).** `CredentialMetadata::capabilities_enabled` is removed. Capability sets come from `compute_capabilities::<C>()` over the `plugin_capability_report::Is*` constants (set by sub-trait membership) at registration; plugins cannot self-attest false capabilities. `CredentialRegistry::iter_compatible(required: Capabilities) -> impl Iterator<Item = (&str, Capabilities)>` is the discovery surface for slot pickers. Closes N6.
- **ADR-0035 phantom-shim canonical form.** `dyn ServiceCapability` requires a per-capability `mod sealed_caps` + `dyn ServiceCapabilityPhantom` rewrite — see [ADR-0035](../../docs/adr/HISTORICAL.md) (amendments 2026-04-24-B + -C + 2026-04-26 rename). The `#[capability]` proc-macro and `#[action_phantom]` rewriter make this one-line for plugin authors.

Plugin authors: see [`src/credentials/`](src/credentials/) for canonical capability sub-trait impls and the `mod sealed_caps` convention (the first-party builtin types were folded into this crate when `nebula-credential-builtin` was deleted per ADR-0092). The landing-gate compile-fail probes in `tests/compile_fail_*.rs` document every invariant — read those first when a credential change feels load-bearing.

## Maturity

See `docs/MATURITY.md` row for `nebula-credential`.

- API stability: `stable` — M12.2 hardening closed 2026-05-20. Error taxonomy reshape per Smithy RFC-0022 (per-variant context structs + boxed payloads + 32-byte size cap closes #588); `SecretString` is a thin wrapper over `secrecy::SecretBox<String>` with `ExposeSecret` trait surface; `ValidatedCredentialBinding` newtype closes the `slot_bindings` confused-deputy non-goal from the ADR-0052 cascade; `CredentialService::resolve_for_slot` is the production bind-population seam; fallback-on-interrupt protects in-flight executions from transient provider failures; three-registry sync invariant probe + composite `register_credential_complete` close the silent-drift vector; dyn-compat probe locks the plugin registry against Rust 1.95 next-gen solver regressions. Phase 5 / M6 trait shape (`type Properties` replacing `type Input`, schema ownership on typed companion structs, 2026-04-29) and P1 trait scaffolding (capability sub-trait split, sensitivity dichotomy, fatal duplicate-KEY registration, `SchemeGuard` / `SchemeFactory` refresh hook, capability-from-type) preserved. 9 scheme types, store contract, and secret primitives implemented. Runtime resolver/registry/executor consolidated into `nebula-credential::runtime` (relocated from `nebula-engine` per ADR-0092). `CredentialContext` embeds `BaseContext` and implements `Context` trait from `nebula-core`. ADR-0084 defers proactive pre-expiry refresh to 1.1. Rotation feature (`rotation`) is feature-gated and still evolving.
- `#![forbid(unsafe_code)]` enforced.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5 (integration model — stored-state vs projected auth-material split), §12.5 (secrets + auth invariants), §13.2 (rotation/refresh seam).
- ADRs: `docs/adr/0081-m6-resource-credential-integration.md` (M6 binding/credential cascade — consolidates ADR-0042/0043/0044; drops `Resource::Credential`, per-slot rotation hook).
- Integration model: `docs/INTEGRATION_MODEL.md` §`nebula-credential`.
- ADR: ADR-0004 (historical — `docs/adr/HISTORICAL.md`) (Metadata→Record, Description→Metadata).
- Siblings: `nebula-core` (cross-cutting IDs/scopes), `nebula-crypto` (`Cipher`/`Kdf` injected here), `nebula-schema` (`ValidSchema` consumed by `Credential::Properties` companion structs), `nebula-action` (binds via `#[credential]` slot fields), `nebula-resource` (binds via `#[credential]` slot fields + owns per-slot rotation fan-out), `nebula-engine` (`credential` module = accessor bridges + test coordinator only; runtime relocated here per ADR-0092), `nebula-storage` (`credential` module owns store impls/layers + Encryption/Cache/Audit decorators).

## Appendix

### Authenticated encryption details (evicted from PRODUCT_CANON.md §12.5)

Credentials at rest are encrypted with **AES-256-GCM** using **Argon2id** as the key derivation function. The credential ID is bound as additional authenticated data (AAD), ensuring ciphertext is tied to the specific credential record — no legacy fallback without AAD. Key rotation is supported via multi-key storage with lazy re-encryption on read.

Specific algorithm/KDF/parameters: see `src/crypto.rs` for the authoritative implementation. These choices are L4 implementation detail — changing the algorithm or parameters requires updating this README and `src/crypto.rs`; no canon revision needed. The L2 invariant ("encryption at rest uses authenticated encryption; do not bypass for debugging") lives in canon §12.5.
