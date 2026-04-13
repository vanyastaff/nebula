# nebula-api

> ⚠️ **STALE 2026-04-13.** This document predates the webhook
> subsystem consolidation. References to `nebula-webhook` are
> obsolete — the orphan crate was deleted and HTTP ingress for
> webhook triggers now lives inside `nebula-api` itself as
> `api::webhook::WebhookTransport`. See
> `docs/plans/2026-04-13-webhook-subsystem-spec.md` and
> `crates/api/src/webhook/` for the current design. The rest of
> this file is kept for historical context only.

Unified HTTP server for Nebula: API + webhook on one port.

## Scope

- **In scope:**
  - `app(webhook_server, workers)` — combined Router (API + webhook)
  - `run(config, webhook_config, workers)` — start axum server
  - `GET /health` — liveness
  - `GET /api/v1/status` — workers + webhook status
  - `POST /webhooks/*` — embedded webhook routes (from nebula-webhook)
  - ApiServer, ApiServerConfig, ApiError
  - WorkerStatus, WebhookStatus

- **Out of scope (for now):**
  - Workflow/execution REST endpoints (planned Phase 2)
  - Credential REST endpoints (planned Phase 4; see [API.md](./API.md) — storage backend chosen at app composition, e.g. Postgres via [credential MIGRATION](../credential/MIGRATION.md))
  - Authentication, rate limiting (planned)
  - WebSocket (planned)
  - OpenAPI spec (planned)
  - GraphQL (deferred)

## Current State

- **Maturity:** MVP — health, status, embedded webhook; one port
- **Key strengths:** Single entry point; API + webhook merged; minimal deps; tower-http (trace, cors, compression)
- **Key risks:** Workers snapshot is static; no engine/storage integration; no auth

## Target State

- **Production criteria:** Workflow/execution endpoints; auth (JWT/API key); rate limiting; OpenAPI; WebSocket for real-time
- **Compatibility guarantees:** /health, /api/v1/status stable; webhook path prefix configurable

## Document Map

- [VISION.md](./VISION.md) — target architecture, module layout, ports, enqueue-and-return
- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [ROADMAP.md](./ROADMAP.md)
- [MIGRATION.md](./MIGRATION.md)
