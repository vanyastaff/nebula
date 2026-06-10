---
name: nebula-credential-lifecycle
description: Use when adding or changing credentials, OAuth refresh, secret rotation, Vault/external providers, per-tenant owner scoping, or credential slots on actions/resources.
---

# Nebula credential lifecycle

**When to use:** touching anything in the credential subsystem — defining a new
credential type, the resolve/refresh/rotate/revoke lifecycle, owner-id / tenant
isolation, external secret backends (Vault), OAuth identity from operator
secrets, or binding credential slots into actions and resources.

Before changing code, read the relevant source first. Authority docs:
`docs/adr/0081-m6-resource-credential-integration.md` (binding/runtime/rotation
contract), `docs/adr/0084-pre-expiry-credential-refresh-deferred.md`
(reactive-only 1.0), `docs/adr/0085-oauth-identity-providers-from-secrets.md`
(Plane A vs Plane B), `docs/adr/0088-credential-subsystem-rewrite.md`
(**proposed** — do not implement unless told), `crates/credential/README.md`,
`crates/credential/CLAUDE.md`, `docs/INTEGRATION_MODEL.md` (§`nebula-credential`),
and `deny.toml`.

## Crate map — know which crate owns what

| Crate | Layer | Owns |
|---|---|---|
| `nebula-credential` (`crates/credential/`) | Core / shared-infra | The typed **contract**: `Credential` trait, capability sub-traits, `AuthScheme`/scheme types, `CredentialState`, `CredentialGuard`/`SchemeGuard`, `CredentialRegistry`, `lifecycle.rs` policy data, `ExternalProvider` chain (`src/provider/`), secret primitives. **No** runtime orchestration. |
| `nebula-credential-runtime` (`crates/credential-runtime/`) | Exec | The `CredentialService` **facade**: resolve/refresh/rotate/revoke entry points (`src/service.rs`, `src/ops.rs`), validate→encrypt→store pipeline, bind-population seam (`src/binding.rs`, `ValidatedCredentialBinding`). |
| `nebula-credential-builtin` (`crates/credential-builtin/`) | Business | First-party concrete credential types; `register_builtins`. |
| `nebula-credential-vault` (`crates/credential-vault/`) | Business | Concrete `LeasedProvider` backend (HashiCorp Vault, ADR-0051 Phase C). |
| `nebula-crypto` (`crates/crypto/`) | Cross-cutting | AES-256-GCM + Argon2id + `EncryptedData`/`CryptoError`. Crypto was **extracted out** of the credential crate (ADR-0088 D7). |
| `nebula-engine` (`crates/engine/src/credential/`) | Exec | The refresh/rotation **mechanism**: `resolver.rs`, `rotation/resource_fanout.rs`, `lease/scheduler.rs`. |

Procedure: identify which of these your change belongs in **before** editing. A
"small helper in the wrong crate" is a layer violation caught by `cargo deny`.

## Checklist by task

### Defining or changing a credential type

1. The type lives in `nebula-credential` (contract) or a `-builtin`/plugin crate
   — never in `-runtime`, `-engine`, or `-api`.
2. Declare the three associated types: `Properties` (`HasSchema` companion
   struct, NOT `Input`), `State` (persisted), `Scheme` (projected). Read the
   `Credential` trait in `crates/credential/src/contract/`.
3. **State-vs-Scheme split is load-bearing.** Never put projected auth material
   in `State`; `project(&State) -> Scheme` is the only path consumer code sees.
   `State` is what is encrypted at rest; `Scheme` is what action code receives.
4. Secrets use the crate's primitives — `SecretString`, `CredentialGuard`,
   `SchemeGuard` (`!Clone`, drop-zeroizes). **Never a raw `String`** for secret
   material. `CredentialState` requires `ZeroizeOnDrop` (compile-fail-probed).
5. Capabilities are **sub-trait membership** (`Refreshable`/`Revocable`/
   `Testable`/`Dynamic`/`Interactive`), never const bools. A declared-but-
   unimplemented capability is a compile error; duplicate-KEY `register` is fatal
   in debug AND release.
6. Schema comes from `Properties: HasSchema` via `nebula_schema::schema_of::<_>()`
   — no per-trait `*_schema` method.
7. **No expressions in credential property values.** Property JSON validates then
   `serde_json::from_value::<C::Properties>` directly; the pipeline deliberately
   omits `ValidValues::resolve`. Secrets must not depend on runtime workflow
   state (seam: `crates/credential/tests/properties_pipeline.rs`).
8. Crypto: import AES-256-GCM / `EncryptedData` / `encrypt_with_aad` from
   `nebula-crypto`, not the credential crate (ADR-0088 D7). The AAD-free
   `encrypt` path is deliberately unexposed (SEC-11).

### The resolve / refresh / rotate / revoke lifecycle

1. The **runtime facade `CredentialService`** (`crates/credential-runtime/src/service.rs`)
   is the single typed entry point. New lifecycle operations go through it.
2. Do **not** re-home the mechanism: the refresh **coordinator** (L1 in-process
   coalesce + L2 durable `RefreshClaimRepo` claim) and rotation **fan-out** stay
   engine-owned — `crates/engine/src/credential/resolver.rs`,
   `rotation/resource_fanout.rs`, `lease/scheduler.rs`. Resource stays a pure
   consumer of resolved guards; the resolver must not move into `nebula-resource`
   (would invert the dependency).
3. **1.0 reality check — refresh is REACTIVE-only** (ADR-0084). Flow: action uses
   credential → resolver observes expiry (stored TTL or provider failure) → L1+L2
   coalesce one provider call → action proceeds. Proactive/pre-expiry refresh
   (a background scheduler) is **deferred to 1.1** — do NOT add one. Callers
   needing warm-up call `CredentialService::refresh()` themselves.
