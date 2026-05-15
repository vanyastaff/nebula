# nebula-api Refactor + Stub-Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `crates/api` into a clean, axum-idiomatic, canon-correct pure library with a single downstream `apps/server` composition binary, then complete every stubbed endpoint the engine/storage layer honors end-to-end.

**Architecture:** Phase 0 is a behavior-neutral structural refactor guarded by the existing test suite (knife §13 + OpenAPI drift + integration tests as the regression net). Phases 1–4 add cross-crate vertical slices, each independently canon-legal (no §4.5 false capability). `nebula-api` keeps its public surface (`build_app`, `AppState`, `ApiError`, `ApiConfig`, ports, `WebhookTransport`) via `lib.rs` re-exports; internal module paths break freely.

**Tech Stack:** Rust 1.95 (edition 2024), axum 0.8, tower/tower-http, tokio, utoipa/utoipa-axum (ADR-0047), thiserror, nebula-storage repos, nebula-engine control queue.

**Spec:** `docs/superpowers/specs/2026-05-15-nebula-api-refactor-design.md`

**Verification rule (Windows worktree):** Never run / report `task dev:check` or `cargo fmt --all` from this deep Claude worktree path — they break with os error 206 (memory `reference_cargo_fmt_all_winpath`). Verify per-crate: `cargo nextest run -p nebula-api`, `cargo fmt -p nebula-api`, `cargo clippy -p nebula-api -- -D warnings`.

**Commit rule:** Conventional Commits, convco-validated, scope `api` or `server`. Decompose git commands (separate `git add` and `git commit`). Co-author trailer per repo policy. On any dep add/change, stage root `Cargo.lock` too.

---

## File Structure (decomposition lock-in)

Target tree is in spec §6.1 (`nebula-api` lib) and §6.2 (`apps/server`). Each
`domain/<x>/routes.rs` returns `OpenApiRouter<AppState>` (ADR-0047 mounting).
`transport/` is the only place allowed to touch `nebula-action`/`nebula-credential`
runtime types. God-file targets: `config/`, `error/`, `middleware/idempotency/`,
`transport/webhook/*`.

---

## Phase 0 — Structural refactor (behavior-neutral)

> Gate for every Phase 0 task: the regression suite must stay green —
> `cargo nextest run -p nebula-api` including `knife`, `openapi_spec`,
> `openapi_canon_compliance`, `idempotency_e2e`, `idempotency_middleware`,
> `webhook_transport_integration`, `e2e_oauth2_flow`, `trace_w3c_smoke`,
> `rest_body_limit`, `integration_tests`. No endpoint, route path, middleware
> order, or OpenAPI spec change is permitted in Phase 0.

### Task 0.0: Establish baseline

**Files:** none (measurement only)

- [ ] **Step 1: Capture green baseline**

Run: `cargo nextest run -p nebula-api`
Expected: PASS (record the passing test count — this is the Phase 0 invariant).

- [ ] **Step 2: Capture OpenAPI spec snapshot**

Run: `cargo nextest run -p nebula-api --test openapi_spec`
Expected: PASS. Note path inventory count — Phase 0 must not change it.

### Task 0.1: Scaffold `apps/server` composition crate

**Files:**
- Create: `apps/server/Cargo.toml`
- Create: `apps/server/src/main.rs`
- Create: `apps/server/src/compose.rs`
- Create: `apps/server/src/transport.rs`
- Modify: root `Cargo.toml` workspace `members` (add `"apps/server"`)
- Modify: root `Cargo.lock` (stage after `cargo check`)
- Reference: `crates/api/src/server/mod.rs` (verbatim wiring source)

- [ ] **Step 1: Read the current composition root**

Read `crates/api/src/server/mod.rs`, `crates/api/src/bin/nebula-server.rs`,
`crates/api/src/bin/nebula-webhook.rs`, `crates/api/src/bin/nebula-realtime.rs`.
Inventory every concrete impl wired (repos, idempotency store, webhook
bootstrap, signal handling, env reads). This wiring is ported **verbatim**
first, refactored second.

- [ ] **Step 2: Add workspace member**

Add `"apps/server"` to root `Cargo.toml` `members` (alpha-adjacent to `crates/api`).

- [ ] **Step 3: Write `apps/server/Cargo.toml`**

