---
title: "feat: API M3 closure — CSRF + PG AuthBackend + OTLP end-to-end"
type: feat
status: proposed
date: 2026-05-25
origin: docs/ROADMAP.md §M3.1 (CSRF + production AuthBackend), §M3.5/§M9.2 (OTLP exporter + one-root-span)
recon:
  - docs/plans/recon/m3-api-auth-state.md  (132 citations, 11 not-found)
  - docs/plans/recon/m3-otlp-state.md      (160 citations, 4 not-found)
---

> **For agentic workers:** Use the `pi-subagents` chain pattern (scout → worker → fresh reviewer per PR). Steps use checkbox (`- [ ]`) syntax. Strict TDD evidence per `task dev:check`. **DO NOT** start PR2 (PG AuthBackend) without confirming PR1 has merged and the worker has re-rebased on `main`.

## Goal

Close the **last three open boxes of M3** (API surface) for 1.0:

1. **PR1 — CSRF enforcement** wired on auth + credential write paths, with negative-path tests (M3.1 box 2).
2. **PR2 — Production PG-backed `AuthBackend`** replacing the dev-only `InMemoryAuthBackend` (M3.1 box 1). Includes the `oauth_state` + verification-token + PAT + session + user PG repos and a pluggable email port (no SMTP transport in this PR — just the trait + dev `EchoSink` impl).
3. **PR3 — OTLP exporter end-to-end** for traces *and* metrics, with a one-root-span integration test against `task obs:up` (M3.5 final box + M9.2 close-out).

**Not in scope of this plan:**
- Production `AuthBackend` lockout + rate-limit integration tests against PG (deferred to a follow-up — the lockout column logic itself lands in PR2).
- OAuth providers loaded from operator secrets (cross-dep on `2026-05-20-credential-stabilize-sweep-plan.md` Wave 4 — Task 17). PR2 leaves `complete_oauth` on the `InMemoryAuthBackend` path; once Wave 4 lands, a small follow-up routes `start_oauth`/`complete_oauth` through `CredentialService::get::<OAuth2Credential>`.
- SMTP transport for `EmailPort` (dev `EchoSink` only; production transport is M3.1 follow-up).
- `nebula_api_auth_*` metrics namespace counters (filed as M3.1 box 6; deferred follow-up).
- ADR-0084 follow-up work (pre-expiry credential refresh).

## Architecture

### PR1 (CSRF)
The `csrf_middleware` (`crates/api/src/middleware/csrf.rs:27-79`) is already complete and exempts PAT/ApiKey via `AuthMethod::Pat | AuthMethod::ApiKey`. The gap is **wiring**: `csrf_middleware` is currently only on `/api/v1/me/*` write paths. Layer it on:
- Plane-B credential write paths (`/api/v1/orgs/*/workspaces/*/credentials/*`)
- Session-bearing `/auth/mfa/*` routes (which carry the `nebula_session` cookie but no CSRF gate today)

Decision: move `/auth/mfa/enroll` + `/auth/mfa/verify` under a sub-router that has both `auth_middleware` and `csrf_middleware` layered (in that order). Other `/auth/*` routes (signup/login/logout/forgot-password/reset-password/verify-email/oauth-*) carry no session cookie at request time — these remain CSRF-exempt by construction.

### PR2 (PG AuthBackend)
Five new spec-16-port-style PG repos under `crates/storage/src/pg/` (paralleling the existing `control_queue.rs`, `idempotency.rs`, `org.rs`, `webhook_activation.rs`, `workspace.rs` layout):
- `pg/user.rs` — `UserRepo` (Argon2id hash storage; `record_login_success`/`record_login_failure` honoring `users.failed_login_count` + `users.locked_until` already in `migrations/postgres/0001_users.sql:14-15`).
- `pg/session.rs` — `SessionRepo` (`sessions` table at `migrations/postgres/0002_user_auth.sql:21-40`).
- `pg/pat.rs` — `PatRepo` (`personal_access_tokens` table at `:42-57`).
- `pg/verification_token.rs` — `VerificationTokenRepo` (`verification_tokens` table at `:60-82`, kind ∈ {`email_verification`, `password_reset`, `mfa_challenge`}). Reusing this table for `mfa_challenges` keeps the migration cost at zero.
- `pg/oauth_state.rs` — `OAuthStateRepo` (new migration `0028_plane_a_oauth_state.sql` — Plane-A PKCE state is not in any existing table).

