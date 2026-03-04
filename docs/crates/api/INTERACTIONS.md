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
| `nebula-credential` | Downstream | Phase 4; credential CRUD + interactive flows; API receives `CredentialManager` in state |
| `nebula-app` | Downstream | Main binary; spawns workers, calls run() |

### Credential storage (Phase 4)

Credential routes (`GET/POST/DELETE /credentials`, etc.) call `CredentialManager` methods. The **storage backend** (in-memory mock, local filesystem, or **Postgres**) is **not** chosen inside nebula-api. The application that composes the stack (e.g. nebula-app or a custom main) builds `CredentialManager` with the desired `StorageProvider` (e.g. `PostgresStorageProvider` over `nebula-storage::PostgresStorage`) and passes it into API state. So "credentials in DB" is a deployment/composition choice; API code stays backend-agnostic.

See [credential POSTGRES_STORAGE_SPEC](../credential/POSTGRES_STORAGE_SPEC.md) and [credential MIGRATION — Postgres](../credential/MIGRATION.md#migration-to-postgres-backed-storage-db-storage) for DB-backed storage.

## Downstream Consumers

### Application (main binary)

- **Expectations:** `run(config, webhook_config, workers)` or `app(webhook, workers)`; single port for API + webhook
- **Contract:** run() blocks until shutdown; workers are snapshot for status
- **Usage:** Spawn workers (tokio::spawn), build workers snapshot, call run()

### Desktop App (Tauri)

- **Expectations:** HTTP API at a configurable base URL (default `http://localhost:5678`)
- **Auth contract:** `POST /auth/oauth/start` + `POST /auth/oauth/callback` — GitHub OAuth flow with deep-link redirect to `nebula://auth/callback`
- **Token usage:** All authenticated routes require `Authorization: Bearer <access_token>` from the OAuth callback response
- **Auth boundary:** API validates bearer token against tokens issued by `/auth/oauth/callback`; invalid/missing token returns `401 { error, message }`
- **M2M contract:** API also accepts `X-API-Key` (keys configured in `NEBULA_API_KEYS`) for machine-to-machine calls
- **Rate limit contract:** protected routes return `429` with `Retry-After` when request budget is exceeded
- **Phase 2 contract:** Full workflow CRUD + execution trigger (see `docs/apps/desktop/INTEGRATION.md`)
- **Phase 3 contract:** Run list, run detail, real-time log streaming via WebSocket (`GET /runs/:id/logs`)
- **Error shape:** `{ "error": "error_code", "message": "Human readable" }` on all non-2xx responses
- **401 behavior:** Desktop signs the user out and redirects to login on any 401 response

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

### Desktop OAuth sequence (current)

```
Desktop → POST /auth/oauth/start { provider: "github", redirectUri: "nebula://auth/callback" }
       ← { authUrl: "https://github.com/login/oauth/authorize?..." }
Desktop opens authUrl in system browser
User authenticates → browser redirects → nebula://auth/callback?code=XXX
Desktop → POST /auth/oauth/callback { provider, code, redirectUri }
       ← { accessToken, user }
Desktop stores token in OS-secure store (tauri-plugin-store)
All subsequent requests: Authorization: Bearer <accessToken>
```

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
- [ ] POST /auth/oauth/start returns 200 + `{ authUrl }`
- [ ] POST /auth/oauth/callback returns 200 + `{ accessToken, user }`
- [x] Unauthenticated request to protected route returns 401 with `{ error, message }`
- [x] Rate-limited request to protected route returns 429 with `Retry-After`
- [x] GET `/api/v1/workflows` returns paginated list when repository is configured
- [x] GET `/api/v1/workflows/:id` returns workflow detail or 404
- [x] POST `/api/v1/workflows` creates a workflow and returns 201
- [x] PATCH `/api/v1/workflows/:id` updates selected fields with optimistic concurrency
- [x] DELETE `/api/v1/workflows/:id` returns 204 or 404