```toml
[package]
name = "nebula-server"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
publish = false

[[bin]]
name = "nebula-server"
path = "src/main.rs"

[dependencies]
nebula-api = { path = "../../crates/api" }
nebula-storage = { path = "../../crates/storage", features = ["credential-in-memory"] }
nebula-engine = { path = "../../crates/engine" }
nebula-credential = { path = "../../crates/credential" }
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "signal", "net", "time"] }
tracing = { workspace = true }
tracing-subscriber = { workspace = true, features = ["env-filter", "fmt"] }
anyhow = { workspace = true }
clap = { workspace = true, features = ["derive", "env"] }

[features]
postgres = ["nebula-api/postgres", "nebula-storage/postgres"]
```
(If `clap`/`anyhow` are not in `[workspace.dependencies]`, add them there and stage root `Cargo.lock`.)

- [ ] **Step 4: Write `apps/server/src/transport.rs`**

```rust
//! Transport selector — one process, one entry point (api/docs/architecture.md).
use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Transport {
    /// Full REST API surface.
    Api,
    /// Webhook ingress only.
    Webhook,
    /// Realtime scaffold (/ws → 501 until §4.5 wired).
    Realtime,
    /// All transports in one process (default).
    All,
}
```

- [ ] **Step 5: Write `apps/server/src/compose.rs`**

Port `crates/api/src/server/mod.rs` wiring verbatim into a
`pub async fn compose(cfg: &nebula_api::ApiConfig) -> anyhow::Result<nebula_api::AppState>`
plus per-transport `AppState` selection. Do NOT redesign the wiring here — a
verbatim move preserves the §12.2 durable-outbox contract. (Exact body is the
current `server/mod.rs` content with `crate::` → `nebula_api::` path fixes.)

- [ ] **Step 6: Write `apps/server/src/main.rs`**

```rust
mod compose;
mod transport;

use clap::Parser;
use transport::Transport;

#[derive(Parser)]
#[command(name = "nebula-server")]
struct Cli {
    /// Ingress transport(s) to run in this process.
    #[arg(long, value_enum, env = "NEBULA_TRANSPORT", default_value = "all")]
    transport: Transport,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    nebula_api::telemetry::init_from_env();
    let cli = Cli::parse();
    let cfg = nebula_api::ApiConfig::from_env()?;
    let state = compose::compose(&cfg).await?;
    let app = nebula_api::build_app_for(&cli.transport_kind(), state, &cfg);
    nebula_api::app::serve(app, cfg.bind_address).await?;
    Ok(())
}
```
(If `nebula_api::build_app_for` / `telemetry::init_from_env` do not yet exist, Task 0.9 adds the thin lib seam; for Step 6 use the existing `build_app` + a `match cli.transport` until 0.9 lands the selector.)

- [ ] **Step 7: Verify it compiles**

Run: `cargo check -p nebula-server`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add apps/server Cargo.toml Cargo.lock
git commit -m "feat(server): scaffold apps/server composition crate (verbatim wiring port)"
```

### Task 0.2: Make `nebula-api` a pure library

**Files:**
- Delete: `crates/api/src/bin/` (3 files)
- Modify: `crates/api/Cargo.toml` (remove 3 `[[bin]]` blocks)
- Modify: `crates/api/src/server/mod.rs` → keep only lib-usable helpers (`serve`, shutdown), move composition to `apps/server`
- Modify: `deny.toml` if bin-only deps become unused

- [ ] **Step 1: Remove `[[bin]]` targets**

Delete the 3 `[[bin]]` blocks from `crates/api/Cargo.toml` (lines ~14–24).

- [ ] **Step 2: Delete `src/bin/`**

Delete `crates/api/src/bin/nebula-server.rs`, `nebula-webhook.rs`, `nebula-realtime.rs`.

- [ ] **Step 3: Slim `server/mod.rs`**

Keep `serve` / `serve_with_shutdown` / `shutdown_signal` in the lib (referenced by `apps/server`). Remove now-dead composition that moved to `apps/server::compose`.

- [ ] **Step 4: Verify lib + server build**

Run: `cargo check -p nebula-api`
Run: `cargo check -p nebula-server`
Expected: PASS both.

- [ ] **Step 5: Regression gate**

Run: `cargo nextest run -p nebula-api`
Expected: PASS (same count as Task 0.0).

- [ ] **Step 6: Commit**

```bash
git add crates/api Cargo.lock
git commit -m "refactor(api)!: drop in-crate binaries — nebula-api is a pure library"
```

### Task 0.3: Relocate the runnable example

**Files:**
- Move: `crates/api/examples/simple_server.rs` → `examples/` (root workspace member)
- Modify: `examples/Cargo.toml` (add `[[example]]` or bin), `crates/api/Cargo.toml` (drop the `sqlx`/`postgres` example wiring if now unused there)

- [ ] **Step 1: Move the file** into the root `examples/` crate, rewrite imports to `nebula_api::*` (+ `nebula_server::compose` if it used the composition root).

- [ ] **Step 2: Verify**

Run: `cargo check -p examples` (or the example's package name)
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add examples crates/api Cargo.lock
git commit -m "refactor(api): relocate simple_server example to root examples/ member"
```

