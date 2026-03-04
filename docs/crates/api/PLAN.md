# Implementation Plan: nebula-api

**Crate**: `nebula-api` | **Path**: `crates/api` | **ROADMAP**: [ROADMAP.md](ROADMAP.md)

## Summary

Unified API server for Nebula providing REST endpoints for workflow management, execution monitoring, credential operations, and real-time streaming. Current focus is Phase 1 (Workflow + Execution REST) after completing the foundation endpoints and OAuth flow.

## Technical Context

**Language/Edition**: Rust 2024 (MSRV 1.93)
**Async Runtime**: Tokio (multi-thread)
**Key Dependencies**: axum, tower-http, reqwest, serde/serde_json, uuid, url, tracing, thiserror, nebula-webhook
**Testing**: `cargo test -p nebula-api`

## Current Status

| Phase | Status | Summary |
|-------|--------|---------|
| Phase 0: Foundation | Done | Health, status, webhook, OAuth endpoints |
| Phase 1: Workflow + Execution REST | In Progress | CRUD workflows, execute, run history |
| Phase 2: Auth Middleware + Rate Limiting | Planned | Bearer token, API keys, rate limiting, CORS |
| Phase 3: Real-Time Streaming | Planned | WebSocket/SSE for live execution logs |
| Phase 4: Credentials, Nodes, OpenAPI | Planned | Credential CRUD, node catalog, OpenAPI spec |

## Phase Details

### Phase 0: Foundation

**Goal**: Single-port server with health, status, webhook, and OAuth endpoints.

**Deliverables**:
- Single-port server (API + webhook merged router)
- GET /health, GET /api/v1/status
- POST /webhooks/* (nebula-webhook embedded)
- POST /auth/oauth/start, POST /auth/oauth/callback

**Status**: Done.

### Phase 1: Workflow + Execution REST

**Goal**: Desktop app can create, list, execute workflows and see run results.

**Deliverables**:
- ApiState extended with engine + storage (or port traits)
- Workflow CRUD: GET/POST/PATCH/DELETE /workflows, GET /workflows/:id
- POST /workflows/:id/activate (toggle active state)
- POST /workflows/:id/execute (manual trigger)
- Run history: GET /runs (list with filter), GET /runs/:id (detail + node trace)
- Request/response types; standard error shape `{ error, message }`

**Exit Criteria**:
- Create a 3-node workflow, execute it, retrieve the run detail via REST
- Integration test with real engine
- Desktop app Phase 2 exit criteria pass end-to-end

**Risks**:
- Coupling api to engine and storage crates
- Auth middleware not yet present -- internal use only initially

**Dependencies**: nebula-engine (execution), nebula-storage (persistence)

### Phase 2: Auth Middleware + Rate Limiting

**Goal**: Protected routes; production-ready for self-hosted deployment.

**Deliverables**:
- Bearer token middleware (validates tokens from OAuth callback)
- API key middleware for machine-to-machine (CI, scripts)
- Rate limiting (tower or custom) with 429 on breach
- CORS refinement (allow desktop `tauri://localhost` origin)
- 401 response shape: `{ "error": "unauthorized", "message": "..." }`

**Exit Criteria**:
- Workflow routes reject unauthenticated requests with 401
- Rate limit returns 429 with Retry-After header
- Desktop app auto-signs-out on 401

**Risks**:
- Token validation strategy (JWT claims vs opaque token DB lookup)
- Rate limit storage (in-memory vs Redis for cluster)

**Dependencies**: None beyond Phase 1

### Phase 3: Real-Time Streaming

**Goal**: Desktop Monitor screen gets live execution logs within 1 second.

**Deliverables**:
- GET /runs/:id/logs -- WebSocket upgrade (primary) or SSE (fallback)
- Execution progress events pushed on node completion
- Heartbeat / ping-pong; client reconnection supported
- Connection management (max connections, per-user limit)

**Exit Criteria**:
- Desktop receives log events within 1 second of backend node completion
- Reconnection after network drop works without data loss (replay last N events)

**Risks**:
- Scale: many concurrent log streams in cloud deployment
- Message protocol: define event shape `{ nodeId, status, output, timestamp }`

**Dependencies**: nebula-engine (event subscription)

### Phase 4: Credentials, Nodes, OpenAPI

**Goal**: Full desktop feature parity. API self-documents.

**Deliverables**:
- Credential CRUD: GET/POST/DELETE /credentials
- Node catalog: GET /nodes (list with category), GET /nodes/:type (full definition + parameter schema)
- OpenAPI 3.0 spec generation (utoipa or similar)
- Swagger UI at /docs
- API versioning guidance (/api/v1 to /api/v2 strategy)

**Exit Criteria**:
- GitHub credential creates + attaches to a GitHub node (desktop flow passes)
- /docs serves interactive spec that matches implementation

**Risks**: None noted

**Dependencies**: nebula-credential, nebula-plugin (node registry)

## Inter-Crate Dependencies

- **Depends on**: nebula-webhook (Phase 0), nebula-engine (Phase 1+), nebula-storage (Phase 1+), nebula-credential (Phase 4)
- **Depended by**: desktop app (all phases consume API endpoints)

## Verification

- [ ] `cargo check -p nebula-api`
- [ ] `cargo test -p nebula-api`
- [ ] `cargo clippy -p nebula-api -- -D warnings`
- [ ] `cargo doc --no-deps -p nebula-api`
