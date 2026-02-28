# Roadmap

## Phase 1: Workflow/Execution REST (Current Focus)

**Deliverables:**
- ApiState extended with engine, storage (or ports)
- Routes: GET/POST /api/v1/workflows, GET /api/v1/workflows/:id
- Routes: POST /api/v1/workflows/:id/execute, GET /api/v1/executions/:id
- Request/response types; error mapping
- OpenAPI spec (optional)

**Risks:**
- Coupling api to engine, storage
- Auth not yet; internal use only initially

**Exit criteria:**
- Create workflow, list workflows, execute, get execution status via REST
- Integration test with real engine

---

## Phase 2: Authentication and Rate Limiting

**Deliverables:**
- JWT middleware; API key middleware
- Rate limiting (tower or custom)
- CORS config refinement
- Auth in OpenAPI spec

**Risks:**
- JWT validation; key management
- Rate limit storage (in-memory vs Redis)

**Exit criteria:**
- Protected routes require auth
- Rate limit returns 429
- Docs for auth headers

---

## Phase 3: WebSocket and Real-Time

**Deliverables:**
- WebSocket handler at /ws or /api/v1/ws
- Execution log streaming
- Status updates (execution progress)
- Connection management; heartbeat

**Risks:**
- Scale (many connections)
- Message protocol design

**Exit criteria:**
- Client connects, receives execution logs
- Reconnection handling

---

## Phase 4: OpenAPI and DX

**Deliverables:**
- OpenAPI 3.0 spec generation (utoipa or similar)
- Swagger UI at /docs
- Example requests in docs
- API versioning guidance (/api/v1, /api/v2)

**Risks:**
- Spec maintenance
- Versioning strategy

**Exit criteria:**
- /docs serves interactive API docs
- Spec matches implementation

---

## Metrics of Readiness

| Metric | Target |
|--------|--------|
| **Correctness** | All routes return expected status/body |
| **Latency** | /health < 1ms; /status < 5ms |
| **Throughput** | Scale with axum |
| **Stability** | No panics; errors propagated |
| **Operability** | Health for k8s; status for debugging |