### Task 0.4: Split `config.rs` (1123 → `config/`)

**Files:**
- Create: `crates/api/src/config/mod.rs`, `config/jwt.rs`, `config/errors.rs`, `config/sub.rs`, `config/env.rs`
- Delete: `crates/api/src/config.rs`

- [ ] **Step 1: Mechanical split by responsibility**

`jwt.rs` ← `JwtSecret` + validation/redaction; `errors.rs` ← `ApiConfigError`;
`sub.rs` ← `Tls/Cookie/Cors/Versioning/Pagination/Idempotency/Webhook` sub-configs;
`env.rs` ← `parse_*_env` helpers; `mod.rs` ← `ApiConfig` + `from_env` + `for_test` +
`pub use` re-exports so external paths via `nebula_api::config::*` and `nebula_api::{ApiConfig, JwtSecret}` are unchanged. Move the 17 inline `#[cfg(test)]` tests next to the type each exercises.

- [ ] **Step 2: Verify + regression**

Run: `cargo nextest run -p nebula-api`
Expected: PASS (same count). `cargo clippy -p nebula-api -- -D warnings` clean.

- [ ] **Step 3: Commit**

```bash
git add crates/api/src/config* && git commit -m "refactor(api): split config.rs into config/ by responsibility"
```

### Task 0.5: Split `errors.rs` (759 → `error/`)

**Files:**
- Create: `crates/api/src/error/mod.rs`, `error/problem.rs`, `error/classify.rs`
- Delete: `crates/api/src/errors.rs`
- Modify: `crates/api/src/lib.rs` (`pub use error::ApiError;` keeps public path; add `#[doc(hidden)] pub use error as errors;` shim if external consumers used `nebula_api::errors::*`)

- [ ] **Step 1: Split** — `problem.rs` ← `ProblemDetails` + builder; `classify.rs` ← `ApiError ↔ NebulaError/storage/validator` From-impls + `Classify`; `mod.rs` ← `ApiError` enum (add `#[non_exhaustive]`) + `IntoResponse`. Keep RFC 9457 behavior byte-identical (the `openapi_canon_compliance` test enforces this).

- [ ] **Step 2: Verify + regression**

Run: `cargo nextest run -p nebula-api`
Expected: PASS (same count); `openapi_canon_compliance` green.

- [ ] **Step 3: Commit**

```bash
git add crates/api/src/error* crates/api/src/lib.rs && git commit -m "refactor(api): split errors.rs into error/ (problem + classify)"
```

### Task 0.6: Split `middleware/idempotency.rs` (1224 → `middleware/idempotency/`)

**Files:**
- Create: `middleware/idempotency/mod.rs`, `layer.rs`, `store.rs`, `memory.rs`, `key.rs`
- Delete: `middleware/idempotency.rs`

- [ ] **Step 1: Split** — `store.rs` ← `IdempotencyStore` trait + `*Error`; `memory.rs` ← `InMemoryIdempotencyStore` (moka); `key.rs` ← key composition + SHA-256 fingerprint + `IdempotencyKeyError`; `layer.rs` ← `IdempotencyLayer`/`Service` + metrics; `mod.rs` ← re-exports + `IdempotencyConfig`. ADR-0048 behavior unchanged.

- [ ] **Step 2: Verify + regression**

