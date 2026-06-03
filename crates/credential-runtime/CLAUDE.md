# nebula-credential-runtime — Claude Code orientation
> Agent quick-map for `crates/credential-runtime/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** The `CredentialService<B, PS>` facade — the owner-isolated runtime that owns the whole credential lifecycle (resolve, refresh, rotate, revoke, bind-population) behind one typed entry point (ADR-0066).
**Layer:** Exec — depends only downward (`nebula-credential`, `nebula-tenancy`, `nebula-storage`, `nebula-engine`, `nebula-schema`; cross-cutting error/log/eventbus/resilience). `engine` + `api` consume it.

## Commands
- `cargo check -p nebula-credential-runtime`
- `cargo nextest run -p nebula-credential-runtime`  ·  doctests: `cargo test -p nebula-credential-runtime --doc`
- `cargo nextest run -p nebula-credential-runtime --features test-util` — needed for `test_support` / `tests/` (in-memory service); `test-util` is dev/test-only, never a release path (ADR-0023).
- `compile_fail/` trybuild cases gate the `ValidatedCredentialBinding` sealed constructor.

## Key files
- `src/service.rs` — `CredentialService`, `Acquisition`, `LayeredStore`, `test_support`; the facade + lifecycle (largest module).
- `src/ops.rs` — `DispatchOps` type-erased async op table + `register_*_ops` per-capability registrars; swap for test doubles here.
- `src/binding.rs` — `ValidatedCredentialBinding` + `TenantFingerprint`: crate-private-constructor newtype closing the `slot_bindings` confused-deputy (only `resolve_for_slot` mints one).
- `src/scope.rs` — `TenantScope` / `FixedScopeResolver`: owner_id derivation (`Scope::credential_owner_id`, ADR-0088 D7).
- `src/error.rs` — `CredentialServiceError` taxonomy (Smithy RFC-0022: per-variant context structs, boxed payloads, 32-byte cap).
- `src/observer.rs` — `CredentialObserver` / `EventMetricObserver` seam (e.g. the `RefreshFallback` span event).
- `src/builder.rs` — `CredentialServiceBuilder`; secure construction is the only path.

## Conventions & never-do
- Tenant isolation lives at the facade boundary — `owner_id` propagates to every call; downstream sinks must never see cross-tenant data. Invariant-bearing composition stays crate-private.
- Only `CredentialService::resolve_for_slot` may construct a `ValidatedCredentialBinding`; engine consumers receive the sealed value. Do not add other constructors.
- Capability (refreshable/testable/revocable/interactive/dynamic) reads from the single `nebula_credential::CredentialRegistry` `Capabilities` bitflag — do NOT reintroduce a parallel flag table (removed in ADR-0088 D3).
- Out of scope: proactive pre-expiry refresh (deferred to 1.1, ADR-0084); reactive `OnceCell`+`RefreshClaimRepo` path is the 1.0 contract. `test-util` MUST stay out of release builds (ADR-0023).
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · `docs/adr/0066-credential-runtime-facade.md` · `docs/adr/0084-pre-expiry-credential-refresh-deferred.md` · ADR-0052 cascade (confused-deputy), ADR-0088 D3/D7 (single capability source, owner_id).
