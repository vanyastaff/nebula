# nebula-api Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula needs a single HTTP entry point: health checks for orchestrators, status for operators, REST API for workflows and executions (planned), and webhook endpoints for triggers. One server, one port, with consistent middleware (trace, CORS, compression) and future auth and rate limiting.

**nebula-api is the unified HTTP server for Nebula: API and webhook on one port.**

It answers: *How do clients and triggers reach Nebula over HTTP — health, status, workflow/execution API, and webhooks — with one server and one config?*

```
HTTP client / webhook
    ↓
axum Router: /health, /api/v1/*, /webhooks/*
    ↓
app(webhook_server, workers) → run(config, webhook_config, workers)
    ↓
GET /health (liveness), GET /api/v1/status (workers + webhook), POST /webhooks/* (trigger)
```

This is the API contract: single entry point; stable routes for health and status; webhook path configurable; future workflow/execution endpoints and auth.

---

## User Stories

### Story 1 — Orchestrator Probes Liveness (P1)

Kubernetes or a load balancer probes GET /health. Server returns 200 when process is alive and can accept traffic. No auth; minimal response.

**Acceptance**:
- GET /health → 200 with optional body (e.g. {"status":"ok"})
- No dependency on engine or storage for liveness; fast path
- Document that /health is for liveness only; use /api/v1/status for readiness if needed

### Story 2 — Operator Checks Workers and Webhook (P1)

Operator opens GET /api/v1/status to see worker snapshot and webhook status. Response is JSON (WorkerStatus, WebhookStatus). Used for dashboard and debugging.

**Acceptance**:
- GET /api/v1/status → 200 + JSON
- Workers snapshot (current design may be static; document and roadmap dynamic)
- Webhook status (enabled, base path)
- Stable path and shape; minor = additive fields only

### Story 3 — Trigger Delivers Webhook (P1)

External service (e.g. GitHub) POSTs to /webhooks/:id or configured path. API routes to embedded webhook server; webhook handler validates and enqueues or runs workflow. Path prefix configurable.

**Acceptance**:
- POST /webhooks/* handled by embedded webhook server
- Webhook config (base path, verification) from config
- One port for API and webhook; no separate webhook port in default design

### Story 4 — Client Calls Workflow/Execution API (P2)

Client wants to start a workflow, list executions, get result. REST endpoints (e.g. POST /api/v1/workflows/:id/run, GET /api/v1/executions/:id) are planned. Auth (JWT/API key) and rate limiting are planned.

**Acceptance**:
- Endpoints documented in ROADMAP and API.md
- When implemented: stable path and versioning (/api/v1/)
- Auth and rate limiting before or with workflow/execution endpoints

---

## Core Principles

### I. One Server, One Port

**API and webhook are served by one axum app on one port. No separate process or port for webhook in default deployment.**

**Rationale**: Simpler ops and config. Single entry point for TLS and ingress. Webhook is just another route.

**Rules**:
- app(webhook_server, workers) builds Router; run() starts one server
- Config: bind address, port, webhook base path
- Document how to put API and webhook behind same host/path if needed

### II. Health and Status Are Stable

**/health and /api/v1/status paths and semantics are stable. Patch/minor do not break them.**

**Rationale**: Orchestrators and dashboards depend on these. Breaking them breaks deployments.

**Rules**:
- /health: liveness only; 200 when process is up
- /api/v1/status: workers + webhook status; additive JSON fields in minor
- Breaking path or required field = major + MIGRATION.md

### III. Webhook Path Configurable

**Webhook base path (e.g. /webhooks) is configurable so that deployments can align with reverse proxy or security rules.**

**Rationale**: Some deployments need /api/webhooks or /v1/webhooks. Config avoids code change.

**Rules**:
- WebhookConfig or run() config has base path
- Document default and override

### IV. No Business Logic in API Crate

**API crate owns HTTP layer: routes, middleware, error mapping. It does not implement workflow engine, storage, or credential logic.**

**Rationale**: Engine, storage, credential are separate. API calls into them; it does not duplicate their logic.

**Rules**:
- Workflow/execution endpoints (when added) call engine or service layer
- Auth and rate limiting are middleware or delegated to dedicated crates
- ApiError maps from domain errors to HTTP status and body

### V. Middleware and Observability

**Trace, CORS, compression (tower-http) are applied. Observability failures must not break requests.**