Run: `cargo nextest run -p nebula-api --test idempotency_e2e` then `cargo nextest run -p nebula-api`
Expected: PASS (same count).

- [ ] **Step 3: Commit**

```bash
git add crates/api/src/middleware && git commit -m "refactor(api): split idempotency middleware into focused modules"
```

### Task 0.7: `services/` → `transport/` (+ split `webhook/transport.rs`)

**Files:**
- Move: `crates/api/src/services/` → `crates/api/src/transport/`
- Split: `transport/webhook/transport.rs` (799) → `routing.rs · signature.rs · replay.rs · dispatch.rs` (+ existing `bootstrap.rs · events.rs · provider.rs · ratelimit.rs · key.rs`)
- Modify: every `use crate::services::…` → `use crate::transport::…`; `lib.rs` re-exports (`pub use transport::webhook::WebhookTransport;` etc. — keep public surface)

- [ ] **Step 1: Rename module** `services` → `transport` (dir move + `mod.rs` + all import sites). `services/credential.rs` (the 12 stubs) → `transport/credential.rs` unchanged (still stubs in Phase 0).

- [ ] **Step 2: Split `webhook/transport.rs`** by responsibility: `signature.rs` ← ADR-0022 signature policy; `replay.rs` ← replay-window; `routing.rs` ← `RoutingMap` dispatch; `dispatch.rs` ← `dispatch_inner` pipeline + oneshot. Public `WebhookTransport` API surface unchanged.

- [ ] **Step 3: Verify + regression**

Run: `cargo nextest run -p nebula-api --test webhook_transport_integration` then full `cargo nextest run -p nebula-api`
Expected: PASS (same count).

- [ ] **Step 4: Commit**

```bash
git add crates/api/src && git commit -m "refactor(api): services/ -> transport/; split webhook transport god-file"
```

### Task 0.8: Introduce `domain/` modules (collapse handlers/+routes/+models/)

**Files:**
- Create: `crates/api/src/domain/mod.rs`, `domain/shared.rs`
- Create per domain `crates/api/src/domain/<x>/{mod,routes,handler,dto}.rs` for: `workflow, execution, credential, catalog, auth, org, me, health, resource`
- Move: `auth/` → `domain/auth/backend/`
- Delete: `handlers/`, `routes/`, `models/` (after content migrated)

- [ ] **Step 1: Dedup `PaginationParams`** into `domain/shared.rs`; delete the copy in `handlers/workflow.rs`; point `execution` at the shared type.

- [ ] **Step 2: Per-domain co-location** — for each domain move its `handlers/<x>.rs` body → `domain/<x>/handler.rs`, `routes/<x>.rs` → `domain/<x>/routes.rs` (returns `OpenApiRouter<AppState>` via `routes!(super::handler::…)`), `models/<x>.rs` → `domain/<x>/dto.rs`. `auth/` subsystem → `domain/auth/backend/`. Do ONE domain at a time, `cargo check -p nebula-api` between each.

- [ ] **Step 3: Rewire assembly** — `domain/mod.rs` builds the merged `OpenApiRouter`; `app.rs` calls `domain::router()`; `openapi.rs` keeps `split_for_parts`.

- [ ] **Step 4: Regression gate (critical)**

Run: `cargo nextest run -p nebula-api`
Expected: PASS (same count as Task 0.0). `openapi_spec` path inventory **unchanged**. `knife` green.

- [ ] **Step 5: Commit**

```bash
git add crates/api/src && git commit -m "refactor(api)!: domain-module taxonomy — collapse handlers/routes/models per domain"
```

### Task 0.9: Public-surface re-export shim + transport selector seam

**Files:**
- Modify: `crates/api/src/lib.rs`
- Create: `crates/api/src/telemetry.rs` (move `telemetry_init.rs` content) — `pub fn init_from_env()`
- Add: `pub fn build_app_for(transport: &TransportKind, state, cfg)` thin selector used by `apps/server`

- [ ] **Step 1: Stabilize `lib.rs`** — re-export the documented public surface (`build_app`, `build_app_for`, `serve`, `AppState`, `ApiConfig`, `JwtSecret`, `ApiError`, `ProblemDetails`, ports, `WebhookTransport`, `CursorParams`, `PaginatedResponse`). Internal paths may break; the public ones may not.

