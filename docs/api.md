[← Previous Page](ARCHITECTURE.md) · [Back to README](../README.md) · [Next Page →](configuration.md)

# API Reference

Nebula exposes an Axum-based HTTP API. Current routes are grouped under `/api/v1`, with health
and readiness checks exposed at the root.

## Base Paths

| Path | Purpose |
|------|---------|
| `/health` | Liveness check |
| `/ready` | Readiness check |
| `/api/v1` | Versioned application API |

## Authentication

- Health and readiness endpoints do not require authentication.
- JWT-style authorization is planned via API middleware.
- Current middleware checks for an `Authorization` header and returns `401 Unauthorized` when it is missing.

## Endpoints

### Health

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Returns status, version, and timestamp |
| `GET` | `/ready` | Returns readiness and dependency status |

### Workflows

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/workflows` | List workflows |
| `POST` | `/api/v1/workflows` | Create workflow |
| `GET` | `/api/v1/workflows/{id}` | Get workflow by id |
| `PUT` | `/api/v1/workflows/{id}` | Update workflow |
| `DELETE` | `/api/v1/workflows/{id}` | Delete workflow |

### Executions

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/workflows/{workflow_id}/executions` | List executions for workflow |
| `POST` | `/api/v1/workflows/{workflow_id}/executions` | Start workflow execution |
| `GET` | `/api/v1/executions/{id}` | Get execution by id |
| `POST` | `/api/v1/executions/{id}/cancel` | Cancel execution |

## Example Requests

```bash
curl http://localhost:8080/health

curl http://localhost:8080/api/v1/workflows
```

## Local API Smoke Test (Developer)

Use this flow when you changed handlers, routing, middleware, or request/response contracts.

```bash
# 1) Start infra (if needed by your change)
docker compose -f deploy/docker/docker-compose.yml up -d

# 2) Start API
cargo run -p nebula-api

# 3) Verify basic availability
curl -i http://localhost:8080/health
curl -i http://localhost:8080/ready

# 4) Verify auth guard behavior (placeholder middleware)
curl -i http://localhost:8080/api/v1/workflows
curl -i -H "Authorization: Bearer dev-token" http://localhost:8080/api/v1/workflows
```

Expected result right now:

1. `/health` and `/ready` return success.
2. Protected route without `Authorization` returns `401 Unauthorized`.
3. Protected route with header reaches route handler path.

## Where to Change What

1. New route shape or version prefix: `crates/api/src/routes/`.
2. Request/response and handler logic: `crates/api/src/handlers/`.
3. Cross-cutting auth behavior: middleware layer in API crate.
4. Contract-level docs updates: this file and related crate docs.

## Definition of Done for API Changes

1. Route behavior is covered by tests or explicit reproducible curl commands.
2. Error paths are intentional and documented.
3. Backward compatibility is preserved unless change is explicitly breaking.
4. Docs reflect the final endpoint shape and auth expectations.

## Common Failure Modes During Development

1. `401` everywhere: missing `Authorization` header on protected routes.
2. Health works but business routes fail: route registration mismatch in `routes/`.
3. Handler compiles but serialization fails: response/request model drift.
4. Endpoint exists but wrong method returns `405`: route method mismatch.

## Notes

- Route shape is defined in `crates/api/src/routes/`.
- Health responses are implemented in `crates/api/src/handlers/health.rs`.
- Authentication behavior is currently a minimal middleware placeholder, not a complete JWT validation flow.

## See Also

- [Architecture](ARCHITECTURE.md) - API layer placement in the workspace
- [Configuration](configuration.md) - API environment variables and defaults
- [Deployment](deployment.md) - Local infra and service startup