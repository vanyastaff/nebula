# nebula-api — Claude Code orientation
> Agent quick-map for `crates/api/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** Thin axum HTTP gateway translating REST into typed port-trait calls; all business logic delegates downward, plus inbound webhook + OAuth transports.
**Layer:** API/Public — depends only downward (root CLAUDE.md -> Layered Dependency Map).

## Commands
- `cargo check -p nebula-api`
- `cargo nextest run -p nebula-api`  ·  doctests: `cargo test -p nebula-api --doc`
- OpenAPI/spec guards: `cargo nextest run -p nebula-api --test openapi_spec` (regenerates spec from the router)
- Feature flags: `postgres` (PG idempotency + `PgAuthBackend`), `test-util` (`ApiConfig::for_test`, bypasses JWT gate — never in prod)
- OAuth e2e test-support module needs `RUSTFLAGS="--cfg nebula_test_util"` (custom cfg, not a feature)

## Key files
- `src/lib.rs` — crate root, public re-exports (`build_app`, `AppState`, `ApiConfig`, `ApiError`)
- `src/app.rs` — `build_app`: OpenApiRouter merge + `split_for_parts` + full middleware stack + `serve()`
- `src/state.rs` — `AppState` builder + API-tier port traits (`OrgResolver`/`WorkspaceResolver`/`MembershipStore`/`SessionStore`/`AuthBackend`)
- `src/error/mod.rs` — `ApiError` (§12.4 RFC 9457 seam, `#[non_exhaustive]`)
- `src/middleware/` — auth → tenancy → rbac → csrf → idempotency stack (order is load-bearing: auth before csrf)
- `src/domain/<x>/handler.rs` — §13 knife seams (`create_workflow`, `activate_workflow`, `start_execution`, `cancel_execution`)
- `src/transport/webhook/` — single converged inbound webhook transport (programmatic + slug-routed)

## Conventions & never-do
- Pure library — ships NO binary/composition root; wiring lives in `apps/server` + `examples/examples/api_simple_server.rs`. Do not add a `main`.
- No SQL driver / storage-schema knowledge here — inject spec-16 storage ports via `AppState::new` (`nebula-storage` owns adapters).
- DTOs MUST NOT embed `nebula-core`/`-storage`/`-engine`/`-credential` types (ADR-0047 §3); wrap cross-layer types (`OrgRoleDto`/`WorkspaceRoleDto`). DTOs carry only `serde_json::Value`/wrappers.
- All errors are RFC 9457 `application/problem+json` via a typed `ApiError` variant — never a new ad-hoc 500 for business failures.
- §4.5 operational honesty: an unwired capability returns honest 501/503, never a faked success. Drift between router and OpenAPI spec is a compile error (`OpenApiRouter::routes(routes!(...))`).
- Cancel/terminate signals share the durable `control_queue_repo` outbox (§12.2) — no second in-memory control channel.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design (endpoint table, CSRF route table, OAuth/idempotency env vars, durability caveats)
- ADRs: 0047 (OpenAPI), 0048/0082 (idempotency), 0049 (webhook), 0050 (W3C trace), 0072 (storage port), 0085 (OAuth IdP)