- [ ] **Step 2: Add `TransportKind` + `build_app_for`** routing `Api|Webhook|Realtime|All` to the right router subset (webhook-only mounts only webhook routes, etc.). Update `apps/server::main` to use it.

- [ ] **Step 3: Verify both crates + regression**

Run: `cargo check -p nebula-server` then `cargo nextest run -p nebula-api`
Expected: PASS (same count).

- [ ] **Step 4: Commit**

```bash
git add crates/api/src apps/server/src && git commit -m "refactor(api): stable lib re-export surface + transport selector seam"
```

### Task 0.10: Phase 0 closeout — full verification

- [ ] **Step 1: Per-crate quality gate**

Run: `cargo fmt -p nebula-api` ; `cargo fmt -p nebula-server`
Run: `cargo clippy -p nebula-api -- -D warnings` ; `cargo clippy -p nebula-server -- -D warnings`
Run: `cargo nextest run -p nebula-api` ; `cargo nextest run -p nebula-server`
Run: `cargo test -p nebula-api --doc`
Expected: ALL PASS; nextest count == Task 0.0 baseline; `openapi_spec` inventory unchanged.

- [ ] **Step 2: Update docs that drifted**

Update `crates/api/README.md` source-layout section to the new tree; note pure-lib + `apps/server`. Update `docs/MATURITY.md` `nebula-api` row (structure refactor; still `frontier`; behavior unchanged). Do NOT advertise removed bins.

- [ ] **Step 3: Commit**

```bash
git add crates/api/README.md docs/MATURITY.md && git commit -m "docs(api): sync README layout + MATURITY after Phase 0 refactor"
```

---

## Phase 1 — `execution/terminate` end-to-end

**Files:** `crates/api/src/domain/execution/handler.rs` (drop stub), `routes.rs` (drop `deprecated`/501 annotation), `dto.rs`; reference `crates/api/src/domain/execution/handler.rs::cancel_execution` for the §12.2 pattern; test `crates/api/tests/knife.rs` or a new `tests/execution_terminate_e2e.rs`.

- [ ] **Step 1: Write the failing seam test** — `terminate` on a non-terminal execution transitions via `ExecutionRepo` (CAS) AND enqueues `Terminate` to `execution_control_queue` in the same logical op; a wired engine consumer drives it to terminal. Model it on the existing cancel knife step.

- [ ] **Step 2: Run it — expect FAIL** (`501`/stub).

- [ ] **Step 3: Implement** `terminate_execution` mirroring `cancel_execution`’s §12.2 path with `ControlCommand::Terminate` (engine honors via ADR-0008 A3). Remove `#[deprecated]`; OpenAPI annotation → real 202/200 schema (drop 501).

- [ ] **Step 4: Run — expect PASS**; full `cargo nextest run -p nebula-api` (count grows by the new test, knife still green).

- [ ] **Step 5: DoD** — typed `ApiError` variant for the new failure modes, tracing span on the enqueue path, MATURITY row note, OpenAPI annotation updated.

- [ ] **Step 6: Commit** `feat(api): execution/terminate end-to-end (§12.2 durable Terminate, ADR-0008 A3)`

---

## Phase 2 — `me/*` (6 handlers)

**Files:** `crates/api/src/domain/me/{handler,routes,dto}.rs`; `crates/api/src/state.rs` (`SessionStore`/me port if missing); `apps/server/src/compose.rs` (wire adapter over `nebula_storage::{UserRepo, SessionRepo}`); test `crates/api/tests/me_e2e.rs`.

- [ ] **Step 1: Failing test per endpoint** (`GET/PUT /me`, session delete, MFA status/delete, email change) against a real `UserRepo`/`SessionRepo`-backed `AppState`.

- [ ] **Step 2: Run — expect FAIL** (501).

- [ ] **Step 3: Implement** — define/confirm API-tier `SessionStore` + me ports in `state.rs`; implement the production adapter in `apps/server::compose` over `nebula_storage::{UserRepo, SessionRepo}`; handlers delegate via ports (no storage types in DTOs — ADR-0047 §3). Drop `#[deprecated]`/501.

- [ ] **Step 4: Run — expect PASS**; full regression green.

