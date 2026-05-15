---
name: nebula-api full refactor + stub completion — design
description: Expert-panel design for the structural refactor of nebula-api (lib + apps/server, domain modules, god-file splits) and the §4.5-honest sequenced completion of its stubbed endpoints.
status: draft
date: 2026-05-15
related: [PRODUCT_CANON.md §12.1/§12.2/§12.4/§12.7/§4.5/§13, STYLE.md §1/§4/§5, ADR-0047, ADR-0048, ADR-0049, ADR-0050, ADR-0020, ADR-0055, crates/api/docs/REST_API_AXUM_GUIDE.md]
---

# nebula-api — full refactor + stub completion

## 1. Goal

Bring `crates/api` (the Nebula HTTP gateway) to a clean, axum/tokio/hyper-idiomatic,
canon-correct state: restructure into self-contained domain modules, split god
files, make the crate a pure library with a single downstream composition-root
binary, and **complete every stubbed endpoint that the engine/storage layer can
honor end-to-end** — sequenced so no step ships a false capability (canon §4.5).

Breaking changes are explicitly allowed and expected.

## 2. Current state (empirical)

`crates/api`: ~17.8k LoC in `src/`, maturity `frontier`, knife steps 3+5 stable.
Cross-crate boundaries are **clean** (ports injected as `Arc<dyn …>`, no
nebula-core/storage/engine/credential types leak into DTOs — ADR-0047 §3 PASS,
no sibling reach-arounds outside `nebula-eventbus`, deny.toml `[wrappers]`
compliant). The defects are **internal**:

- **God files**: `middleware/idempotency.rs` (1224 — trait+memory impl+layer+metrics+tests),
  `config.rs` (1123 — JwtSecret+errors+7 sub-configs+from_env+tests),
  `handlers/workflow.rs` (832), `services/webhook/transport.rs` (799 —
  routing+signature+replay+ratelimit+dispatch tangled), `errors.rs` (759),
  `handlers/execution.rs` (752), `auth/in_memory.rs` (714, ~60% tests).
  Canon §12.7 ("no god files") is violated.
- **`handlers/` + `routes/` + `models/` triple-spread**: one endpoint is split
  across three files; route files are 12–40 lines that only re-export handlers.
- **`PaginationParams` duplicated** in `handlers/workflow.rs` and consumed by
  `handlers/execution.rs` while `models/pagination.rs` already exists.
- **`services/` is a misnomer**: `services/credential.rs` is 12 stub fns
  returning `ServiceUnavailable`; `services/{oauth,webhook}` are protocol
  transports, not business services. The name implies an empty business layer.
- **Honest 501 stubs**: `me/*` (6), `org/*` (9), `execution/{terminate,restart}`
  (2), `resource/{list,get}` (1–2), `services/credential` (12 fns). These are
  `#[deprecated]` + 501 per ADR-0047 stub policy — correct today, but the goal
  is to complete the ones the stack can honor.
- 3 binaries in `crates/api/src/bin/` (`nebula-server`, `nebula-webhook`,
  `nebula-realtime`); `build_app` already takes a pre-built `AppState`
  (composition logic is already injection-based, not in-crate-wired).

**Downstream readiness** (decisive for §4.5):

- `nebula-storage` already ships repo traits + pg impls: `OrgRepo`,
  `WorkspaceRepo`, `UserRepo`, `SessionRepo`, `CredentialRepo`, `ResourceRepo`
  (`crates/storage/src/repos/*`, `crates/storage/src/pg/{org,workspace}.rs`).
  SQL schema exists (migrations `0001_users`…`0005_memberships`, `0008`, `0009`,
  `0017`).
- API-tier ports `OrgResolver`/`WorkspaceResolver`/`MembershipStore`/`SessionStore`
  (defined in `crates/api/src/state.rs`) have **only test impls**
  (`tests/common/mod.rs`) — no production adapter wires them to the storage
  repos. This is why me/org are stubbed: the resolution adapters are missing,
  not the storage layer.
- Engine honors `Terminate` dispatch (ADR-0008 A3, MATURITY engine row).
- Engine `restart` and `resource` catalog have no end-to-end path → must stay
  honest 501 (shipping them = §4.5 false capability).
- `apps/` workspace member does **not** exist.

## 3. Expert panel & dialectic

Simulated panel, seated with radical critics (no echo-chamber validation):

| Seat | Mandate |
|---|---|
| Axum/Tower core | `FromRequestParts`, `tower::Layer`, `OpenApiRouter` mounting, extractor order |
| Tokio/Hyper runtime | graceful shutdown, backpressure, load-shed, `axum::serve` |
| Rust 1.95 language eng. | AFIT vs `#[async_trait]` seam, `let-else`, typestate, `#[non_exhaustive]` |
| API contract / RFC 9457 | ProblemDetails, OpenAPI drift, versioning |
| Security (adversarial) | OWASP, confused-deputy, secret handling, CSRF/idempotency |
| Nebula canon enforcer | §12.1 thin API, §12.2 durable outbox, §4.5 honesty |
| 🔴 Radical critic A | "kill the handlers/+routes/ dualism; the crate does too much" |
| 🔴 Radical critic B | "nebula-api must ship zero binaries; it is a library" |

