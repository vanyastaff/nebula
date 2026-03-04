# nebula-api Architecture Vision

> **Purpose**: Target architecture and structural principles for a production-grade, scalable API layer. Complements [CONSTITUTION.md](./CONSTITUTION.md) (principles, non-negotiables) with concrete layout and evolution path.  
> **Aligned with**: [REST_API_AXUM_GUIDE.md](../../REST_API_AXUM_GUIDE.md), [ARCHITECTURE.md](./ARCHITECTURE.md), [INTERACTIONS.md](./INTERACTIONS.md), [remove-ports-and-drivers](../../plans/remove-ports-and-drivers.md).

---

## 1. North Star: API as Thin Entry Point

**nebula-api is the single HTTP boundary.** It does not run workflows, own execution state, or implement storage. It:

- Accepts HTTP requests and validates input (DTOs, auth, rate limits).
- Calls **ports** (traits: `WorkflowRepo`, `ExecutionRepo`, `TaskQueue`, future `CredentialManager`) injected via `ApiState`.
- Maps domain/port errors to HTTP status and body (e.g. RFC 9457 / `ApiHttpError`).
- Serves health, status, webhook, and REST routes on **one port**.

Execution and engine live in **nebula-runtime** / **nebula-engine**; persistence lives in **nebula-storage** (and optional backends). The app binary composes API + webhook + workers + storage and passes port implementations into state.

```
                    HTTP client / Webhook
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  nebula-api                                                      │
│  • Routes: /health, /api/v1/*, /webhooks/*                       │
│  • Handlers: extract → validate → call port/service → respond    │
│  • State: Arc<dyn WorkflowRepo>, Arc<dyn ExecutionRepo>, …      │
│  • No engine.run() in request path; only enqueue + return         │
└─────────────────────────────────────────────────────────────────┘
    │                    │                    │
    ▼                    ▼                    ▼
WorkflowRepo      ExecutionRepo         TaskQueue (future)
(storage)         (storage)             (runtime)
```

This keeps the API horizontally scalable behind a load balancer while workers and queues scale independently (n8n/Temporal-class).

---

## 2. Target Module Layout (Scalable Structure)

Goal: one clear place per concern; domain-based route groups; thin handlers; services as thin orchestration over ports.

### 2.1 Directory Map

```
nebula-api/src/
├── lib.rs              # Public API: app(), app_with_state(), run()
├── app.rs              # api_router() composition; ApiServer, ApiError
├── config.rs           # Env-backed config (bind, OAuth, rate limit, API keys)
├── state.rs            # ApiState + builder (with_workflow_repo, with_execution_repo, …)
├── errors.rs           # ApiHttpError, ApiResult, IntoResponse; RFC 9457–style mapping
│
├── routes/
│   ├── mod.rs          # Single entry: api_router() — merge public + nest /api/v1
│   ├── system.rs       # public_routes(): /health, /ready; v1_routes(): /api/v1/status
│   ├── auth.rs         # oauth_routes(): /auth/oauth/*; v1_routes(): /api/v1/auth/me
│   ├── workflows.rs    # v1_routes(): /api/v1/workflows (list, get, create, update, delete)
│   ├── runs.rs         # (Phase 2) v1_routes(): /api/v1/runs
│   └── ...             # credentials, nodes, resources when added
│
├── handlers/           # Thin: extract → validate → delegate → map response
│   ├── mod.rs
│   ├── system.rs
│   ├── workflows.rs
│   ├── runs.rs         # (Phase 2)
│   └── ...
│
├── services/           # Thin orchestration over ports; no domain rules
│   ├── mod.rs
│   ├── error.rs        # ServiceError (InvalidInput, NotFound, Conflict, Internal)
│   ├── workflows.rs    # WorkflowService(repo) → list, get, create, update, delete
│   └── runs.rs         # (Phase 2) RunService(execution_repo, task_queue?)
│
├── models/             # DTOs and API response types only
│   ├── mod.rs
│   ├── common.rs       # PaginationQuery, PaginatedResponse<T>, ApiErrorResponse
│   ├── workflows.rs
│   ├── runs.rs
│   └── ...
│
├── auth/               # Auth boundary: OAuth flow, extractors, CORS
│   ├── mod.rs
│   ├── extractor.rs    # Authenticated extractor
│   ├── oauth.rs        # start, callback
│   └── cors.rs
│
├── extractors/         # Custom extractors (e.g. Authenticated)
├── middleware/         # HTTP trace, future: rate-limit layer registration
└── (no repositories/) # Persistence is behind ports; no repo impls in API
```

### 2.2 Routing Strategy

- **Public routes** (no prefix): `/health`, `/ready`, `/auth/oauth/*` — merged at root.
- **Versioned API**: all REST under `/api/v1` via **one** nested router built from domain modules:
  - `Router::new().nest("/api/v1", v1_router)` where `v1_router` is `system::v1_routes().merge(auth::v1_routes()).merge(workflows::v1_routes()).merge(runs::v1_routes())`.
- **Webhook**: when embedded, `merge(webhook.router())` at root (unchanged).
- **Layers**: applied once to the top-level router (body limit, timeout, trace, CORS); order per REST_API_AXUM_GUIDE (ServiceBuilder top-to-bottom).

Each domain (`workflows`, `runs`, …) exposes a `v1_routes()` (or `routes()`) that returns `Router<ApiState>` and only binds paths and methods to handlers. Handlers live in `handlers/` so route modules stay small and testable.

