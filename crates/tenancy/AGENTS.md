# nebula-tenancy — Agent orientation
> Agent quick-map for `crates/tenancy/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Multi-tenancy security boundary — resolves an authenticated `Principal` into the port `Scope` and wraps each storage-port store in a scope-substituting decorator so callers cannot forge another tenant's scope.
**Layer:** Business — depends only downward (`nebula-storage-port`, `nebula-core`, `nebula-credential`; no sqlx/adapter/upward deps).

## Commands
- `cargo check -p nebula-tenancy`
- `cargo nextest run -p nebula-tenancy`  ·  doctests: disabled (`[lib] doctest = false` — none to run)

## Key files
- `src/lib.rs` — re-export surface; note `Credential`-prefixed names (`CredentialScopeLayer`, `CredentialScopeResolver`) deliberately avoid collision with the port-scope `ScopeResolver`
- `src/resolver.rs` — `Principal`, `ScopeResolver` trait, default `BindingScopeResolver`, `request_scope(&TenantContext)`; the fail-closed `Principal`→`Scope` projection
- `src/error.rs` — `TenancyError` (`MissingWorkspace` / `Unauthorized`); coarse on purpose, never reveals which half mismatched
- `src/decorator/mod.rs` + `decorator/*.rs` — one `Scoped*Store` per port trait (execution, workflow, control_queue, idempotency, journal, node_result, resource, trigger, webhook); each substitutes the bound scope on every call
- `src/credential_scope.rs` — re-homed credential `ScopeLayer` (keys on legacy `metadata["owner_id"]`, distinct from port-scope model)

## Conventions & never-do
- **Substitute, never compare-and-reject.** Decorators inject the bound `Scope`; let the backend `WHERE workspace_id=? AND org_id=?` filter. A distinct "wrong scope" vs "no row" path is an existence oracle — id↔scope mismatch must surface as `NotFound`/`Ok(None)`.
- **Fail-closed projection.** Absent workspace binding ⇒ `TenancyError::MissingWorkspace`; never silently widen to org-only. Credential layer: `None` owner = admin/global bypass.
- This crate owns scoping **policy** only — it must NOT own the `Scope` type (that is Core-tier `nebula-storage-port` plain data) and must NOT add a backend/sqlx dependency.
- Keep the re-homed credential layer order (`ScopeLayer → AuditLayer → EncryptionLayer → CacheLayer → Backend`) and its fail-closed audit + zeroize-on-drop invariants intact (ADR-0029); regression-tested in `crates/storage/tests/credential_*`.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design + threat-model table (spec §6.1) · ADR-0072 (storage port/adapter/tenancy split) · ADR-0029 (credential scope-layer fail-closed audit)