Trait definitions live in `crates/storage/src/repos/user.rs` (extending the existing module — currently `[NOT FOUND]` for these traits; the legacy `PgUserStore` at `crates/storage/src/postgres/identity.rs:38-130` is **not** the same surface and stays untouched in this PR).

`PgAuthBackend` (`crates/api/src/domain/auth/backend/pg.rs`) is a thin façade that delegates each of the 19 `AuthBackend` methods to the five repos + reuses the existing Argon2id (`password.rs`) / TOTP (`mfa.rs`) / SHA-256 PAT (`pat.rs`) helpers from `crates/api/src/domain/auth/backend/*` unchanged.

A new `EmailPort` trait (`crates/api/src/ports/email.rs`) decouples verification/reset email delivery from the backend. PR2 ships an `EchoSink` impl (mirrors `InMemoryAuthBackend::email_sink`); SMTP impl is a follow-up.

Composition: `apps/server/src/compose.rs` selects between `InMemoryAuthBackend` and `PgAuthBackend` based on a new `ApiConfig::auth_backend: AuthBackendKind` (parallel to `IdempotencyApiConfig::backend`), with fail-closed behavior when PG is requested but `DATABASE_URL` is absent.

### PR3 (OTLP)
Two exporter installs + one full-stack test:
- **Traces:** `crates/api/src/telemetry_init.rs:37-50` currently builds an **exporter-less** `SdkTracerProvider`. Wire an OTLP `SpanExporter` (mirroring `crates/log/src/telemetry/otel.rs:91-138`) gated on `OTEL_EXPORTER_OTLP_ENDPOINT`. Return a `TelemetryGuard` from `init_api_telemetry` so `main.rs` can hold it until shutdown.
- **Metrics:** new `crates/metrics/src/otlp.rs` adds `SdkMeterProvider` + periodic reader that pulls `MetricsRegistry::snapshot_{counters,gauges,histograms}` and pushes to OTLP. ADR-0046 §"Flat module layout" is honored — single new module, no submodule tree.
- **Test:** `crates/api/tests/otlp_one_root_span.rs` — boots a real `build_app` + engine, POSTs an execution that crosses control-queue, verifies via a collector probe (`debug` exporter or `httptest` mock) that the traces all carry the same root trace id.
- **Compose:** `deploy/docker/docker-compose.observability.yml` (referenced by `Taskfile.yml:286-289` but absent today) ships with `otelcol-contrib` + Jaeger. Plus `deploy/.env.example` for `OTEL_EXPORTER_OTLP_ENDPOINT`.

## Tech Stack

- Rust 1.95 (edition 2024, resolver 3)
- `sqlx = "0.8"` with `postgres,uuid,chrono,migrate` (already in workspace)
- `argon2 = "0.5"` (already used in `password.rs`)
- `opentelemetry-otlp = "0.31.1"` with `grpc-tonic,trace,metrics` (workspace pin needs the `metrics` feature added — `Cargo.toml:153`)
- `opentelemetry_sdk = "0.31.0"` with `rt-tokio,metrics` (feature add)
- `cargo nextest run` for tests; `task dev:check` before each commit
- `convco` validates commit messages (per `CLAUDE.md` Agent Git Workflow)

## Cross-deps (DO NOT DUPLICATE)

| Dep | Status | Handling |
|----|----|----|
| `2026-05-20-credential-stabilize-sweep-plan.md` Wave 4 (Task 17 wires `nebula-api → CredentialService`) | in flight | PR2 does **not** route OAuth through `CredentialService`. PR2 just persists `oauth_state` + leaves `complete_oauth` calling the (still-in-memory) provider exchange. A separate ~150-LOC follow-up replaces `start_oauth`/`complete_oauth` with `CredentialService` calls after Wave 4 lands. |
| ADR-0046 (metrics/telemetry merged, OTLP deferred to M9.2) | done | PR3 honors `flat module layout` — single `otlp.rs` in `crates/metrics/src/`. |
| ADR-0050 (W3C trace context propagation binary contract) | done | PR3 extends `init_api_telemetry` rather than redesigning it. |
| M14.2 (`ExecutionEvent` eventbus migration) | **already done in code** (`crates/engine/src/engine.rs:65,241-957`) — ROADMAP entry is stale | No action; PR3 just notes the discrepancy in commit message and ROADMAP entry can be checked off as part of the M9.2 closure note. |

## Worktree + branch plan