### 2.3 Handler → Service → Port

- **Handler**: extract `State`, `Json`, `Path`, `Query`, `Authenticated`; validate DTO (e.g. `payload.validate()`); call service; map `ServiceError` → `ApiHttpError`; return `ApiResult<impl IntoResponse>`.
- **Service**: holds `Arc<dyn WorkflowRepo>` (or other port); implements list/get/create/update/delete using port methods only; returns `ServiceResult<T>` or maps port errors to `ServiceError`.
- **Port** (trait in nebula-storage / nebula-runtime): implemented outside API (in-memory, Postgres, queue, etc.). API never depends on concrete backends.

No business rules in API: e.g. “can this workflow transition to active?” belongs in engine/workflow; API calls the port and maps conflict/validation errors to HTTP.

---

## 3. Execution and Runs: Enqueue and Return

For **POST /api/v1/workflows/:id/run** (and similar):

1. Handler validates auth and payload (e.g. idempotency key if supported).
2. Handler (or RunService) calls a **port**: e.g. `ExecutionRepo::start_run` or `TaskQueue::enqueue(workflow_id, …)`.
3. API returns **202 Accepted** with `Location: /api/v1/runs/:run_id` and body `{ "run_id": "…" }`.
4. Client polls **GET /api/v1/runs/:id** or subscribes via WebSocket/SSE (Phase 3).

Execution runs in **workers**, not in the request path. This keeps API latency low and allows horizontal scaling of API and workers independently. See REST_API_AXUM_GUIDE § 1.1 (Паттерн «Enqueue and Return»).

---

## 4. Dependencies and Ports

- **API depends on**:
  - **Traits (ports)** from `nebula-storage`: `WorkflowRepo`, `ExecutionRepo` (and their errors).
  - **Traits from nebula-runtime** (when runs/execute are implemented): e.g. `TaskQueue` for enqueue.
  - **nebula-core**: identifiers (`WorkflowId`, etc.) and shared types used in DTOs.
- **API does not depend on**:
  - Concrete storage backends (e.g. `nebula-storage` Postgres feature only in app binary).
  - **nebula-engine** or execution engine types in handler code (only port interfaces).
- **App binary** (or test harness) constructs `ApiState` with `with_workflow_repo(Arc::new(PostgresWorkflowRepo::new(…)))` etc., so API stays backend-agnostic and testable with in-memory mocks.

This aligns with [remove-ports-and-drivers](../../plans/remove-ports-and-drivers.md): contracts in nebula-storage (and runtime); API and app only use those contracts.

---

## 5. Error and Observability

- **Single error type for HTTP**: `ApiHttpError` (or equivalent) with `IntoResponse`; map all service/port errors to status + optional body (e.g. RFC 9457 problem details).
- **Service layer**: `ServiceError` (InvalidInput, NotFound, Conflict, Internal) so handlers have one mapping function from service to HTTP.
- **Observability**: tracing in middleware; middleware failures must not turn into 500 for the client (fail-safe). Metrics and tracing backend are out of scope for API crate; app can add exporters.

---

## 6. Phase Alignment and Gaps

| Phase   | Architecture target |
|--------|----------------------|
| **Current** | Routes: system, auth, workflows. Handlers thin; WorkflowService over WorkflowRepo. State: workflow_repo, execution_repo optional. No webhook merge in current lib (simplified). |
| **Phase 1 completion** | Runs routes + handlers + RunService; ExecutionRepo in state; activate/execute endpoints; enqueue-and-return for run. |
| **Phase 2** | Auth middleware and rate limiting applied to protected routes; CORS refined. |
| **Phase 3** | WebSocket/SSE for run logs; connection handling in API; events from runtime/engine. |
| **Phase 4** | Credentials, nodes, resources routes; OpenAPI spec; optional `/docs`. |

**Structural gaps to close** (no specific order):

- Introduce **single `api_router()`** in `routes/mod.rs` that clearly separates public vs `/api/v1` and uses `.nest("/api/v1", v1)` and `.merge(…)` consistently.
- Add **runs** module (routes, handlers, service) and wire **ExecutionRepo** + (when ready) **TaskQueue** for enqueue-and-return.
- Ensure **no engine or storage backend** in API’s dependency tree except traits and types from storage/core/runtime.
- Optional: **OpenAPI** (utoipa or similar) and **RFC 9457** response shape documented and used for new endpoints.
- Keep **services thin**: only orchestration over ports; move any “business” rule to engine/workflow/credential and expose via port or reject in API as validation only.

---

## 7. Principles Summary (from CONSTITUTION + Vision)

1. **One server, one port** — API and webhook on one Router.
2. **Thin HTTP layer** — handlers only extract, validate, call service/port, map errors.
3. **Ports only** — depend on traits (WorkflowRepo, ExecutionRepo, TaskQueue, …); implementations live in storage/runtime/app.
4. **Enqueue and return** — run/execute endpoints return 202 + run_id; execution in workers.
5. **Stable health and status** — /health, /api/v1/status; no breaking changes in minor.
6. **Observability and middleware** — trace, CORS, compression; fail-safe.
7. **Scalable layout** — domain-based routes and handlers; single `api_router()`; nest/merge used consistently.

This vision is the reference for refactors and new features: any change that thickens handlers with business logic, or ties API to a concrete backend, conflicts with the target architecture and should be redirected (e.g. move logic behind a port or into engine/storage).
