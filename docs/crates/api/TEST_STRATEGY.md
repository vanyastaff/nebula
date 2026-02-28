# Test Strategy

## Test Pyramid

- **Unit:** ApiServerConfig default; WorkerStatus, WebhookStatus serialization; ApiError display.
- **Integration:** run() with in-memory listener; GET /health, GET /api/v1/status; webhook merge.
- **Contract:** /health returns 200; /api/v1/status returns JSON with workers, webhook; StatusResponse schema.
- **End-to-end:** unified_server example; curl /health, /status.

## Critical Invariants

- GET /health returns 200 OK when server is running.
- GET /api/v1/status returns JSON { workers: [...], webhook: { status, route_count, paths } }.
- Webhook routes are reachable at path_prefix (e.g. /webhooks/*).
- run() fails with ApiError if webhook creation or bind fails.
- ApiServerConfig::default() binds 0.0.0.0:5678.

## Scenario Matrix

| Scenario | Coverage |
|----------|----------|
| Happy path | run() succeeds; /health 200; /status 200 + JSON |
| Bind failure | run() returns Io error |
| Webhook creation failure | run() returns Webhook error |
| Webhook merge | POST /webhooks/... reaches webhook handler |
| Empty workers | /status returns workers: [] |
| WorkerStatus fields | id, status, queue_len in JSON |

## Tooling

- **Integration tests:** axum::test::TestServer or bind to 127.0.0.1:0 (random port).
- **Example:** unified_server; manual curl or script.
- **CI quality gates:** cargo test -p nebula-api.

## Exit Criteria

- **Coverage goals:** Health, status handlers; app() merge; run() error paths.
- **Flaky test budget:** Zero.
- **Performance regression:** /health < 1ms.