Three chained branches, each from `origin/main`. Squash-merge each PR before starting the next so the next worker rebases onto a clean main.

| PR | Branch | Slug |
|----|----|----|
| PR1 — CSRF wiring | `feat/api-csrf-enforce` | `csrf-enforce` |
| PR2 — PG AuthBackend | `feat/api-pg-auth-backend` | `pg-auth-backend` |
| PR3 — OTLP closure | `feat/api-otlp-e2e` | `otlp-e2e` |

Creation pattern (per `CLAUDE.md` + `scripts/worktree.sh new <name> <type> <scope>`):

```bash
bash scripts/worktree.sh new csrf-enforce feat api
```

---

## Pre-flight (run once before PR1)

### Task 0: Verify clean base

- [ ] **Step 1: Fetch latest main**

```bash
git fetch origin main
```

- [ ] **Step 2: Verify `task dev:check` green on `main`**

```bash
cd C:/Users/vanya/RustroverProjects/nebula
task dev:check
```

Expected: exits 0. If fmt fails on Windows due to deep worktree paths, run `cargo fmt -p <crate> -- --check` per touched crate (memory: `cargo_fmt_all_winpath`).

- [ ] **Step 3: Read recon docs**

The two recon files under `docs/plans/recon/` are the authoritative source of `file:line` citations for this plan. Open them in any worker session before starting.

---

## PR1 — CSRF enforcement wiring (~140 LOC code + ~60 LOC tests + docs)

**Branch:** `feat/api-csrf-enforce`
**Worker:** single `worker` subagent (forked context), reviewer is a fresh `reviewer` subagent on the final diff.
**Estimated LOC:** ~250 (well under review budget; single small PR).

### Wave 1 — Wire `csrf_middleware` on credential write paths

**Files:**
- `crates/api/src/domain/mod.rs` (lines 73, 112-126) — currently mounts `credential_routes` without CSRF; add `.layer(middleware::from_fn(csrf_middleware))` after the auth layer.

- [ ] **Step 1: Layer CSRF on `credential_routes`**

The middleware order matters: `auth_middleware` must precede `csrf_middleware` because the latter reads `AuthContext` from extensions (`crates/api/src/middleware/csrf.rs:43-50`).

Reference: existing wiring of CSRF on `me_routes` — find it via `rg "csrf_middleware" crates/api/src/domain/`.

- [ ] **Step 2: Add negative-path tests in `seam_credential_write_path_validation.rs`**

Add two cases to `crates/api/tests/seam_credential_write_path_validation.rs`:
- `creates_returns_403_when_csrf_header_missing`
- `creates_returns_403_when_csrf_header_mismatches_cookie`

Reference: existing CSRF helper in `crates/api/tests/me_e2e.rs:46-66`.

### Wave 2 — Wire `csrf_middleware` on `/auth/mfa/*`

**Files:**
- `crates/api/src/domain/auth/routes.rs:9-21` — currently a flat router; split MFA enroll + verify into a sub-group with auth + CSRF layered.
- `crates/api/src/domain/mod.rs:73` — replace `let auth_routes = auth::routes::router();` with the sub-grouped router (e.g. `auth::routes::router_with_session_subgroup(...)` if helpers are added).

- [ ] **Step 3: Refactor `/auth/mfa/*` under a CSRF-gated sub-router**

Note: `mfa_verify` has dual use — confirms enrollment **and** completes second-step login (`handler.rs:339`). The login-second-step path uses the `mfa_challenge_token` body field and **does not** carry a session cookie yet, so CSRF cannot be enforced on that path. Either split the handler into two endpoints (`/auth/mfa/verify` for enrollment confirm requires session + CSRF; `/auth/login/mfa` for second-step login is cookie-less and CSRF-exempt) **or** branch inside the handler. **Recommendation: split — clearer surface.**

- [ ] **Step 4: Add MFA CSRF negative-path test**

New `crates/api/tests/auth_mfa_csrf.rs`:
- `mfa_enroll_returns_403_when_csrf_header_missing_with_session`
- `mfa_verify_enroll_path_returns_403_when_csrf_header_missing`
- `mfa_verify_login_second_step_succeeds_without_csrf_header` (cookie-less path).

### Wave 3 — Docs

- [ ] **Step 5: Document CSRF policy in `crates/api/README.md`**