**Resolved disputes:**

- *Critic A vs Axum expert* — handlers/routes/models triple-spread vs ADR-0047
  requiring `routes/<group>` to own `OpenApiRouter::routes(routes!(…))`.
  → **Resolved:** per-domain co-location. Each `domain/<x>/routes.rs` returns
  `OpenApiRouter<AppState>` (ADR-0047 mounting preserved structurally) while
  handler + DTO live in the same domain module.
- *Critic B vs status quo* — 3 in-crate binaries vs library-first (§2/ADR-0020),
  "one process = one entry point" (`api/docs/architecture.md`), facade (ADR-0055).
  → **Resolved:** pure library + new `apps/server` with one binary.
- *Clean-architecture (axum-enterprise doc) vs canon §12.1* — a
  presentation/application/infrastructure split inside the crate.
  → **Rejected** by canon enforcer: business logic lives in `nebula-engine`
  behind ports; an in-API "application/use-case" layer is empty ceremony =
  "framework before product" (§14). The generic Handler→Service→Repository
  advice in `REST_API_AXUM_GUIDE.md` maps, in Nebula, to **Handler→Port**.
- *Idiom currency* — agent flagged `#[async_trait]` on ports as debt.
  → **Overruled:** STYLE §1 mandates `#[async_trait]` for `Arc<dyn …>` traits
  until `async_fn_in_dyn_trait` stabilizes. Keep it; it is correct.

## 4. Locked decisions

| # | Decision | Rationale |
|---|---|---|
| D1 | Full structural refactor, breaking allowed | user directive |
| D2 | **Complete every stub the stack can honor**, the rest stay honest 501 | user "all stubs"; canon §4.5 caveat |
| D3 | `nebula-api` → **pure library**; new `apps/server` crate, **one** binary `nebula-server` with `--transport=api\|webhook\|realtime\|all` | §2/ADR-0020/ADR-0055 + architecture.md "one entry point" |
| D4 | **Domain-module** taxonomy; reject clean-arch layers | canon §12.1, STYLE §0, §12.7 |
| D5 | `services/` → `transport/` (protocol subsystems, not business) | removes false "empty service layer" signal |
| D6 | Split every god file by responsibility | canon §12.7 |
| D7 | Single program: Phase 0 refactor → sequenced stub-completion phases | user "всё в одной работе"; §4.5 per-step legality |

## 5. Binding constraints (must hold every phase)

- **ADR-0047** — utoipa `OpenApiRouter::routes(routes!(handler))` is the only
  spec-mounting path; drift = compile error. DTOs MUST NOT embed
  nebula-core/storage/engine/credential types (wrap in domain `dto.rs`). Stub
  policy: `#[deprecated]` + `(status = 501, …)` + ` (planned)` tag.
- **canon §12.1** — no SQL/storage-schema knowledge beyond declared ports.
- **canon §12.2** — every run/cancel/terminate signal durable + engine-consumable;
  written to `execution_control_queue` in the **same logical op** as the state
  transition; no second control channel.
- **canon §12.4 / STYLE §4** — RFC 9457 `application/problem+json`; every
  failure → typed `ApiError` variant + explicit status; no ad-hoc 500;
  `nebula-error::Classify` at the boundary.
- **canon §12.5 / STYLE §6** — secrets zeroized, redacted `Debug`, never in
  logs/errors/metrics labels.
- **canon §4.5 / §13** — knife seams `domain/workflow/handler.rs::{create,activate}`,
  `domain/execution/handler.rs::{start,cancel}` must stay green; no false
  capability.
- **STYLE §1/§5** — `#[async_trait]` for dyn ports; native AFIT only for
  generic-only traits; `#[non_exhaustive]` on growable public enums; newtypes
  for ids; builder for `AppState`/configs.
- **ADR-0048** idempotency hybrid backend; **ADR-0049** webhook single pipe;
  **ADR-0050** W3C trace through control queue — preserved, just relocated.

## 6. Target architecture

### 6.1 Crate `nebula-api` (library only)

