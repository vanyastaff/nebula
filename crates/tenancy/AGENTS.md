# nebula-tenancy — Agent orientation
> Agent quick-map for `crates/tenancy/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Multi-tenancy security boundary — resolves an authenticated `Principal` into the port `Scope` and wraps the enumerated general Scope-taking storage ports in scope-substituting decorators so callers cannot forge another tenant's scope. Credential persistence is the deliberate owner-bound exception.
**Layer:** Business — depends only downward (`nebula-storage-port`, `nebula-core`; no credential, sqlx, adapter, or upward dependencies).

## Commands
- `cargo check -p nebula-tenancy`
- `cargo nextest run -p nebula-tenancy`  ·  doctests: disabled (`[lib] doctest = false` — none to run)

## Key files
- `src/lib.rs` — re-export surface for the general principal-to-scope policy and scoped port decorators
- `src/resolver.rs` — `Principal`, `ScopeResolver` trait, default `BindingScopeResolver`, `request_scope(&TenantContext)`; the fail-closed `Principal`→`Scope` projection
- `src/error.rs` — `TenancyError` (`MissingWorkspace` / `Unauthorized`); coarse on purpose, never reveals which half mismatched
- `src/decorator/mod.rs` + `decorator/*.rs` — `Scoped*Store` wrappers for execution, workflow, control_queue, idempotency, journal, node_result, resource, trigger, and webhook; each substitutes the bound scope on every call. This is not a promise that every storage-port trait has a decorator.

## Conventions & never-do
- **Substitute, never compare-and-reject.** Decorators inject the bound `Scope`; let the backend `WHERE workspace_id=? AND org_id=?` filter. A distinct "wrong scope" vs "no row" path is an existence oracle — id↔scope mismatch must surface as `NotFound`/`Ok(None)`.
- **Fail-closed projection.** Absent workspace binding ⇒ `TenancyError::MissingWorkspace`; never silently widen to org-only. Credential command authority is a separate injected policy owned by `nebula-credential` and composed in `apps/server`; tenancy has no credential admin bypass.
- This crate owns scoping **policy** only — it must NOT own the `Scope` type (that is Core-tier `nebula-storage-port` plain data) and must NOT add a backend/sqlx dependency.
- Credential persistence is already owner-bound by `CredentialSelector`; do not reintroduce a metadata-keyed credential decorator here. Audit/encryption/cache/backend composition remains in `nebula-storage`.
- Direct downward domain/port dependencies follow the root layer map; durable cross-crate commands/facts use persisted state or explicit outbox/inbox ports; nebula-eventbus carries only lossy observation and wake hints.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design + threat-model table (spec §6.1) · ADR-0072 (storage port/adapter/tenancy split) · ADR-0029 (credential scope-layer fail-closed audit)