New section "CSRF" under "Authentication" describing:
- Double-submit cookie pattern (cookie name, attributes from `CookieConfig`)
- Which routes enforce CSRF (table)
- PAT/ApiKey exemption rationale
- `X-CSRF-Token` header contract

### PR1 Acceptance criteria

- [ ] `task dev:check` green
- [ ] New tests pass
- [ ] No regression in `crates/api/tests/me_e2e.rs` or `crates/api/tests/seam_credential_write_path_validation.rs`
- [ ] `cargo test -p nebula-api --tests auth_mfa_csrf` exists and passes
- [ ] Fresh `reviewer` subagent audits the diff — confirms middleware order (auth → CSRF), no overshoot onto cookie-less endpoints

### PR1 Commit message

```
feat(api): enforce CSRF on credential write paths and /auth/mfa/*

Layer csrf_middleware (existing, complete) on:
- /api/v1/orgs/*/workspaces/*/credentials/* (write methods)
- /auth/mfa/enroll
- /auth/mfa/verify (enrollment-confirm subset; login-second-step
  split into /auth/login/mfa which is cookie-less and CSRF-exempt)

Closes M3.1 box 2 (CSRF enforcement) per docs/ROADMAP.md.

Tests:
- crates/api/tests/seam_credential_write_path_validation.rs:
  + missing-header + cookie/header-mismatch negatives
- crates/api/tests/auth_mfa_csrf.rs (new): 3 cases

Docs: crates/api/README.md — new CSRF section.
```

---

## PR2 — Production PG-backed `AuthBackend` (~1900 LOC, **staged in 3 commits**)

**Branch:** `feat/api-pg-auth-backend`
**Worker:** single `worker` subagent (forked context), with an `oracle` consult before commit 3 (PgAuthBackend façade) to lock the design once trait surface is stable. Final reviewer is a fresh `reviewer` subagent.
**Estimated LOC:** ~1900 (HARD upper bound 2300 — flag if breaching).

> **Review burnout watch:** at 1900 LOC this is the single biggest PR of the slice but stays under the 4000-line session budget. The 3-commit split is review-friendly without leaving a half-broken state on `main` — commits 1 + 2 alone don't reach `apps/server` so the dev backend stays default until commit 3.

### Commit 1 — Storage: PG repos + traits + migration (~860 LOC)

**Files (new traits in `crates/storage/src/repos/user.rs`):**
- `pub trait UserRepo: Send + Sync` (8 methods)
- `pub trait SessionRepo: Send + Sync` (5 methods)
- `pub trait PatRepo: Send + Sync` (5 methods)
- `pub trait VerificationTokenRepo: Send + Sync` (5 methods)
- `pub trait OAuthStateRepo: Send + Sync` (4 methods)

**Files (new PG impls under `crates/storage/src/pg/`):**
- `pg/user.rs` (~250 LOC) — Argon2id hash stored as TEXT; `record_login_success` clears counters; `record_login_failure` increments + optionally sets `locked_until`.
- `pg/session.rs` (~160 LOC)
- `pg/pat.rs` (~180 LOC) — uses the existing `idx_pat_hash` partial index.
- `pg/verification_token.rs` (~150 LOC) — `kind` enum covers `email_verification`, `password_reset`, `mfa_challenge`.
- `pg/oauth_state.rs` (~80 LOC) — new table.

**Files (migration):**
- `crates/storage/migrations/postgres/0028_plane_a_oauth_state.sql` (~25 LOC) — `oauth_states` table: `state TEXT PRIMARY KEY`, `provider TEXT NOT NULL`, `code_verifier TEXT NOT NULL`, `redirect_uri TEXT`, `created_at TIMESTAMPTZ NOT NULL DEFAULT now()`, `expires_at TIMESTAMPTZ NOT NULL`, `consumed_at TIMESTAMPTZ`. Plus a single index on `expires_at` for cleanup.
- `crates/storage/migrations/sqlite/0028_plane_a_oauth_state.sql` — SQLite parity (mirrors PG; uses TEXT timestamps per project convention).

**Files (storage-side tests):**
- `crates/storage/tests/pg_identity_repos.rs` (~300 LOC) — DATABASE_URL-gated; one round-trip per trait. Follow the `crates/storage/src/pg/*.rs` test convention (`crates/storage/src/pg/mod.rs:13-15`).

- [ ] **Step 1: Land migration**
- [ ] **Step 2: Add traits in `repos/user.rs`** (re-export from `crates/storage/src/lib.rs`)
- [ ] **Step 3: Implement PG repos**
- [ ] **Step 4: Storage round-trip tests**

