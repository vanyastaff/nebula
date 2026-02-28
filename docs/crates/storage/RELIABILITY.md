# Reliability

## SLO Targets

| Metric | Target | Notes |
|--------|--------|-------|
| **Availability** | Backend-dependent | Postgres/Redis/S3 have their own SLA |
| **Latency** | get/set < 10ms (local) | Configurable timeouts |
| **Error budget** | Backend-dependent | Storage propagates backend errors |

## Failure Modes

### Dependency Outage

- **Postgres down:** All ops fail with Backend error; no fallback
- **Redis down:** Same
- **S3 down:** Same
- **MemoryStorage:** No external deps; always available (until process exit)

### Timeout/Backpressure

- **Connection pool exhausted:** Postgres/Redis may block or fail; pool config critical
- **S3 rate limit:** Backend error; retry with backoff at consumer
- **No built-in timeout:** Consumers or backend client config

### Partial Degradation

- **One backend down:** If multi-backend (e.g. cache + persistent), cache may serve stale
- **Disk full (Postgres):** Backend error; no partial write

### Data Corruption

- **Serialization error:** StorageError::Serialization; value not written/read
- **Backend corruption:** Out of scope; backend responsibility

## Resilience Strategies

### Retry Policy

- Storage does not retry. Consumer or wrapper may retry on Backend error.
- Transient failures (connection reset): retry with backoff.
- Serialization: no retry (data issue).

### Circuit Breaking

- N/A at storage level. Consumer or resilience layer may use circuit breaker for backend calls.

### Fallback Behavior

- **MemoryStorage:** No fallback; in-memory only.
- **Multi-tier:** Consumer could use MemoryStorage as cache, Postgres as primary; on Postgres failure, serve from cache (stale).

### Graceful Degradation

- Read-only mode: if backend allows; storage does not implement.
- Degraded list: if list fails, return partial or error.

## Operational Runbook

### Alert Conditions

- Backend connection failures
- Pool exhaustion
- High latency (p99)

### Dashboards

- Backend connection count
- get/set/delete rate
- Error rate by type

### Incident Triage Steps

1. Check backend health (Postgres, Redis, S3)
2. Check connection pool usage
3. Check for serialization errors (bad data?)
4. Check consumer retry/backoff

## Capacity Planning

### Load Profile Assumptions

- get >> set for read-heavy (workflow load, execution load)
- set rate during execution writes
- Key space: workflow count, execution count, etc.

### Scaling Constraints

- **MemoryStorage:** Single process; lost on restart
- **Postgres:** Connection pool; consider read replicas
- **Redis:** Connection pool; Redis Cluster for sharding
- **S3:** Request rate; consider multipart for large values