```
crates/api/src/
  lib.rs                 public re-exports (build_app, AppState, ApiError, ports, transport)
  app.rs                 build_app: OpenApiRouter merge + split_for_parts + middleware stack + serve()
  state.rs               AppState (builder) + API-tier port traits
  config/
    mod.rs               ApiConfig assembly + from_env
    jwt.rs               JwtSecret (validation, redaction)
    errors.rs            ApiConfigError
    sub.rs               Tls/Cookie/Cors/Versioning/Pagination/Idempotency/Webhook sub-configs
    env.rs               parse_*_env helpers
  error/
    mod.rs               ApiError enum (#[non_exhaustive]) + IntoResponse
    problem.rs           ProblemDetails (RFC 9457) + builder
    classify.rs          ApiError ↔ NebulaError / storage / validator mapping
  extract/               ValidatedJson + FromRequestParts extractors
  middleware/
    mod.rs
    idempotency/         layer.rs · store.rs (trait) · memory.rs · key.rs
    auth.rs · tenancy.rs · rbac.rs · csrf.rs · rate_limit.rs
    request_id.rs · security_headers.rs · trace_w3c.rs · internal_auth.rs
  domain/
    mod.rs               assembles all domain routers into one OpenApiRouter
    shared.rs            PaginationParams, CursorParams, common DTO bits (dedup)
    workflow/            routes.rs · handler.rs · dto.rs        (§13 seam)
    execution/           routes.rs · handler.rs · dto.rs        (§13 seam)
    credential/          routes.rs · handler.rs · dto.rs · oauth.rs
    catalog/             routes.rs · handler.rs · dto.rs
    auth/                routes.rs · handler.rs · dto.rs · backend/ (Plane A: trait, in_memory, mfa, pat, session, password, oauth_state)
    org/ · me/ · health/ · resource/   (routes.rs · handler.rs · dto.rs each)
  transport/
    mod.rs
    webhook/             transport.rs split → routing.rs · signature.rs · replay.rs · dispatch.rs · bootstrap.rs · events.rs · provider.rs · ratelimit.rs · key.rs
    oauth/               flow.rs · http.rs · state.rs
  openapi.rs             OpenApiDoc + spec assembly (split_for_parts hand-off)
```

Rules: `domain/<x>/routes.rs` → `OpenApiRouter<AppState>`; `handler.rs` thin
(extract → port/transport call → DTO map → `Result<_, ApiError>`); `dto.rs`
owns wire types + `ToSchema`; no business logic; no cross-domain imports
(shared bits in `domain/shared.rs`). `transport/` is the only place allowed to
depend on `nebula-action`/`nebula-credential` runtime types (ADR-0049 bridge).

### 6.2 Crate `apps/server` (new composition root)

```
apps/server/
  Cargo.toml             depends on nebula-api + concrete storage/engine/credential backends
  src/main.rs            CLI: --transport=api|webhook|realtime|all, env, runtime builder
  src/compose.rs         wire AppState: repos, port adapters, engine, signals, observability
  src/transport.rs       Transport enum + per-transport AppState/router selection
```

`nebula-server` replaces the 3 bins. `nebula-webhook`/`nebula-realtime` become
`--transport=webhook|realtime`. `crates/api/examples/simple_server.rs` moves to
the root-level `examples/` workspace member (per house rule: runnable examples
live in root `examples/`, not per-crate) and is rewritten against the new
`apps/server::compose` API or kept as a minimal `build_app` demo.

### 6.3 Port reconciliation

`OrgResolver`/`WorkspaceResolver` (slug→id, used by `middleware/tenancy.rs`) and
`MembershipStore`/`SessionStore` (roles/session, used by `middleware/rbac.rs` &
`me/*`) stay as **API-tier resolution ports** in `state.rs`. Their **production
adapters** are implemented in `apps/server::compose` over the existing
`nebula_storage::{OrgRepo, WorkspaceRepo, UserRepo, SessionRepo}` (api→storage
is allow-listed). `org/*` CRUD handlers delegate to `nebula_storage::OrgRepo`
via `AppState`. This keeps the thin-resolver vs storage-CRUD split explicit and
removes the "redundant wrapper" smell.

## 7. §4.5-honest phase breakdown

Each phase is independently canon-legal and independently shippable.

- **Phase 0 — structural refactor (no behavior change).**
  lib/apps split; domain modules; god-file decomposition; `services/`→`transport/`;
  `PaginationParams` dedup; one binary + transport selector; example relocation.
  All current 501s stay honest. Knife (steps 1–6) stays green. OpenAPI drift
  tests stay green. **Zero endpoint semantic change.**
- **Phase 1 — `execution/terminate` end-to-end.** Enqueue `Terminate` to
  `execution_control_queue` in the same logical op as the state transition
  (§12.2), engine honors via ADR-0008 A3. Drop `#[deprecated]`/501.
- **Phase 2 — `me/*` (6).** API-port `SessionStore`/me adapters over
  `nebula_storage::{UserRepo, SessionRepo}` wired in `apps/server::compose`;
  handlers delegate via ports. Drop stubs.