**Commit message:**
```
feat(storage): add UserRepo/SessionRepo/PatRepo/VerificationTokenRepo/OAuthStateRepo PG impls

Adds spec-16-port-style PG repos under crates/storage/src/pg/
backing the production AuthBackend in nebula-api. Adds
0028_plane_a_oauth_state migration (PG + SQLite parity) — the
Plane-A PKCE state was the only auth surface without a table.

Traits live in crates/storage/src/repos/user.rs. The legacy
PgUserStore at crates/storage/src/postgres/identity.rs is
untouched in this PR.

Tests: DATABASE_URL-gated round-trips in
crates/storage/tests/pg_identity_repos.rs.

Refs M3.1 (production AuthBackend).
```

### Commit 2 — API: `EmailPort` trait + `ApiConfig` backend selector (~110 LOC)

**Files:**
- `crates/api/src/ports/email.rs` (new, ~30 LOC) — `EmailPort` trait with one async method `send(&self, EmailRequest) -> EmailResult<()>` and an `EchoSink` impl for dev/tests.
- `crates/api/src/state.rs` — add `pub email_port: Option<Arc<dyn EmailPort>>` slot (~10 LOC) and a `with_email_port` builder (~10 LOC).
- `crates/api/src/config/sub.rs` + `crates/api/src/config/env.rs` — `AuthBackendKind: Memory | Postgres` enum + env binding `NEBULA_API_AUTH_BACKEND` (~40 LOC).
- `crates/api/src/domain/auth/backend/in_memory.rs` — replace inline `email_sink` with a constructor that accepts an `Arc<dyn EmailPort>` (default = `EchoSink`). Backward-compatible (~20 LOC).

- [ ] **Step 5: Land `EmailPort` + `EchoSink`**
- [ ] **Step 6: Add `AuthBackendKind` config + env**
- [ ] **Step 7: Refactor `InMemoryAuthBackend` to consume `EmailPort` (no behavior change)**

**Commit message:**
```
feat(api): EmailPort trait + AuthBackendKind config selector

Decouples verification/reset email delivery from the auth backend
so PgAuthBackend can ship without inheriting an in-memory email
sink. The dev EchoSink mirrors current InMemoryAuthBackend::email_sink
behavior; an SMTP impl is a follow-up.

AuthBackendKind { Memory, Postgres } config (parallel to
IdempotencyApiConfig::backend) drives composition root selection
in the next commit.

Refs M3.1 (production AuthBackend).
```

### Commit 3 — API: `PgAuthBackend` + composition (~900 LOC)

**Files:**
- `crates/api/src/domain/auth/backend/pg.rs` (new, ~700 LOC) — `PgAuthBackend` struct with the five repo Arcs + `EmailPort` Arc + reuses existing `password.rs`/`mfa.rs`/`pat.rs` helpers; implements all 19 `AuthBackend` methods.
- `crates/api/src/domain/auth/backend/mod.rs:46` — add `pub use pg::PgAuthBackend;`
- `apps/server/src/compose.rs:140-209` — branch on `config.auth_backend` (mirrors `build_idempotency_store` pattern at `apps/server/src/compose.rs:230+`); fail-closed when `Postgres` selected but `DATABASE_URL` is `None`.
- `crates/api/tests/auth_pg_e2e.rs` (new, ~250 LOC) — DATABASE_URL-gated; covers signup → verify-email → login → MFA enroll/verify → PAT mint/use/revoke → forgot/reset → OAuth state persistence (synthetic provider).

- [ ] **Step 8: Implement `PgAuthBackend` (delegate per method to repos)**
- [ ] **Step 9: Wire composition selector**
- [ ] **Step 10: E2E tests over PG**
- [ ] **Step 11: `oracle` consult on the final design before commit** — flag any leaks of in-memory invariants into PG semantics (e.g. transactional boundaries on signup → email send).

**Commit message:**
```
feat(api): PgAuthBackend impl + composition selector

Production AuthBackend impl over the spec-16-style PG repos in
nebula-storage. Selected via NEBULA_API_AUTH_BACKEND=postgres at
the composition root; fails closed when DATABASE_URL is absent.

PgAuthBackend reuses the existing Argon2id / TOTP / SHA-256 PAT
helpers (crates/api/src/domain/auth/backend/{password,mfa,pat}.rs)
unchanged.

Closes M3.1 box 1 (production AuthBackend) per docs/ROADMAP.md.

OAuth provider configs from operator secrets (cross-dep on
2026-05-20-credential-stabilize-sweep-plan.md Wave 4) remain a
follow-up — start_oauth/complete_oauth still route through the
existing provider config path.

Tests: crates/api/tests/auth_pg_e2e.rs — DATABASE_URL-gated full
lifecycle covering signup → MFA → PAT → password reset → OAuth
state persistence.
```

