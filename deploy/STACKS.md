# Nebula Deployment Modes (n8n-style)

This file defines a practical deployment blueprint for three modes:
- Local
- Self-Hosted
- SaaS

All modes are Postgres-first to keep behavior consistent across environments.

## 1. Local (developer laptop)

Goal: quick startup with production-like persistence model.

Recommended stack:
- `postgres`
- optional `redis` (if queue/cache is enabled)
- app binaries run from local workspace (`cargo run ...`)

Compose:
- `deploy/docker/docker-compose.yml`

Command:

```bash
docker compose -f deploy/docker/docker-compose.yml up -d
```

Then run app from source:

```bash
cargo run -p nebula-api --bin unified_server
```

## 2. Self-Hosted (single node / small team)

Goal: one-command server deployment with separate app processes.

Recommended stack:
- `postgres`
- `redis`
- `nebula-api` (includes embedded worker loop via `NEBULA_WORKER_COUNT`)

Compose:
- `deploy/docker/docker-compose.selfhosted.yml`

Command:

```bash
docker compose -f deploy/docker/docker-compose.selfhosted.yml up -d --build
```

Notes:
- By default, compose builds `nebula-api` from source (`deploy/docker/Dockerfile.api`).
- You can override with a prebuilt image via `NEBULA_API_IMAGE`.

## 3. SaaS (managed multi-tenant)

Goal: horizontally scalable baseline.

Recommended baseline:
- managed Postgres (external)
- managed Redis (external)
- multiple `nebula-api` replicas
- ingress/load balancer + observability + secrets management

Blueprint compose (local simulation only):
- `deploy/docker/docker-compose.saas.blueprint.yml`

Use this as architecture reference, not as final production orchestration.
Production target should be Kubernetes/ECS/Nomad with managed services.

## Minimum environment contract

- `DATABASE_URL`
- `REDIS_URL` (when queue/cache enabled)
- `NEBULA_API_BIND` (e.g. `0.0.0.0:5678`)
- `NEBULA_WORKER_COUNT`
- `RUST_LOG` or `NEBULA_LOG`

Telemetry/Sentry optional:
- `OTEL_EXPORTER_OTLP_ENDPOINT`
- `SENTRY_DSN`