- **Phase 3 — `org/*` (9).** `OrgResolver`/`WorkspaceResolver`/`MembershipStore`
  production adapters over `nebula_storage::{OrgRepo, WorkspaceRepo}`; CRUD
  handlers delegate to `OrgRepo`/`WorkspaceRepo`. Drop stubs.
- **Phase 4 — credential CRUD (`transport`/credential, ~12 fns).** Wire
  `nebula_credential` store over `nebula_storage::CredentialRepo`; OAuth ceremony
  already partial (MATURITY P10) — finish CRUD/test/refresh/revoke.
- **Stays honest 501 (out of scope until their milestone):**
  `execution/restart` (engine restart-action milestone), `resource/{list,get}`
  (resource catalog milestone). Documented with reason; not false capability.

Per-phase Definition of Done: typed `ApiError` variant + tracing span +
invariant/seam test (canon §4.6/§13, observability-as-completion); MATURITY
`nebula-api` row updated; OpenAPI annotation updated (drop `deprecated`/501 →
real schema); no §12.2/§12.7 regression.

## 8. Breaking changes

- `nebula-api` no longer provides `[[bin]]` targets (moved to `apps/server`).
- 3 binary names → 1 (`nebula-server --transport=…`); `*_BIND_ADDRESS` env
  semantics consolidated under the selector.
- Module paths change (`handlers::workflow` → `domain::workflow::handler`,
  `services::webhook` → `transport::webhook`, `errors::ApiError` →
  `error::ApiError`, `config::*` submodulised). `lib.rs` re-exports keep the
  **public** surface (`build_app`, `AppState`, `ApiError`, `ApiConfig`, ports,
  `WebhookTransport`) stable where external consumers exist; internal paths break.
- `crates/api/examples/simple_server.rs` removed (relocated to root `examples/`).
- `postgres` feature forwarding may move to `apps/server`.

No public OpenAPI contract change in Phase 0 (drift tests enforce this).
Phases 1–4 only *add* honored endpoints (remove 501s) — forward-compatible.

## 9. Error handling, data flow, observability

- Data flow unchanged conceptually: request → middleware stack (rate_limit →
  request_id → security_headers → trace_w3c → TraceLayer → compression → CORS
  → auth → tenancy → rbac → csrf → idempotency) → thin handler → port/transport
  → DTO → RFC 9457 on error. Middleware order preserved exactly (it is
  load-bearing for ADR-0048/0050); only file locations move.
- `error/` keeps one `ApiError` (`#[non_exhaustive]`), `IntoResponse`,
  `ProblemDetails`, `Classify` mapping. No new ad-hoc 500.
- Observability is DoD, not follow-up: the new `apps/server` startup, transport
  selector, and every new port adapter ship with a typed error + tracing span
  + an invariant/seam test in the same phase.

## 10. Testing strategy

- Phase 0 is a **refactor**: existing `tests/` (knife, openapi_spec,
  openapi_canon_compliance, idempotency_e2e, webhook_transport_integration,
  e2e_oauth2_flow, trace_w3c_smoke, rest_body_limit) must pass **unchanged in
  intent** (only import paths updated). They are the safety net.
- Add a transport-selector test in `apps/server`.
- Phases 1–4: each adds an end-to-end seam test proving the engine/storage
  honors the endpoint (canon §13 "integration bar", not DB-metadata-only).
- Verification per crate (`cargo nextest run -p nebula-api`,
  `cargo nextest run -p server`); **do not** report `task dev:check` green from
  this Windows worktree — `cargo fmt --all`/fmt:check break with os error 206 on
  deep Claude worktree paths; verify fmt per-crate.

## 11. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Phase 0 silently changes a route/middleware order | OpenAPI drift tests + knife as regression gate; mechanical move only |
| Module-path churn breaks downstream crates | `lib.rs` re-export shim for the documented public surface; `cargo check -p` each consumer |
| New `apps/server` re-implements composition wrong (§12.2 outbox) | port the existing `server/mod.rs` wiring verbatim first, refactor second |
| Cargo.lock conflicts on dep moves | per house rule: on dep add/change stage root `Cargo.lock`; rebase conflict → `checkout --theirs`, never `cargo update -p` |
| Big-bang PR | phases land as separate commits/PRs; Phase 0 is behavior-neutral |
| Implementation in disposable Claude worktree | move to a persistent `scripts/worktree.sh new` worktree before executing the plan |

## 12. Out of scope

- Re-deriving the schema system (ADRs 0058/0061–0063 action+schema redesign).
- `execution/restart`, `resource/{list,get}` implementation (milestone-gated;
  stay honest 501).
- New API capabilities not already stubbed.
- WebSocket/SSE realtime beyond the existing 501 scaffold (§4.5).
- Changing the OpenAPI generator (ADR-0047 is binding).

## 13. Open questions

None blocking. Worktree relocation and per-phase MATURITY updates are handled
at plan/execution time (writing-plans → using-git-worktrees → executing-plans).