### PR2 Acceptance criteria

- [ ] `task dev:check` green on each commit (no half-broken intermediate)
- [ ] DATABASE_URL-gated tests pass against a real `task db:up` Postgres
- [ ] `apps/server` defaults to `InMemoryAuthBackend` when `NEBULA_API_AUTH_BACKEND` is unset (no surprise prod switch)
- [ ] Fresh `reviewer` audits the final diff — flags any new dashmap leaks, missing `tracing::instrument` spans on PG path-equivalents, and AAD/encryption surface drift
- [ ] `oracle` consult sign-off on transactional boundary decisions (signup commit ordering, lockout race window)

---

## PR3 — OTLP exporter end-to-end + one-root-span test (~700 LOC)

**Branch:** `feat/api-otlp-e2e`
**Worker:** single `worker` subagent (forked context). Fresh `reviewer` on diff.
**Estimated LOC:** ~700.

### Wave 1 — Traces OTLP exporter

**Files:**
- `crates/api/src/telemetry_init.rs:37-50` — extend `init_api_telemetry` with an env-gated OTLP `SpanExporter` install + return a `TelemetryGuard`.
- `apps/server/src/main.rs:19` — accept the guard, hold it until graceful shutdown.
- `crates/api/Cargo.toml:48-50` — add `opentelemetry-otlp = { workspace = true }`.

- [ ] **Step 1: Add resolver + exporter + provider shutdown guard** mirroring `crates/log/src/telemetry/otel.rs:91-138` and `:177-183`.

### Wave 2 — Metrics OTLP exporter

**Files:**
- `crates/metrics/src/otlp.rs` (new, ~350 LOC) — `OtlpMetricsExporter` with periodic reader pulling `MetricsRegistry::snapshot_{counters,gauges,histograms}` and pushing via `opentelemetry-otlp`. Honor `LabelAllowlist` cardinality budget.
- `crates/metrics/src/lib.rs:30-58` — `pub mod otlp;` (ADR-0046 flat layout preserved).
- `crates/metrics/Cargo.toml:14-21` — add `opentelemetry-otlp` + sdk metrics features.
- `Cargo.toml:152-155` — extend workspace `opentelemetry-otlp` features to include `metrics`.
- Init wiring point: in `crates/api/src/telemetry_init.rs` (alongside the traces exporter) — receives the same `Arc<MetricsRegistry>` already on `AppState`.

- [ ] **Step 2: Implement metrics exporter + tests for cardinality budget enforcement**

### Wave 3 — Compose stack + e2e test

**Files:**
- `deploy/docker/docker-compose.observability.yml` (new, ~60 LOC) — `otelcol-contrib` + Jaeger.
- `deploy/.env.example` (new) — `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317` etc.
- `crates/api/tests/otlp_one_root_span.rs` (new, ~250 LOC) — boots `build_app` + engine over in-memory storage, points OTLP at an in-process collector mock (e.g. `httptest::ServerBuilder`), POSTs an execution, drains, asserts: (1) one root trace id across all spans, (2) inbound `traceparent` ID is preserved, (3) metrics export hits the collector at least once.

- [ ] **Step 3: Land compose + env**
- [ ] **Step 4: Land integration test (gated by `OTEL_E2E_TEST=1` to keep CI fast unless explicitly run)**

### PR3 Acceptance criteria

- [ ] `task dev:check` green
- [ ] `task obs:up` succeeds (smoke-test by hand once locally; document in README)
- [ ] `OTEL_E2E_TEST=1 cargo test -p nebula-api --test otlp_one_root_span` passes against the mock collector
- [ ] ROADMAP §M3.5 final box + §M9.2 OTLP exporter checkboxes flipped to `[x]` in the closing commit
- [ ] Fresh `reviewer` audits diff — checks shutdown guard discipline, no double-install of `OpenTelemetryLayer`, label-allowlist honored in metrics export

### PR3 Commit message

