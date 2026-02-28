# Decisions

## D001: One port for API + webhook

**Status:** Adopt

**Context:** Need HTTP entry point for health, status, webhooks. Separate ports add deployment complexity.

**Decision:** Single bind address; Router merges api_router() and webhook_server.router(). One process, one port.

**Alternatives considered:**
- Separate API and webhook ports — more config; rejected
- Webhook as separate service — deployment complexity; rejected for MVP

**Trade-offs:** Simpler; webhook and API share middleware (cors, compression, trace).

**Consequences:** Path prefix for webhooks (/webhooks) avoids collision with /health, /api.

**Migration impact:** None.

**Validation plan:** unified_server example; integration test.

---

## D002: Embedded webhook (nebula-webhook)

**Status:** Adopt

**Context:** Webhook server can run standalone or embedded. API embeds it.

**Decision:** WebhookServer::new_embedded() creates in-process webhook; router() returns Router; api merges with merge(). Same process, same port.

**Alternatives considered:**
- API owns webhook routes — would duplicate webhook logic; webhook crate owns routes
- Separate webhook process — more complexity; rejected

**Trade-offs:** api depends on webhook; webhook is reusable standalone.

**Consequences:** WebhookServerConfig passed to run(); base_url, path_prefix configurable.

**Migration impact:** None.

**Validation plan:** Webhook routes work when merged.

---

## D003: Static workers snapshot

**Status:** Adopt (for MVP)

**Context:** /api/v1/status shows workers. Workers run in separate tokio tasks; api needs their state.

**Decision:** App passes Vec<WorkerStatus> to run(). Snapshot at startup; no live updates. Replace with dynamic WorkerPool ref later.

**Alternatives considered:**
- Arc<WorkerPool> — would require api to depend on worker/engine; deferred
- Channel for status updates — more complex; deferred

**Trade-offs:** Simple; status may be stale. Acceptable for MVP.

**Consequences:** Workers snapshot built by app; api just displays.

**Migration impact:** Phase 2 may add live worker state.

**Validation plan:** Status returns passed workers.

---

## D004: No engine/storage in MVP

**Status:** Adopt

**Context:** Archive had workflow/execution endpoints. Full API requires engine, storage.

**Decision:** MVP has health + status only. No workflow/execution routes. Engine and storage integration in Phase 2.

**Alternatives considered:**
- Full API now — would couple api to engine, storage, workflow; too much for MVP
- Stub routes — could return 501; adds noise; rejected

**Trade-offs:** Minimal api; fast to ship. Phase 2 adds routes.

**Consequences:** App does not pass engine/storage to api yet.

**Migration impact:** ApiState will need engine, storage fields in Phase 2.

**Validation plan:** Phase 2 design; ApiState extension.

---

## D005: Axum over Actix

**Status:** Adopt

**Context:** Need async HTTP framework. Axum and Actix-web are main options.

**Decision:** Axum. Tokio ecosystem; Router, extractors, tower compatibility.

**Alternatives considered:**
- Actix-web — mature; different ecosystem
- warp — less active; Axum preferred

**Trade-offs:** Axum is well-maintained; tower-http for middleware.

**Consequences:** axum 0.7; tower-http 0.6.

**Migration impact:** None.

**Validation plan:** Current implementation works.
