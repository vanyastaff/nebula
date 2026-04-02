# Tasks: nebula-api

**ROADMAP**: [ROADMAP.md](ROADMAP.md) | **PLAN**: [PLAN.md](PLAN.md)

## Format: `[ID] [P?] Description`

- **[P]**: Can run in parallel with other [P] tasks in same phase
- IDs use prefix API

---

## Phase 0: Foundation

**Goal**: Single-port server with health, status, webhook, and OAuth endpoints.

- [x] API-T001 [P] Set up axum server scaffold with single-port router
- [x] API-T002 [P] Implement GET /health and GET /api/v1/status endpoints
- [x] API-T003 Embed nebula-webhook router under POST /webhooks/*
- [x] API-T004 Implement POST /auth/oauth/start (begin GitHub OAuth, return authUrl)
- [x] API-T005 Implement POST /auth/oauth/callback (exchange code, return accessToken + user)

**Checkpoint**: Server starts, health/status respond, OAuth flow completes end-to-end.

---

## Phase 1: Workflow + Execution REST

**Goal**: Desktop app can create, list, execute workflows and see run results.

- [x] API-T006 Extend ApiState with engine and storage handles (or port traits)
- [x] API-T007 Define request/response types and standard error shape `{ error, message }`
- [x] API-T008 [P] Implement GET /workflows (list with pagination) and GET /workflows/:id
- [x] API-T009 [P] Implement POST /workflows (create) and PATCH /workflows/:id (update)
- [x] API-T010 [P] Implement DELETE /workflows/:id
- [ ] API-T011 Implement POST /workflows/:id/activate (toggle active state)
- [ ] API-T012 Implement POST /workflows/:id/execute (manual trigger via engine)
- [ ] API-T013 [P] Implement GET /runs (list with filter by workflow, status, date) and GET /runs/:id (detail + node trace)
- [ ] API-T014 Write integration test: create 3-node workflow, execute, retrieve run detail

**Checkpoint**: All workflow CRUD and run endpoints respond correctly. Integration test passes with real engine.

---

## Phase 2: Auth Middleware + Rate Limiting

**Goal**: Protected routes; production-ready for self-hosted deployment.

- [x] API-T015 Implement bearer token middleware (validate tokens from /auth/oauth/callback)
- [x] API-T016 Implement API key middleware for machine-to-machine access
- [x] API-T017 Add rate limiting layer (tower or custom) returning 429 with Retry-After header
- [x] API-T018 Refine CORS configuration (allow `tauri://localhost` origin for desktop)
- [x] API-T019 Define 401 response shape and wire all protected routes
- [x] API-T020 Write tests: unauthenticated request gets 401, rate-limited request gets 429

**Checkpoint**: Workflow routes reject unauthenticated requests. Rate limit triggers correctly. Desktop auto-signs-out on 401.

---

## Phase 3: Real-Time Streaming

**Goal**: Live execution logs delivered to desktop within 1 second.

- [ ] API-T021 Define streaming event schema `{ nodeId, status, output, timestamp }`
- [ ] API-T022 Implement GET /runs/:id/logs as WebSocket upgrade (primary path)
- [ ] API-T023 Implement SSE fallback for /runs/:id/logs
- [ ] API-T024 Add heartbeat/ping-pong and connection management (max connections, per-user limit)
- [ ] API-T025 Implement reconnection support with replay of last N events
- [ ] API-T026 Write integration test: execute workflow, verify log events arrive within 1 second

**Checkpoint**: Desktop receives real-time log events. Reconnection after network drop works without data loss.

---

## Phase 4: Credentials, Nodes, OpenAPI

**Goal**: Full desktop feature parity with self-documenting API.

- [ ] API-T027 [P] Implement GET /credentials, POST /credentials, DELETE /credentials/:id
- [ ] API-T028 [P] Implement GET /nodes (list with category + description) and GET /nodes/:type (full definition + parameter schema)
- [ ] API-T029 Add OpenAPI 3.0 spec generation (utoipa or similar)
- [ ] API-T030 Serve Swagger UI at /docs
- [ ] API-T031 Document API versioning strategy (/api/v1 to /api/v2)
- [ ] API-T032 Write end-to-end test: create GitHub credential, attach to node, verify via REST

**Checkpoint**: All CRUD endpoints functional. /docs serves interactive OpenAPI spec matching implementation.

---

## Dependencies & Execution Order

Phases are strictly sequential: each phase builds on the endpoints and infrastructure from the previous one.

- **Phase 0** (done) provides the server scaffold and OAuth.
- **Phase 1** requires nebula-engine and nebula-storage to be available.
- **Phase 2** can proceed independently of engine changes but requires Phase 1 routes to protect.
- **Phase 3** requires engine event subscription (execution progress events).
- **Phase 4** requires nebula-credential and plugin/node registry.

Within each phase, tasks marked [P] can be developed in parallel.