```
feat(api,metrics): OTLP exporter end-to-end + one-root-span test

Wires opentelemetry-otlp into init_api_telemetry (traces) and
nebula-metrics::otlp (metrics) with periodic reader pulling the
existing MetricsRegistry snapshot path (ADR-0046 flat layout).

Ships deploy/docker/docker-compose.observability.yml + .env.example
(the file referenced by Taskfile.yml `obs:up` since #707 but never
committed).

Adds the full-stack integration test asserting a single root trace
id flows API → control queue → engine → action.

Closes M3.5 final box (full-stack one-root-span test) + M9.2
(OTLP exporter verification) per docs/ROADMAP.md.

Note: M14.2 ExecutionEvent eventbus migration is already done in
code (crates/engine/src/engine.rs:65,241-957) — ROADMAP entry
should be ticked off as part of this closure.
```

---

## Subagent matrix

| Step | Agent | Why |
|----|----|----|
| PR1 worker | `worker` (forked ctx) | One writer thread; ~250 LOC well within single-worker budget |
| PR1 review | `reviewer` (fresh ctx) | Adversarial audit of middleware order + cookie-less path handling |
| PR2 commit 1 worker | `worker` (forked ctx) | Storage repos + migration; isolated from API |
| PR2 commit 2 worker | same `worker` continuing | Trivial config + port plumbing |
| PR2 commit 3 worker | same `worker` continuing | PgAuthBackend façade; oracle consulted before final commit |
| PR2 oracle | `oracle` (fork ctx) | Lock design before commit 3 — flag transactional/race issues |
| PR2 review | `reviewer` (fresh ctx) | Big PR (~1900 LOC) — independent audit gate |
| PR3 worker | `worker` (forked ctx) | All three waves single-threaded |
| PR3 review | `reviewer` (fresh ctx) | Verify shutdown discipline + double-install guards |

**Single-writer discipline:** never run two workers in parallel on overlapping crates. PR1 → PR2 → PR3 strictly sequential; the next worker starts only after the previous PR squash-merges to `main` and the working branch rebases.

## Verification gate (full-slice)

After PR3 squash-merges, before declaring M3 closed:

- [ ] `task dev:check` clean on `main`
- [ ] `cargo deny check` green (no new wrapper edges)
- [ ] `cargo doc --no-deps --workspace` warning-free (M10.5 gate)
- [ ] ROADMAP `[x]` updates for M3.1 boxes 1-2, M3.5 final box, M9.2 OTLP boxes, M14.2 (already done; note in the closure)
- [ ] MATURITY.md API row reflects post-M3-closure state (status remains `partial` until §M3.6 shift-left validation audit ships separately)

## Risks + mitigations

| Risk | Mitigation |
|----|----|
| PR2 transactional boundary mistakes (signup commits before email queued; reset-token race) | `oracle` consult before commit 3 + reviewer checks each method has explicit comment on commit ordering |
| OTLP exporter double-install (one in `nebula-api`, one in `nebula-log` if both wired) | PR3 enforces a single install site — `init_api_telemetry` is sole entry; `nebula-log` install path stays unused by `apps/server` |
| Windows fmt path issues on deep worktree | Per-crate `cargo fmt -p` fallback documented in pre-flight |
| Cross-dep with credential plan Wave 4 lands first and changes `AppState` builder | PR2 keeps `start_oauth`/`complete_oauth` on the existing provider config path; rebase cost is low because we don't touch `credential_service` slot |
| Reviewer burnout on PR2 (1900 LOC) | 3-commit split makes the review chunkable; reviewer prompt explicitly suggests one commit at a time |

## Rollout

1. Land PR1. Merge. Update branch protections if needed.
2. Land PR2 commit 1 → commit 2 → commit 3 sequentially, with `task dev:check` between each. Squash-merge as one PR.
3. Land PR3. Squash-merge.
4. Update `docs/ROADMAP.md` checkboxes in a small `docs(roadmap):` commit on `main`.
5. Save phase artifacts to Engram with topic keys `sdd/m3-closure/{proposal,plan,apply-progress,verify-report}`.

## Memory + audit

- Plan saved to: `docs/plans/2026-05-25-002-feat-api-m3-closure-plan.md`
- Recon saved to: `docs/plans/recon/m3-api-auth-state.md`, `docs/plans/recon/m3-otlp-state.md`
- Engram topic key for this plan: `sdd/m3-closure/plan`
- Cross-link in ROADMAP `Next step` section (after PR3 merges)
