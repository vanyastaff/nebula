# API

## Public Surface

### Stable APIs

- `app(webhook_server, workers)` → Router
- `api_only_app(webhook_server, workers)` → Router (API routes only; no webhook merge)
- `app_with_state(state)` → Router (API + webhook; explicit dependency injection)
- `api_only_app_with_state(state)` → Router (API only; explicit dependency injection)
- `run(config, webhook_config, workers)` → `impl Future<Output = Result<(), ApiError>>`
- `contracts` module:
  - `ApiErrorResponse { error, message }`
  - `PaginationQuery`, `PaginatedResponse<T>`
  - workflow/run DTOs for Phase 1 endpoints
- `ApiServer` — `new(config)`, `app(webhook, workers)`
- `ApiServerConfig` — `bind_addr`; Default: 0.0.0.0:5678
- `ApiError` — Webhook(webhook::Error), Io(std::io::Error)
- `WorkerStatus` — id, status, queue_len
- `WebhookStatus` — status, route_count, paths; `from_server(server)`
- `StatusResponse` — workers, webhook (internal; JSON response)

### ApiState Dependencies (Phase 1)

- `workflow_repo: Option<Arc<dyn WorkflowRepo>>` — workflow persistence port (`nebula-ports`)
- `execution_repo: Option<Arc<dyn ExecutionRepo>>` — execution persistence/coordination port (`nebula-ports`)

### Routes

#### Infrastructure (stable)

