# Reliability

## SLO Targets

- **Availability:** Idempotency check must not block execution. Storage failure: fail open (execute) or fail closed (reject) — configurable.
- **Latency:** check_and_mark < 1ms (in-memory); < 5ms (Postgres).
- **Correctness:** No duplicate execution for same key; no false negatives (missed duplicate).

## Failure Modes

| Mode | Impact | Mitigation |
|------|--------|------------|
| Storage unavailable | Cannot check/store keys | Fail open (execute) or fail closed; retry |
| Key collision (hash) | False duplicate | Use UUIDs or larger key space; document |
| TTL too short | Duplicate after expiry | Configure 24h+ for critical ops |
| Memory manager lost | Restart loses state | Persistent storage for production |

## Resilience Strategies

- **Retry policy:** Storage ops retry with backoff; idempotent storage ops.
- **Circuit breaking:** Optional for storage; stop checks after repeated failures.
- **Fallback behavior:** In-memory fallback when storage down (degraded dedup).
- **Graceful degradation:** Fail open: execute without dedup when storage unavailable.

## Operational Runbook

- **Alert conditions:** Storage errors; high duplicate rate; cache size.
- **Dashboards:** Dedup hit rate; storage latency; key count.
- **Incident triage:** Duplicate execution → check key generation, storage health.

## Capacity Planning

- **Load profile assumptions:** 1k–10k keys/sec; 24h TTL; cleanup job.
- **Scaling constraints:** Single storage backend; sharding for scale (Phase 2).