**Rationale**: Production needs tracing and CORS. Middleware must be non-blocking and fail-safe.

**Rules**:
- tower-http or equivalent for trace, CORS, compression
- Middleware that fails (e.g. trace export down) does not return 500 to client
- Document middleware order and config

---

## Production Vision

### The API in an n8n-class fleet

In production, one or more API instances sit behind a load balancer. GET /health is used for liveness; GET /api/v1/status for operator dashboard. Webhooks are POSTed to /webhooks/* and handled by embedded webhook server. Future: POST /api/v1/workflows/:id/run, GET /api/v1/executions/:id, auth (JWT/API key), rate limiting, OpenAPI, WebSocket for real-time. One port; TLS at load balancer or at server.

```
ApiServer
    ├── GET /health
    ├── GET /api/v1/status → WorkerStatus, WebhookStatus
    ├── POST /webhooks/* → webhook handler (embedded)
    ├── (Phase 2) POST /api/v1/workflows/:id/run, GET /api/v1/executions/:id
    ├── (Phase 2) Auth, rate limiting
    └── Middleware: trace, CORS, compression
```

Workers snapshot: current design may be static; target is dynamic integration with worker crate. Engine and storage integration for workflow/execution endpoints when implemented.

### From the archives: phase 5 and production

The archive `_archive/archive-phase-5-production.md` and api docs describe production criteria: workflow/execution endpoints, auth, rate limiting, OpenAPI, WebSocket. Production vision aligns: health and status stable; webhook configurable; Phase 2 adds workflow/execution and auth; compatibility guarantees for existing routes.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Workflow/execution REST endpoints | High | Phase 2; engine/storage integration |
| Auth (JWT/API key) | High | Middleware or dedicated crate |
| Rate limiting | Medium | Per-IP or per-key |
| Dynamic workers snapshot | Medium | Integrate with worker crate |
| OpenAPI spec | Medium | For client generation |
| WebSocket for real-time | Low | Deferred |

---

## Key Decisions

### D-001: API and Webhook on One Port

**Decision**: Single server serves both API routes and webhook routes.

**Rationale**: Simpler deployment and TLS. One ingress rule.

**Rejected**: Separate webhook server/port — would double ops burden.

### D-002: axum as the HTTP Stack

**Decision**: Use axum for router and tower for middleware.

**Rationale**: Async, ecosystem fit, and tower compatibility for trace/CORS/compression.

**Rejected**: Actix or raw hyper — axum is the chosen stack.

### D-003: Health Is Liveness Only

**Decision**: /health returns 200 when process is alive; does not check engine or storage.

**Rationale**: Liveness should be fast and not fail when a dependency is temporarily down. Readiness can be a separate endpoint or status.

**Rejected**: /health checking DB — would make pod restart on DB blip.

### D-004: Status Path Under /api/v1/

**Decision**: Status is GET /api/v1/status for future versioning and namespace.

**Rationale**: Keeps room for /api/v1/workflows, etc. without breaking status.

**Rejected**: GET /status without version — would conflict with future API layout.

---

## Open Proposals

### P-001: Workflow/Execution Endpoints

**Problem**: Clients need to start workflows and read executions.

**Proposal**: POST /api/v1/workflows/:id/run, GET /api/v1/executions/:id, list endpoints; integrate with engine and storage.

**Impact**: New routes and dependencies on engine/storage; auth required.

### P-002: Auth Middleware

**Problem**: Production needs JWT or API key auth for API routes.

**Proposal**: Middleware that validates token/key and injects identity; webhook may have separate verification (e.g. signature).

**Impact**: New dependency and config; document public vs protected routes.

---

## Non-Negotiables

1. **One server, one port** — API and webhook together; no separate webhook server in default.
2. **/health and /api/v1/status stable** — no breaking change in patch/minor.
3. **Webhook path configurable** — base path from config.
4. **No business logic in API crate** — HTTP layer only; call engine/storage for domain.
5. **Middleware non-blocking** — trace/CORS/compression do not fail requests.
6. **Breaking route or response contract = major + MIGRATION.md** — clients and orchestrators depend on it.

---

## Governance

- **PATCH**: Bug fixes, docs. No change to routes or response shape.
- **MINOR**: Additive (new endpoints, new status fields). No removal of routes or required fields.
- **MAJOR**: Breaking changes to paths or response. Requires MIGRATION.md.
