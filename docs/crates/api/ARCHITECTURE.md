# Architecture

## Problem Statement

- **Business problem:** Nebula needs an HTTP entry point for health checks, status, and webhooks. One process should serve API and webhook on one port (Docker/local simplicity).
- **Technical problem:** Provide unified axum Router merging API routes and embedded webhook server; single bind address; minimal coupling to engine/storage (MVP).

## Current Architecture

### Module Map

| Location | Responsibility |
|----------|----------------|
| `lib.rs` | app(), run(); Router merge |
| `server.rs` | api_router() composition, health/status handlers, ApiServerConfig/ApiServer/ApiError |
| `state.rs` | ApiState + dependency wiring (repos, oauth/rate-limit stores) |
| `auth/` | `extractor`, `oauth`, `cors` split for auth boundary and OAuth flow |
| `workflows.rs` | Workflow REST handlers (list/get/create/update/delete) |
| `error.rs` | Unified HTTP error envelope (`ApiHttpError`, `ApiResult`) |
| `middleware.rs` | Cross-cutting middleware registration (HTTP trace layer) |
| `status.rs` | WorkerStatus, WebhookStatus |
| `contracts.rs` | Shared API contracts (error envelope, pagination, workflow/run DTOs) |

### Data/Control Flow

1. **Main** (or app crate) creates WebhookServer (embedded), workers snapshot, calls `run(config, webhook_config, workers)`.
2. **run** binds TcpListener, builds `app(webhook, workers)` = api_router().with_state(state).merge(webhook.router()).
3. **axum::serve** runs on one port.
4. **GET /health** → 200 OK.
5. **GET /api/v1/status** → JSON { workers, webhook }.
6. **POST /webhooks/*** → webhook server handlers.

### Known Bottlenecks

- **Static workers:** Workers passed as Vec<WorkerStatus>; no live pool; status is snapshot.
- **Partial ports wiring:** `WorkflowRepo` is now wired for list/get/create/update under `/api/v1/workflows`; delete/activate/execute and all `ExecutionRepo` routes remain.
- **Auth scope is partial:** Auth boundary protects `/api/v1/auth/me` and workflow routes; runs/streaming/credentials routes are not implemented yet.
- **No WebSocket:** Real-time updates not implemented.

## Target Architecture

### Target Module Map

```
nebula-api/
├── lib.rs         — app, run (current)
├── server.rs      — api_router, ApiState, ApiServer (current)
├── status.rs      — WorkerStatus, WebhookStatus (current)
├── routes/        — (Phase 2) workflows, executions, nodes
├── auth/          — (Phase 2) JWT, API key
├── websocket/     — (Phase 2) real-time
└── openapi/       — (Phase 2) spec generation
```

### Public Contract Boundaries

- `app(webhook, workers)` → Router; merge API + webhook.
- `run(config, webhook_config, workers)` → Result<(), ApiError>; blocks until shutdown.
- ApiState: webhook, workers; extensible for engine, storage.

### Internal Invariants

- One port for API + webhook.
- /health always returns 200 when server is up.
- /api/v1/status reflects current ApiState.

## Design Reasoning

### Key Trade-off 1: One port vs separate

- **Current:** One port; API + webhook merged. Simpler deployment; one container.
- **Alternative:** Separate API and webhook ports. More complex; rejected for MVP.

### Key Trade-off 2: Minimal vs full API

- **Current:** Health + status only; no workflow/execution. Fast to ship; engine integration deferred.
- **Target:** Full REST (workflows, executions); auth; WebSocket. Phase 2.

### Rejected Alternatives

- **GraphQL:** Deferred; REST + WebSocket first (see archive).
- **Actix-web:** Axum chosen; async, ecosystem fit.

## Comparative Analysis

Sources: n8n, Node-RED, Temporal, Prefect.

| Pattern | Verdict | Rationale |
|---------|---------|-----------|
| One port API + webhook | **Adopt** | n8n; simpler deployment |
| REST for workflows/executions | **Adopt** | Standard; OpenAPI |
| WebSocket for real-time | **Adopt** | Phase 2; execution logs, status |
| GraphQL | **Defer** | REST sufficient; add later if needed |
| JWT/API key auth | **Adopt** | Phase 2; production requirement |

## Breaking Changes (if any)

- None planned; routes additive.

## Open Questions

- Q1: Dynamic workers vs snapshot? (Live WorkerPool ref vs Vec)
- Q2: API versioning (/api/v1/) — when to add v2?
