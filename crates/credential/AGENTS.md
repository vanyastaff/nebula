# nebula-credential — Agent orientation
> Local guide for `crates/credential/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** The typed Credential Contract — declares the split between stored `State` (encrypted at rest) and projected auth `Scheme` (what action code receives). Runtime resolve/refresh/rotation **orchestration** lives in `src/runtime/` and `CredentialService` (ADR-0092). `apps/server` is the first-party composition root and owns production key, storage, catalog, refresh, and authority adapters. `nebula-api` retains only unsupported `test-util` fixtures; `nebula-engine` consumes typed runtime seams, and neither duplicates resolver logic.
**Layer:** Shared-infra (credential contract) — importable by Exec/API/Business per the `deny.toml` `[bans].deny` `wrappers` allowlist; depends only on Core + cross-cutting (root AGENTS.md → Layered Dependency Map).

## Common Tasks

| Task | Steps |
|------|-------|
| Add a credential or scheme | Implement the scheme's `AuthScheme`/sensitivity contract; define a `Credential` whose `Scheme` projects it. `CredentialRegistry::register` registers credential instances, not bare schemes. |
| Add a new capability | Add sub-trait in `src/contract/` — capabilities are sub-trait membership, never const flags. Duplicate-KEY `register` is fatal. |
| Add external secret provider | Extend `ExternalProvider` chain in `src/provider/` (ADR-0051). Error-discriminated fallback: only `NotFound` falls through. |

## Commands

- Feature flags: `rotation` (gated, evolving)
- `tests/compile_fail_*.rs` encode capability, sensitivity, guard, and slot invariants. For a timeout, reproduce the affected target with `cargo test -p nebula-credential --test <target>` to distinguish compilation cost from a hang, then rerun the required nextest check. Never accept a timeout as passing evidence.

## Key files

- `src/lib.rs` — flat root re-exports are the canonical surface (`use nebula_credential::SecretString`); submodules are escape hatches only.
- `src/contract/` — `Credential` base trait + capability sub-traits (`Interactive`/`Refreshable`/`Revocable`/`Testable`/`Dynamic`), `CredentialRegistry`, resolve types.
- `src/scheme/` — `AuthScheme` base + `SensitiveScheme`/`PublicScheme` dichotomy (§15.5) + 9 built-in scheme types.
- `src/secrets/` — `SecretString`, `CredentialGuard`, `SchemeGuard`/`SchemeFactory` refresh surface, PKCE helpers (AES-GCM crypto moved out, see below).
- `src/runtime/resolver.rs` — `CredentialResolver` (cached handles, `scheme_factory`, `resolve_with_refresh`).
- `src/service/facade.rs` — `CredentialService` (`resolve_for_slot`, `scheme_factory` for §15.7 resource pools).
- `src/service/controller.rs` — one-decision authority boundary; `src/service/crud.rs` — semantic mutations and property validation.
- `src/lifecycle.rs` — capabilities-as-data (`CredentialPolicy`/`RefreshStrategy`/`RevokeStrategy`, ADR-0088 D2).
- `src/provider/` — `ExternalProvider` chain for Vault/AWS/GCP/Azure secret managers (ADR-0051); error-discriminated fallback (only `NotFound` falls through).

## Conventions & never-do

- **No expression execution for credential properties.** Decode property JSON as
  literal `AuthoredValue` under the declared `C::Properties` schema, consume it
  through `validate` and `resolve_data`, decode `C::Properties` once at the trusted
  secret-aware boundary, and pass `&C::Properties` to provider resolve. Never run
  `ValidValues::resolve` against workflow context. Template-like data stays literal;
  executable nodes are rejected. Secret values cannot depend on runtime workflow
  state (seam: `tests/properties_pipeline.rs`).
- **Properties are declarations, not proofs.** `Properties` is the schema-bearing
  type; unknown test doubles use `serde_json::Value`, never `ResolvedValues`.
  Fixtures construct proof through `schema_of::<C::Properties>()`, literal
  ingestion, consuming validation, and `resolve_data`; no fake `Any` proof for
  a concrete credential. Assert required-field and type rejections at the schema
  boundary, including canonical paths and codes, before a provider can be called.
- **Declared secrets are already protected at dispatch.** Built-in secret fields use
  zeroizing `SecretString`. Typed decoding uses the separately named
  `into_typed_exposing_secrets` boundary once; ordinary `into_typed` refuses secrets.
  Keep decoder causes and public reports redacted, and never repeat transforms on
  prepared values.
- **Catalog construction is checked.** `Credential::metadata` returns a
  schema-free `CredentialMetadataDraft`; registry admission derives the one
  canonical schema from `C::Properties` through fallible `schema_of`. Propagate
  admission failures before registration or dispatch; do not panic or
  substitute an empty schema.
- **Crypto lives in `nebula-crypto`** (ADR-0088): import AES-256-GCM/`EncryptedData`/`encrypt_with_aad` from there, NOT this crate. AAD-free `encrypt` is deliberately unexposed (SEC-11). The object-safe persistence contract and port-local rows live in `nebula-storage-port`; the sole backend/decorator implementations live in `nebula-storage`. On the supported authenticated HTTP management path, `CredentialController` derives mandatory owner-bound selectors only after authority allows the command. Technical runtime/service paths still accept `TenantScope`; making the controller plus operation ledger the sole semantic writer is K3 debt.
- **Capabilities are sub-trait membership, never const flags** — duplicate-KEY `register` is fatal in debug AND release; a declared-but-unimplemented capability is a compile error. Don't reintroduce capability bools or per-trait `*_schema` (schema = `Properties: HasSchema`, read via `schema_of`).
- `CredentialState` requires `ZeroizeOnDrop`; `Debug` redacts secrets; `SchemeGuard` is `!Clone` and drop-zeroizes.
- Refresh authority is the backend-authored material epoch, not serialized-byte equality or the general row version. Display/gate transitions preserve it; explicit material/reconnect, every durable reauthentication decision, and every successful refresh advance it even for byte-identical data and clear the old gate. Exact local finalization failures remain distinct from `OutcomeUnknown` for both winners and payload-free L1 waiters, and both retain the claim fail-closed.
- First-party deployment wiring belongs in `apps/server`; `nebula-api::ports::credential_service_factory` is an unsupported `test-util` fixture and must never acquire production or provider policy.
- Supported authenticated HTTP management calls enter through `CredentialController`: one injected `CredentialTenantAuthority` decision, then one privately minted owner-bound command. Port-local owner/selector constructors and `CredentialPersistence` are public technical data/contracts, not authority and not supported SDK/API surfaces. Never add `None == admin`, expose those handles to handlers/integrations, or describe K1 as the K3 sole-writer/ledger closure.

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Validation and secrecy | [properties_pipeline](tests/properties_pipeline.rs), [redaction](tests/redaction.rs), [serde_redaction](tests/serde_redaction.rs). |
| Capability registration | [registry_capabilities_iter](tests/registry_capabilities_iter.rs), [runtime_duplicate_key_fatal](tests/runtime_duplicate_key_fatal.rs), and the affected compile-fail suite. |
| Refresh and persistence boundary | [refresh_routing_architecture](tests/refresh_routing_architecture.rs), plus the affected storage refresh/lifecycle tests; local runtime tests alone do not prove backend atomicity. |

## See also

- `docs/DESIGN.md` — current K1 authority and K2 persistence boundary plus explicit K3/K4
  follow-up work
- `README.md` — current shipped design (v4 / Phase 5 trait shape, §15.4–15.8, migration recipe)
- Canon §3.5 / §12.5 / §13.2; ADR-0081; ADR-0088 (crypto split), ADR-0051 (external providers), ADR-0033 (Plane B, in `HISTORICAL.md`)
