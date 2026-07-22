# nebula-credential — Agent orientation
> Agent quick-map for `crates/credential/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** The typed Credential Contract — declares the split between stored `State` (encrypted at rest) and projected auth `Scheme` (what action code receives). Runtime resolve/refresh/rotation **orchestration** lives in `src/runtime/` and `CredentialService` (ADR-0092). `apps/server` is the first-party composition root and owns production key, storage, catalog, refresh, and authority adapters. `nebula-api` retains only unsupported `test-util` fixtures; `nebula-engine` consumes typed runtime seams, and neither duplicates resolver logic.
**Layer:** Shared-infra (credential contract) — importable by Exec/API/Business per the `deny.toml` `[bans].deny` `wrappers` allowlist; depends only on Core + cross-cutting (root AGENTS.md → Layered Dependency Map).

## Common Tasks

| Task | Steps |
|------|-------|
| Add a new credential scheme | 1. Define scheme in `src/scheme/` implementing `AuthScheme` 2. Register in `CredentialRegistry` |
| Add a new capability | Add sub-trait in `src/contract/` — capabilities are sub-trait membership, never const flags. Duplicate-KEY `register` is fatal. |
| Add external secret provider | Extend `ExternalProvider` chain in `src/provider/` (ADR-0051). Error-discriminated fallback: only `NotFound` falls through. |
| Understand crypto | Crypto lives in `nebula-crypto` (ADR-0088). Import AES-256-GCM from there. AAD-free `encrypt` is deliberately unexposed (SEC-11). |
| Test credential properties | Compile-fail tests in `compile_fail_*.rs` encode load-bearing invariants. Read these first when changes feel risky. |

## Commands
- `cargo check -p nebula-credential`
- `cargo nextest run -p nebula-credential`  ·  doctests: `cargo test -p nebula-credential --doc`
- Feature flags: `rotation` (gated, evolving)
- `compile_fail_*.rs` (trybuild) encode the load-bearing invariants — read these first when a change feels risky; may false-TIMEOUT on cold cache under nextest (warm + plain `cargo test`).

## Key files
- `src/lib.rs` — flat root re-exports are the canonical surface (`use nebula_credential::SecretString`); submodules are escape hatches only.
- `src/contract/` — `Credential` base trait + capability sub-traits (`Interactive`/`Refreshable`/`Revocable`/`Testable`/`Dynamic`), `CredentialRegistry`, resolve types.
- `src/scheme/` — `AuthScheme` base + `SensitiveScheme`/`PublicScheme` dichotomy (§15.5) + 9 built-in scheme types.
- `src/secrets/` — `SecretString`, `CredentialGuard`, `SchemeGuard`/`SchemeFactory` refresh surface, PKCE helpers (AES-GCM crypto moved out, see below).
- `src/runtime/resolver.rs` — `CredentialResolver` (cached handles, `scheme_factory`, `resolve_with_refresh`).
- `src/service/facade.rs` — `CredentialService` (`resolve_for_slot`, `scheme_factory` for §15.7 resource pools).
- `src/lifecycle.rs` — capabilities-as-data (`CredentialPolicy`/`RefreshStrategy`/`RevokeStrategy`, ADR-0088 D2).
- `src/provider/` — `ExternalProvider` chain for Vault/AWS/GCP/Azure secret managers (ADR-0051); error-discriminated fallback (only `NotFound` falls through).

## Conventions & never-do
- **No expressions in credential property values** — property JSON validates then `serde_json::from_value::<C::Properties>` directly; never run `ValidValues::resolve`. Secrets must not depend on runtime workflow state (seam: `tests/properties_pipeline.rs`).
- **Crypto lives in `nebula-crypto`** (ADR-0088): import AES-256-GCM/`EncryptedData`/`encrypt_with_aad` from there, NOT this crate. AAD-free `encrypt` is deliberately unexposed (SEC-11). The object-safe persistence contract and port-local rows live in `nebula-storage-port`; the sole backend/decorator implementations live in `nebula-storage`. On the supported authenticated HTTP management path, `CredentialController` derives mandatory owner-bound selectors only after authority allows the command. Technical runtime/service paths still accept `TenantScope`; making the controller plus operation ledger the sole semantic writer is K3 debt.
- **Capabilities are sub-trait membership, never const flags** — duplicate-KEY `register` is fatal in debug AND release; a declared-but-unimplemented capability is a compile error. Don't reintroduce capability bools or per-trait `*_schema` (schema = `Properties: HasSchema`, read via `schema_of`).
- `CredentialState` requires `ZeroizeOnDrop`; `Debug` redacts secrets; `SchemeGuard` is `!Clone` and drop-zeroizes.
- Direct downward domain/port dependencies follow the root layer map; durable cross-crate commands/facts use persisted state or explicit outbox/inbox ports; nebula-eventbus carries only lossy observation and wake hints.
- First-party deployment wiring belongs in `apps/server`; `nebula-api::ports::credential_service_factory` is an unsupported `test-util` fixture and must never acquire production or provider policy.
- Supported authenticated HTTP management calls enter through `CredentialController`: one injected `CredentialTenantAuthority` decision, then one privately minted owner-bound command. Port-local owner/selector constructors and `CredentialPersistence` are public technical data/contracts, not authority and not supported SDK/API surfaces. Never add `None == admin`, expose those handles to handlers/integrations, or describe K1 as the K3 sole-writer/ledger closure.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code (`#![forbid(unsafe_code)]`).

## See also
- `docs/DESIGN.md` — current K1 boundary plus explicit K2/K3/K4 follow-up work
- `README.md` — current shipped design (v4 / Phase 5 trait shape, §15.4–15.8, migration recipe)
- Canon §3.5 / §12.5 / §13.2; ADR-0081; ADR-0088 (crypto split), ADR-0051 (external providers), ADR-0033 (Plane B, in `HISTORICAL.md`)
