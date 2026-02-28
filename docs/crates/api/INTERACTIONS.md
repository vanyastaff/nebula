# Interactions

## Ecosystem Map (Current + Planned)

### Existing Crates

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-webhook` | Upstream | WebhookServer, WebhookServerConfig; embedded router merged into API |
| `axum` | Upstream | Router, serve, extractors |
| `tower-http` | Upstream | trace, cors, compression-gzip |
| `tokio` | Upstream | TcpListener, async runtime |

### Planned / Potential

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-engine` | Downstream | Phase 2; execute workflow from API |
| `nebula-storage` | Downstream | Phase 2; workflow/execution persistence |
| `nebula-app` | Downstream | Main binary; spawns workers, calls run() |

## Downstream Consumers

### Application (main binary)

- **Expectations:** `run(config, webhook_config, workers)` or `app(webhook, workers)`; single port for API + webhook
- **Contract:** run() blocks until shutdown; workers are snapshot for status
- **Usage:** Spawn workers (tokio::spawn), build workers snapshot, call run()

## Upstream Dependencies

| Crate | Why needed | Hard contract | Fallback |
|-------|------------|---------------|----------|
| `nebula-webhook` | Embedded webhook router | WebhookServer, new_embedded, router() | — |
| `axum` | HTTP server, Router | Router, serve | — |
| `tower-http` | Middleware | trace, cors, compression | — |
| `tokio` | Async, TcpListener | — | — |
| `serde`, `serde_json` | StatusResponse | — | — |
| `tracing` | Logging | — | — |
| `thiserror` | ApiError | — | — |

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|----------------------|-----------|----------|------------|------------------|-------|
| api -> webhook | in | WebhookServer, router merge | sync | ApiError::Webhook | Embedded mode |
| api -> app | out | run(), app() | async | Result<ApiError> | App calls api |
| api -> axum | in | Router, serve | async | Io error | — |

## Runtime Sequence

1. App creates WebhookServer::new_embedded(webhook_config).
2. App spawns N workers (tokio::spawn; workers pull from queue, execute).
3. App builds workers snapshot (Vec<WorkerStatus>) for /api/v1/status.
4. App calls run(api_config, webhook_config, workers).
5. run() binds listener, builds app = api_router + webhook.router(), serves.
6. Clients: GET /health, GET /api/v1/status, POST /webhooks/*.

## Cross-Crate Ownership

| Responsibility | Owner |
|----------------|-------|
| HTTP server, routes | `nebula-api` |
| Webhook routes, registration | `nebula-webhook` |
| Worker loop, queue | App / engine (not api) |
| Status data (workers) | App provides; api displays |

## Failure Propagation

- **Webhook error:** ApiError::Webhook; from new_embedded.
- **Io error:** ApiError::Io; from bind or serve.
- **Handler errors:** health/status are infallible; webhook handlers return their own errors.

## Versioning and Compatibility

- **Compatibility promise:** /health, /api/v1/status paths stable; StatusResponse schema additive.
- **Breaking-change protocol:** Major version bump.
- **Deprecation window:** Minimum 2 minor releases.

## Contract Tests Needed

- [ ] GET /health returns 200
- [ ] GET /api/v1/status returns JSON with workers, webhook
- [ ] Webhook routes merged; POST /webhooks/... reaches webhook handler
- [ ] run() binds and serves
