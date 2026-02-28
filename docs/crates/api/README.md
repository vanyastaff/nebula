# nebula-api

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

- **Out of scope:**
  - Workflow/execution REST endpoints (planned Phase 2)
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

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
