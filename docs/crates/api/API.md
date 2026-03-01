# API

## Public Surface

### Stable APIs

- `app(webhook_server, workers)` → Router
- `run(config, webhook_config, workers)` → `impl Future<Output = Result<(), ApiError>>`
- `ApiServer` — `new(config)`, `app(webhook, workers)`
- `ApiServerConfig` — `bind_addr`; Default: 0.0.0.0:5678
- `ApiError` — Webhook(webhook::Error), Io(std::io::Error)
- `WorkerStatus` — id, status, queue_len
- `WebhookStatus` — status, route_count, paths; `from_server(server)`
- `StatusResponse` — workers, webhook (internal; JSON response)

### Routes

#### Infrastructure (stable)

| Method | Path | Description |
|--------|------|-------------|
| GET | /health | Liveness; 200 OK |
| GET | /api/v1/status | JSON { workers, webhook } |
| POST | /webhooks/* | Webhook endpoints (from nebula-webhook) |

#### Auth (implemented)

| Method | Path | Description |
|--------|------|-------------|
| POST | /auth/oauth/start | Begin OAuth flow; body `{ provider, redirectUri }`; returns `{ authUrl }` |
| POST | /auth/oauth/callback | Exchange code; body `{ provider, code, redirectUri }`; returns `{ accessToken, user }` |

#### Workflows *(Phase 2)*

| Method | Path | Description |
|--------|------|-------------|
| GET | /workflows | List workflows |
| GET | /workflows/:id | Get workflow by ID |
| POST | /workflows | Create workflow |
| PATCH | /workflows/:id | Update workflow |
| DELETE | /workflows/:id | Delete workflow |
| POST | /workflows/:id/activate | Activate / deactivate workflow |
| POST | /workflows/:id/execute | Manual trigger (test run) |

#### Runs *(Phase 2–3)*

| Method | Path | Description |
|--------|------|-------------|
| GET | /runs | List runs (filter by workflow, status, date) |
| GET | /runs/:id | Get run detail + node-by-node trace |
| GET | /runs/:id/logs | Stream execution logs (WebSocket upgrade or SSE) |

#### Credentials *(Phase 4)*

| Method | Path | Description |
|--------|------|-------------|
| GET | /credentials | List credentials |
| POST | /credentials | Create credential |
| DELETE | /credentials/:id | Delete credential |

#### Nodes *(Phase 4)*

| Method | Path | Description |
|--------|------|-------------|
| GET | /nodes | List available node types |
| GET | /nodes/:type | Get node definition + parameter schema |

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