| Method | Path | Description |
|--------|------|-------------|
| GET | /health | Liveness; 200 OK |
| GET | /ready | Readiness; 200 READY |
| GET | /api/v1/status | JSON { workers, webhook } |
| POST | /webhooks/* | Webhook endpoints (from nebula-webhook) |

#### Auth (implemented)

| Method | Path | Description |
|--------|------|-------------|
| POST | /auth/oauth/start | Begin OAuth flow; body `{ provider, redirectUri }`; returns `{ authUrl }` |
| POST | /auth/oauth/callback | Exchange code; body `{ provider, code, redirectUri }`; returns `{ accessToken, user }` |
| GET | /api/v1/auth/me | Protected probe endpoint; requires `Authorization: Bearer <token>` |

### Protected Route Security Contract

- Accepts either:
  - `Authorization: Bearer <access_token>` where token was issued by `/auth/oauth/callback`
  - `X-API-Key: <key>` where key is configured in `NEBULA_API_KEYS` (comma-separated)
- Missing/invalid auth returns `401` with shape:
  - `{ "error": "error_code", "message": "Human readable" }`
- Protected routes are rate-limited:
  - default: 120 requests / 60 seconds per auth principal + path
  - configurable via `NEBULA_RATE_LIMIT_MAX_REQUESTS` and `NEBULA_RATE_LIMIT_WINDOW_SECONDS`
  - on limit breach returns `429` with `Retry-After` header

#### Workflows *(Phase 1 baseline)*

| Method | Path | Description |
|--------|------|-------------|
| GET | /api/v1/workflows | List workflows (pagination: `offset`, `limit`) |
| GET | /api/v1/workflows/:id | Get workflow by ID |
| POST | /api/v1/workflows | Create workflow |
| PATCH | /api/v1/workflows/:id | Update workflow |
| DELETE | /api/v1/workflows/:id | Delete workflow |
| POST | /api/v1/workflows/:id/activate | Activate / deactivate workflow |
| POST | /api/v1/workflows/:id/execute | Manual trigger (test run) |

#### Runs *(Phase 2–3)*

| Method | Path | Description |
|--------|------|-------------|
| GET | /runs | List runs (filter by workflow, status, date) |
| GET | /runs/:id | Get run detail + node-by-node trace |
| GET | /runs/:id/logs | Stream execution logs (WebSocket upgrade or SSE) |

#### Credentials *(Phase 4)*

| Method | Path | Description |
|--------|------|-------------|
| GET | /credentials | List credentials (metadata only, no secrets) |
| GET | /credentials/:id | Get credential status + metadata |
| POST | /credentials | Create credential; returns 202 + interaction if user action required |
| POST | /credentials/:id/callback | Continue interactive flow (deliver OAuth code, device confirmation, etc.) |
| DELETE | /credentials/:id | Delete credential and revoke tokens |
| GET | /credential-types | List registered credential types with parameter schemas |

**Interactive flow — 202 response pattern:**

When `POST /credentials` triggers an interactive flow (e.g. OAuth2 Authorization Code), the response is:

```json
HTTP 202 Accepted
{
  "id": "cred-abc123",
  "status": "pending_interaction",
  "interaction": {
    "type": "redirect",
    "url": "https://github.com/login/oauth/authorize?client_id=...&state=..."
  }
}
```

Other `interaction.type` values: `"display_info"` (Device Flow — show user/device code), `"code_input"` (prompt user to paste a code), `"await_confirmation"` (show instructions).

After the user completes the interaction, the client calls `POST /credentials/:id/callback`:

```json
{ "params": { "code": "ghu_XXX", "state": "YYY" } }
```

On success: `200 { "id": "cred-abc123", "status": "active", "metadata": { "name": "GitHub", "type": "oauth2_github", "scopes": ["repo"] } }`

#### Nodes *(Phase 4)*

| Method | Path | Description |
|--------|------|-------------|
| GET | /nodes | List available node types |
| GET | /nodes/:type | Get node definition + parameter schema |

#### Resources *(Phase 4)*

| Method | Path | Description |
|--------|------|-------------|
| GET | /resources | List all resources with health + pool stats (snapshot) |
| GET | /resources/:id | Single resource status detail |
| GET | /resources/events | **SSE** — live stream of `ResourceEvent` (one event per line, JSON) |
| POST | /resources/:id/drain | Admin: drain all pool instances, block new acquires until complete |

**SSE event shape:**

```json
{ "type": "HealthChanged", "resource_id": "postgres", "from": "Healthy", "to": "Degraded" }
{ "type": "Acquired",      "resource_id": "redis",    "wait_duration_ms": 12 }
{ "type": "PoolExhausted", "resource_id": "postgres", "waiters": 3 }
{ "type": "Quarantined",   "resource_id": "redis",    "reason": "health check failed 3 times" }
```

Desktop connects with `EventSource('/resources/events')`, updates TanStack Query cache on each event
without polling. Each SSE connection subscribes independently to `Manager::event_bus()`.

## Usage Patterns

### Run server (blocking)

```rust
use nebula_api::{ApiServerConfig, WorkerStatus, run};
use nebula_webhook::WebhookServerConfig;

let api_config = ApiServerConfig {
    bind_addr: "127.0.0.1:5678".parse().unwrap(),
};

let webhook_config = WebhookServerConfig {
    bind_addr: api_config.bind_addr,
    base_url: "http://127.0.0.1:5678".into(),
    path_prefix: "/webhooks".into(),
    enable_compression: true,
    enable_cors: true,
    body_limit: 10 * 1024 * 1024,
};

let workers = vec![
    WorkerStatus { id: "wrk-1".into(), status: "active".into(), queue_len: 2 },
    WorkerStatus { id: "wrk-2".into(), status: "idle".into(), queue_len: 0 },
];

run(api_config, webhook_config, workers).await?;
```

### Build app for custom serve

```rust
use nebula_api::app;
use nebula_webhook::WebhookServer;

let webhook = WebhookServer::new_embedded(webhook_config)?;
let app = app(webhook, workers);
// axum::serve(listener, app).await?;
```

### ApiServer (alternative)

```rust
let server = ApiServer::new(ApiServerConfig::default());
let app = server.app(webhook, workers);
```

## Minimal Example

```bash
cargo run -p nebula-api --example unified_server
```

Then:
- `GET http://127.0.0.1:5678/health` → OK
- `GET http://127.0.0.1:5678/api/v1/status` → JSON
- `POST http://127.0.0.1:5678/webhooks/...` → webhook (when registered)

## Error Semantics

- **ApiError::Webhook:** WebhookServer::new_embedded failed.
- **ApiError::Io:** bind or serve failed (address in use, etc.).

## Compatibility Rules

- **Major version bump:** Route removal; StatusResponse schema change.
- **Deprecation policy:** Minimum 2 minor releases.
