[← Previous Page](configuration.md) · [Back to README](../README.md) · [Next Page →](PROJECT_STATUS.md)

# Deployment

The repository includes a local Docker Compose stack for infrastructure bring-up and a `deploy/`
directory for container and Kubernetes material.

## Local Compose Stack

Run the local stack from the repository root:

```bash
docker compose -f deploy/docker/docker-compose.yml up -d
```

Enable Redis as an optional cache profile:

```bash
docker compose -f deploy/docker/docker-compose.yml --profile cache up -d
```

## Services

| Service | Purpose | Default Port |
|---------|---------|--------------|
| PostgreSQL 16 | Primary local database | `5432` |
| Redis 7 | Optional cache/profile | `6379` |

## Deployment Notes

- PostgreSQL uses a named Docker volume for persistence.
- Redis is optional and only starts when the `cache` profile is enabled.
- Health checks are configured for Postgres via `pg_isready`.
- Additional deployment artifacts live under `deploy/docker/` and `deploy/kubernetes/`.

## Suggested Runtime Validation

After the infra stack starts, verify the API separately with:

```bash
curl http://localhost:8080/health
curl http://localhost:8080/ready
```

## Developer Runbook (Local)

Use this runbook for local end-to-end bring-up.

```bash
# 1) Bring up infra dependencies
docker compose -f deploy/docker/docker-compose.yml up -d

# 2) Verify containers are healthy
docker compose -f deploy/docker/docker-compose.yml ps

# 3) Start Nebula API process
cargo run -p nebula-api

# 4) Validate runtime status
curl -i http://localhost:8080/health
curl -i http://localhost:8080/ready
```

If you do not need Redis for your current task, keep cache profile disabled.

## Troubleshooting Quick Table

| Symptom | Likely Cause | Action |
|--------|---------------|--------|
| Postgres container keeps restarting | Local port clash or bad volume state | Check `docker compose ... logs postgres`, free port `5432`, recreate container/volume if needed |
| API process starts but `/ready` fails | Dependency not reachable or not initialized | Verify compose services and check API logs for dependency checks |
| `curl /health` fails connection | API not running or wrong port | Re-run `cargo run -p nebula-api`, verify bind port |
| Redis missing in local tests | Cache profile not enabled | Start compose with `--profile cache` |

## Definition of Done for Deployment-Sensitive Changes

1. Clean bring-up from stopped state is documented and repeatable.
2. Health and readiness are explicitly validated.
3. Any new dependency is reflected in deploy artifacts and docs.
4. Failure mode and rollback steps are described in PR notes.

## See Also

- [Configuration](configuration.md) - Environment variables used by services
- [API Reference](api.md) - Health and readiness endpoints
- [Project Status](PROJECT_STATUS.md) - Current operational maturity