[← Previous Page](api.md) · [Back to README](../README.md) · [Next Page →](deployment.md)

# Configuration

Runtime configuration for the Nebula API and local infrastructure.
All values have sensible development defaults — override in production via environment variables.

Source of truth: [`crates/api/src/config.rs`](../crates/api/src/config.rs).

## API Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `API_BIND_ADDRESS` | `0.0.0.0:8080` | Host and port for the API server |
| `API_REQUEST_TIMEOUT` | `30` | Request timeout in seconds |
| `API_MAX_BODY_SIZE` | `2097152` | Maximum request body size in bytes (~2 MB) |
| `API_CORS_ORIGINS` | `*` | Comma-separated allowed CORS origins |
| `API_ENABLE_COMPRESSION` | `true` | Enable gzip/brotli/zstd response compression |
| `API_ENABLE_TRACING` | `true` | Enable request tracing spans |
| `API_JWT_SECRET` | `dev-secret-change-in-production` | JWT signing secret (not fully wired yet) |
| `API_RATE_LIMIT` | `100` | Requests per second per IP |

## Local Infrastructure Variables

Used by `deploy/docker/docker-compose.yml`.

| Variable | Default | Description |
|----------|---------|-------------|
| `POSTGRES_USER` | `nebula` | Postgres username |
| `POSTGRES_PASSWORD` | `nebula` | Postgres password |
| `POSTGRES_DB` | `nebula` | Postgres database name |
| `POSTGRES_PORT` | `5432` | Published Postgres port |
| `REDIS_PORT` | `6379` | Published Redis port |

## Quick Override (Development)

Create a `.env` file in the repo root or export variables before running:

```bash
# Change API port to 9090
export API_BIND_ADDRESS=0.0.0.0:9090

# Increase body size for large payloads (10 MB)
export API_MAX_BODY_SIZE=10485760

# Restrict CORS to local frontend
export API_CORS_ORIGINS=http://localhost:5173

cargo run -p nebula-api
```

Alternatively set variables in `.env` (loaded by the compose stack, not by `cargo run` directly).

## Production Checklist

Before deploying, verify these overrides are applied:

- [ ] `API_JWT_SECRET` — set to a strong random value, not the default.
- [ ] `API_CORS_ORIGINS` — restrict to your domain(s), never `*`.
- [ ] `API_RATE_LIMIT` — tune per your expected traffic.
- [ ] `POSTGRES_PASSWORD` — use a strong password or managed credentials.
- [ ] `API_BIND_ADDRESS` — bind to `127.0.0.1` if behind a reverse proxy.

## How Configuration Loads

```
Environment variable → ApiConfig::from_env() → ApiConfig struct → used by router/server
                                ↑
                        Missing? Uses Default impl
```

`ApiConfig` implements `Default` and `from_env()`. All fields fall back to default values
when the corresponding environment variable is absent. There is no config file format yet —
everything is environment-driven.

## See Also

- [API Reference](api.md) — endpoints that consume this configuration
- [Deployment](deployment.md) — compose-based local startup