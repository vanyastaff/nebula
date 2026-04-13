# Roadmap

> ⚠️ **STALE 2026-04-13.** References to `nebula-webhook` are
> obsolete — orphan crate deleted, webhook HTTP ingress now lives
> in `nebula-api::webhook`. See
> `docs/plans/2026-04-13-webhook-subsystem-spec.md`.

## Phase 0: Foundation ✅

**Done:**
- Single-port server (API + webhook merged router)
- GET /health, GET /api/v1/status
- POST /webhooks/* (nebula-webhook embedded)
- POST /auth/oauth/start — begin GitHub OAuth, return authUrl
- POST /auth/oauth/callback — exchange code, return accessToken + user

---

## Phase 1: Workflow + Execution REST (Current Focus)

**Goal:** Desktop app can create, list, execute workflows and see run results.
**Aligns with:** Desktop Phase 2 — Workflow Management.

**Deliverables:**
- ApiState extended with engine + storage (or port traits)
- `GET /workflows` — list with pagination
- `GET /workflows/:id`
- `POST /workflows` — create
- `PATCH /workflows/:id` — update
- `DELETE /workflows/:id`
- `POST /workflows/:id/activate` — toggle active state
- `POST /workflows/:id/execute` — manual trigger
- `GET /runs` — list with filter (workflow, status, date)
- `GET /runs/:id` — run detail + node-by-node trace
- Request/response types; standard error shape `{ error, message }`

**Risks:**
- Coupling api to engine, storage
- Auth middleware not yet — internal use only initially (desktop dev environment)

**Exit criteria:**
- Create a 3-node workflow, execute it, retrieve the run detail via REST
- Integration test with real engine
- Desktop app Phase 2 exit criteria pass end-to-end

---

## Phase 2: Auth Middleware + Rate Limiting

**Goal:** Protected routes; production-ready for self-hosted deployment.
**Aligns with:** Desktop Phase 2 (multi-connection remote servers need real auth).

**Deliverables:**
- Bearer token middleware (validates tokens issued by `/auth/oauth/callback`)
- API key middleware for machine-to-machine (CI, scripts)
- Rate limiting (tower or custom) — 429 on breach
- CORS refinement (allow desktop `tauri://localhost` origin)
- 401 response shape: `{ "error": "unauthorized", "message": "..." }`

**Risks:**
- Token validation strategy (JWT claims vs opaque token DB lookup)
- Rate limit storage (in-memory sufficient for single-instance; Redis for cluster)

**Exit criteria:**
- Workflow routes reject unauthenticated requests with 401
- Rate limit returns 429 with Retry-After header
- Desktop app auto-signs-out on 401 (existing behavior, now exercised)

---

## Phase 3: Real-Time Streaming

**Goal:** Desktop Monitor screen gets live execution logs within 1 second.
**Aligns with:** Desktop Phase 3 — Monitor & Registry.

**Deliverables:**
- `GET /runs/:id/logs` — WebSocket upgrade (primary) or SSE (fallback)
- Execution progress events pushed on node completion
- Heartbeat / ping-pong; client reconnection supported
- Connection management (max connections, per-user limit)

**Risks:**
- Scale: many concurrent log streams in cloud deployment
- Message protocol: define event shape (`{ nodeId, status, output, timestamp }`)

**Exit criteria:**
- Desktop receives log events within 1 second of backend node completion
- Reconnection after network drop works without data loss (replay last N events)

---

## Phase 4: Credentials, Nodes, OpenAPI

**Goal:** Full desktop feature parity. API self-documents.
**Aligns with:** Desktop Phase 4 — Credentials & Registry.

**Deliverables:**
- `GET /credentials`, `POST /credentials`, `DELETE /credentials/:id`
- `GET /nodes` — list node types with category + description
- `GET /nodes/:type` — full definition + parameter schema
- OpenAPI 3.0 spec generation (utoipa or similar)
- Swagger UI at /docs
- API versioning guidance (/api/v1 → /api/v2 strategy)

**Exit criteria:**
- GitHub credential creates + attaches to a GitHub node (desktop flow passes)
- /docs serves interactive spec that matches implementation

---

## Metrics of Readiness

| Metric | Target |
|--------|--------|
| **Correctness** | All routes return expected status/body |
| **Latency** | /health < 1ms; /status < 5ms; /workflows list < 50ms |
| **Throughput** | Scale with axum (no artificial bottlenecks) |
| **Stability** | No panics; errors propagated as `{ error, message }` |
| **Operability** | /health for k8s liveness; /status for debugging |
