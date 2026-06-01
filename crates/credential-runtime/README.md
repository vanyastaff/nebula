---
name: nebula-credential-runtime
role: Credential Management Bounded Context (CredentialService facade)
status: stable
last-reviewed: 2026-05-20
canon-invariants: [L2-12.5, L2-13.2]
related: [nebula-credential, nebula-engine, nebula-storage, nebula-tenancy]
---

# nebula-credential-runtime

## Purpose

`nebula-credential-runtime` is the **runtime management bounded context** for credentials: the `CredentialService<B, PS>` facade that owns the complete credential lifecycle — resolve, refresh, rotate, revoke, and bind-population — behind a single typed entry point.

This crate is the production implementation of the ADR-0066 facade contract. It ships:

- **`CredentialService`** — the owner-isolated facade. Tenant isolation is enforced at the facade boundary (`owner_id` propagated to every call); downstream sinks never see cross-tenant data.
- **`DispatchOps`** — the trait that the facade dispatches to; swap implementations for test doubles without touching call-sites.
- **`ValidatedCredentialBinding`** — a crate-private-constructor newtype that closes the `slot_bindings` confused-deputy non-goal from the ADR-0052 cascade. Only `CredentialService::resolve_for_slot` may produce a binding; engine dispatch consumers receive the sealed value.
- **`resolve_for_slot`** — the production bind-population seam. This is the sole entry point for `register_and_bind` callers in `nebula-resource`; it closes the `§M11.5 quiesce contract with zero callers` gap.
- **Fallback-on-interrupt** — on transient provider refresh failure with a non-expired cached snapshot, the service returns the cached material rather than propagating the failure (aws-credential-types resilience pattern). Callers see a `RefreshFallback` diagnostic event on the tracing span.
- **Single capability source** — capability (refreshable / testable / revocable / interactive / dynamic) is read from the `nebula_credential::CredentialRegistry` `Capabilities` bitflag, computed once from sub-trait membership at `register::<C>()`. The former parallel `StateProjectionRegistry` (engine) + `CredentialDispatch` (runtime) flag tables and their sync invariant were removed in ADR-0088 D3 — one table cannot drift from itself. `DispatchOps<B,PS>` remains as the type-erased async **operation**-closure table (it must be generic over the store/pending types).

## Role in the dependency graph

`nebula-credential-runtime` sits at the **Exec** layer (see CLAUDE.md Layered Dependency Map — Exec covers `engine`, `storage`, runtime facades, sandbox). It depends on:

- `nebula-credential` (Core contract types)
- `nebula-tenancy` (tenant scope enforcement)
- `nebula-storage-port` (Core storage seam)
- `nebula-error` / `nebula-log` (Cross-cutting)

The `engine` (Exec tier) and `api` (API tier) consume this crate downward. No upward imports.

## M12.2 hardening status (2026-05-20)

This crate was created as part of the M12.2 `nebula-credential` stabilize sweep. It is `stable` at extraction — all hardening items described in the sweep are reflected in this crate's production path:

- Error taxonomy per Smithy RFC-0022 (per-variant context structs, boxed payloads, 32-byte size cap)
- `ValidatedCredentialBinding` confused-deputy closure
- `resolve_for_slot` bind-population seam (production caller for `register_and_bind`)
- Fallback-on-interrupt for transient refresh failures
- Single capability source (the `CredentialRegistry` bitflag; the parallel dispatch/projection flag tables were removed in ADR-0088 D3)

## Out of scope

- **Proactive pre-expiry refresh** — deferred to 1.1 per ADR-0084. The reactive path (L1 `OnceCell` + L2 `RefreshClaimRepo`) remains the 1.0 contract.
- **Builtin credential catalog** — tracked under M12.3 / #604.
- **OAuth path migration in `nebula-api`** — `AppState` holds `CredentialService` but the OAuth domain code still consumes `scoped_store` via `CredentialScopeLayer`. Full migration is a separate PR; `CredentialScopeLayer` deletion from `nebula-tenancy` follows that.

## ADR references

- [ADR-0066](../../docs/adr/0066-credential-runtime-facade.md) — `CredentialService` facade design
- [ADR-0084](../../docs/adr/0084-pre-expiry-credential-refresh-deferred.md) — Proactive refresh deferred to 1.1