- [ ] **Step 5: DoD** — typed errors, spans, MATURITY row, OpenAPI annotations real.

- [ ] **Step 6: Commit** `feat(api): me/* end-to-end over nebula-storage User/Session repos`

---

## Phase 3 — `org/*` (9 handlers)

**Files:** `crates/api/src/domain/org/{handler,routes,dto}.rs`; `crates/api/src/state.rs` (`OrgResolver`/`WorkspaceResolver`/`MembershipStore`); `apps/server/src/compose.rs` (production adapters over `nebula_storage::{OrgRepo, WorkspaceRepo}`); `crates/api/src/middleware/tenancy.rs`/`rbac.rs` now backed by real resolvers; test `crates/api/tests/org_e2e.rs`.

- [ ] **Step 1: Failing tests** — org get, member list/get/add/remove, role get/update, roles list/get — against real `OrgRepo`/`WorkspaceRepo`-backed state; plus a tenancy-resolution test (slug→id no longer test-only).

- [ ] **Step 2: Run — expect FAIL** (501).

- [ ] **Step 3: Implement** production `OrgResolver`/`WorkspaceResolver`/`MembershipStore` adapters in `apps/server::compose` over the storage repos; CRUD handlers delegate to `OrgRepo`/`WorkspaceRepo`. Tenancy/RBAC middleware now resolves for real (was stubbed). Drop 501s.

- [ ] **Step 4: Run — expect PASS**; full regression incl. `knife` (tenant-scoped routes) green.

- [ ] **Step 5: DoD** — typed errors, spans, MATURITY row, OpenAPI annotations real.

- [ ] **Step 6: Commit** `feat(api): org/* end-to-end + production tenancy resolvers`

---

## Phase 4 — credential CRUD (`transport/credential`, ~12 fns)

**Files:** `crates/api/src/transport/credential.rs` (the 12 `ServiceUnavailable` stubs), `crates/api/src/domain/credential/{handler,routes,dto}.rs`; `apps/server/src/compose.rs` (wire `nebula_credential` store over `nebula_storage::CredentialRepo`); test `crates/api/tests/credential_crud_e2e.rs` + extend `e2e_oauth2_flow.rs`.

- [ ] **Step 1: Failing tests** — create/get/update/delete/list/test/refresh/revoke/resolve/continue/list_types/get_type against a real credential-store-backed state (OAuth ceremony already partial — MATURITY P10).

- [ ] **Step 2: Run — expect FAIL** (`ServiceUnavailable`).

- [ ] **Step 3: Implement** — replace stubs with delegation to the `nebula_credential` store over `nebula_storage::CredentialRepo`; secret material zeroized/redacted (canon §12.5 / STYLE §6); DTOs redact via `#[schema(format="password", write_only)]`. Drop `ServiceUnavailable`.

- [ ] **Step 4: Run — expect PASS**; full regression incl. `e2e_oauth2_flow` green; secret-redaction test added.

- [ ] **Step 5: DoD** — typed errors, spans, MATURITY row, OpenAPI annotations real, redaction test in place.

- [ ] **Step 6: Commit** `feat(api): credential CRUD end-to-end over nebula-storage CredentialRepo`

---

## Out of scope (stay honest 501 — canon §4.5)

- `execution/restart` — engine restart-action milestone not closed.
- `resource/{list,get}` — resource catalog milestone not closed.

Keep `#[deprecated]` + `(status = 501, …)` + ` (planned)` tag + a clear reason
string. The `openapi_canon_compliance` test must continue to enforce the stub
policy for these two.

---

## Final closeout

- [ ] All phases committed; per-crate `fmt`/`clippy -D warnings`/`nextest`/doctests green for `nebula-api` and `nebula-server`.
- [ ] `knife` (§13 steps 1–6) green; `openapi_spec` + `openapi_canon_compliance` green; OpenAPI spec now documents the completed endpoints (501s removed for shipped phases).
- [ ] `docs/MATURITY.md` `nebula-api` row truthful (engine-integration upgraded where phases landed).
- [ ] `crates/api/README.md` layout + endpoint table match reality (no removed-bin drift).
- [ ] Move work onto a persistent `bash scripts/worktree.sh new api-refactor refactor api` worktree if not already there; squash-merge to `main` per AGENTS.md.