4. Refresh/rotation must not silently strand in-flight executions holding valid
   material (canon §13.2). Failure is explicit in status or a typed error
   (e.g. `ReauthRequired` for terminal failures), never a silent drop.
5. Observability is Definition of Done: a new state/error/hot path ships a typed
   error variant + tracing span + invariant check.

### Owner-id / tenant isolation (the security primitive)

1. Every lifecycle call is scoped by a **canonical `owner_id`** derived through a
   `Scope` (`nebula-storage-port`, Core). The per-tenant boundary IS the security
   primitive — a tenant's credentials must be invisible to other tenants.
2. The single canonical derivation is `Scope::credential_owner_id`
   (`nebula-storage-port`). Both producers — the API edge (`owner_id_from_scope`)
   and the runtime `TenantScope` — route through it. Do **not** introduce a
   second owner-id format; the `{org}/{ws}` vs `{org}:{ws}` drift was a latent
   cross-tenant key-collision bug (ADR-0088 D7). Segments are length-prefixed so
   the `(org, workspace)` → key map is injective.
3. Enforcement has two physical points and that is intentional: the API plane
   enforces via `nebula_tenancy::ScopeLayer` (Business); the runtime facade
   enforces the same invariant in-Exec via its owner checks
   (`owner_matches`/`load_owned` in `crates/credential-runtime/src/service.rs`).
   They cannot collapse to one instance because Exec→Business is forbidden by
   `cargo deny`.

### Binding credential slots into actions / resources

1. Actions and resources declare **typed `#[credential(key)]` slot fields**, bound
   per-node — NOT implicit type-only resolution (ADR-0081 absorbs ADR-0042). The
   workflow node maps `slot_name → CredentialId` explicitly.
2. Slot field type is `CredentialGuard<C::Scheme>` (the projected scheme), not
   `CredentialGuard<C>`. The framework projects `State → Scheme` before populating
   the slot. Resources use `SlotCell<CredentialGuard<…>>` (lock-free ArcSwap) with
   engine-owned hot-swap + `on_credential_refresh` reactive hooks.
3. The **bind-population producer is still a frontier gap** — there is no
   production credential→slot resolver wired end-to-end (M12.4). The seam exists:
   `ValidatedCredentialBinding` + `CredentialService::resolve_for_slot`
   (`crates/credential-runtime/src/binding.rs`), tenant-fingerprint sealed. Do
   **not** claim bind-population is green; it is a known gap.

### External secret backends (Vault / AWS / GCP / Azure)

1. Implement against the `ExternalProvider` chain (`crates/credential/src/provider/`,
   ADR-0051): dyn-safe via `ProviderFuture<'a>`, resolutions return a
   `ProviderResolution` envelope (secret + optional lease + optional TTL). Lease-
   aware backends impl the `LeasedProvider` sub-trait (`renew`/`revoke`).
2. Chain fallback is error-discriminated: **only `ProviderError::NotFound` falls
   through** to the next provider; any other error short-circuits.
3. Wire a new backend through a **composition root** via `Arc<dyn ExternalProvider>`
   — never a direct upward dep from an Exec/API crate. Then add the crate to the
   appropriate `deny.toml` `[bans].deny` wrapper allowlist **with a reason**
   string (the `wrappers = [...]` list inside the relevant `{ crate = … }` entry;
   `nebula-credential-vault` starts with an empty consumer allowlist by design).

### OAuth

Two non-overlapping planes — do not conflate:

- **Plane A — identity login** (ADR-0085): operator-supplied OAuth client proves
  "this is Alice", Nebula mints its own session and **discards** IdP tokens. This
  lives in `nebula-api` (`crates/api/src/transport/oauth/`). Operator IdP-client
  config is **infrastructure config** in `ApiConfig::auth.oauth.providers` (env
  vars), NOT credential rows — does not touch `CredentialService`. Anti-SSRF gate
  (`validate_oauth_outbound_url`) applies to every server-side OAuth fetch.
- **Plane B — credential OAuth**: a stored OAuth credential lets a workflow node
  call a third-party API on the user's behalf. This is `nebula-credential`'s
  `OAuth2Credential` + `Interactive::continue_resolve`. The two planes coexist;
  Plane A must NOT route through `continue_resolve` (token-discard is a type-level
  property).

## Hard constraints

- **ADR-0088 is PROPOSED, not shipped.** Its Protocol model / `#[nebula::credential]`
  attribute macro / crate re-layering are a **future** rewrite. Design against the
  **accepted** contract (ADR-0081/0084/0085: `Credential` trait + capability
  sub-traits + `Properties`/`State`/`Scheme`) unless explicitly told to implement
  the rewrite. (Partially-landed ADR-0088 pieces already in tree: `nebula-crypto`
  extraction, `crates/credential/src/lifecycle.rs` policy data, registry collapse,
  canonical `Scope::credential_owner_id` — verify the actual code before assuming.)
- Cross-crate communication goes through `nebula-eventbus`, not direct sibling
  imports across layer boundaries.
- Library code: no `unwrap()`/`expect()`/`panic!()`; typed `thiserror`/`NebulaError`;
  `#![forbid(unsafe_code)]`. `Debug` impls redact secret fields.

## Verify

- `cargo check -p nebula-credential` (or the crate you touched)
- `cargo nextest run -p nebula-credential` (+ `-p nebula-credential-runtime`,
  `-p nebula-engine` if you touched the facade or mechanism)
- `cargo test -p nebula-credential --doc`
- The `compile_fail_*.rs` trybuild probes in `crates/credential/tests/` encode the
  load-bearing invariants — read them first when a change feels risky (they may
  false-TIMEOUT on cold cache under nextest; warm + plain `cargo test`).
- `task deny` after any dep/wrapper change.
