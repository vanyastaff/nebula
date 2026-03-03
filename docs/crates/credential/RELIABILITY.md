# Reliability

## SLO Targets

### Latency

| Operation | Cache Hit (p95) | Cache Miss (p95) | Target |
|-----------|-----------------|-------------------|--------|
| Get Credential | <10ms | <100ms | Production |
| Store Credential | N/A | <150ms | Production |
| Rotate Credential | N/A | <500ms | Production |
| Delete Credential | N/A | <100ms | Production |
| Encrypt/Decrypt | N/A | <5ms | Production |
| Key Derivation (Argon2id) | N/A | <50ms | Production |

### Throughput

| Metric | Target | Notes |
|--------|--------|-------|
| Operations/sec (single instance) | 10,000 | Cache hit heavy workload |
| Operations/sec (cluster) | 100,000 | Linear scaling with node count |
| Concurrent connections | 1,000 | Per instance |
| Cache hit ratio | >80% | Production workload |

### Provider-Specific Latency

| Provider | Read p95 | Write p95 | Notes |
|----------|----------|-----------|-------|
| Local (filesystem) | <1ms | <5ms | Single-node only |
| Kubernetes Secrets | 5–15ms | 10–30ms | Namespace-isolated |
| HashiCorp Vault | 10–50ms | 20–80ms | KV v2 + Transit |
| Azure Key Vault | 30–80ms | 50–120ms | HSM-backed |
| AWS Secrets Manager | 50–100ms | 80–150ms | KMS-encrypted |

## Failure Modes

### Dependency Outage

- **AWS/Vault/K8s unreachable:** Storage operations return `StorageError::Timeout`. Cache may serve stale data (TTL-dependent). Rotation pauses.
- **Redis unavailable (L2 cache):** Falls back to L1 in-memory cache only. Higher storage load.
- **KMS unavailable:** Cannot derive new encryption keys. Existing cached credentials still usable. New store/rotate operations fail.

### Timeout and Backpressure

- Storage provider timeout: default 30s, returns `StorageError::Timeout`
- No retry at credential crate level — caller/resilience crate may retry idempotent operations (retrieve, list, exists)
- Non-idempotent operations (store, delete) must not be retried blindly

### Partial Degradation

- Batch operations: `BatchError` with partial success; caller decides retry strategy
- Individual failures within batch do not roll back successful items

### Data Corruption

- Decryption failure: `CryptoError::DecryptionFailed` — never return partial data (fail-secure)
- Tampered ciphertext: AES-GCM authentication tag detects modification
- No recovery path — credential must be re-created or restored from backup

## Resilience Strategies

### Caching

- **L1 (in-memory):** Moka cache, TTL-based with LRU eviction, per-node
- **L2 (Redis, future):** Shared across fleet, longer TTL (~30 min), reduces storage load
- **Negative cache:** Failed lookups cached briefly to avoid repeated storage hits
- **Invalidation:** On credential update/delete, cache entry evicted immediately

### Retry Policy

- Not applied at credential crate level (by design)
- Caller/resilience crate may wrap `StorageProvider` with retry logic
- Safe to retry: `retrieve`, `list`, `exists` (idempotent)
- Unsafe to retry: `store`, `delete` (non-idempotent without CAS)

### Circuit Breaking

- N/A at crate level; `nebula-resilience` may wrap provider
- Provider health check via `StorageMetrics` latency/error tracking

### Graceful Degradation

| Scenario | Behavior |
|----------|----------|
| Provider unavailable | Fail operation; cache serves if TTL allows |
| Cache hit | Serve from cache; skip storage |
| Rotation failure | Rollback to previous credential; alert operator |
| Encryption key unavailable | Fail new operations; cached credentials still usable |

### Rotation Resilience

- `RotationTransaction` implements backup → new → grace period → revoke atomically
- Failure at any phase → rollback to backup credential
- Grace period: old credential remains valid, no in-flight request fails
- Jitter on periodic rotation prevents thundering herd (multiple credentials rotating simultaneously)
- Blue-green rotation: instant rollback capability, zero downtime

## Operational Runbook

### Alert Conditions

| Alert | Threshold | Severity | Action |
|-------|-----------|----------|--------|
| Decryption failure rate | > 1% | Critical | Verify encryption key; check for tampering |
| Scope violation spike | > 5/min | Critical | Review access patterns; possible attack |
| Storage timeout rate | > 5% | High | Check backend health; scale resources |
| Cache hit ratio drop | < 60% | Warning | Increase cache size/TTL; check eviction |
| Rotation failure | Any | High | Check provider; verify credentials; inspect logs |
| Expired credentials | > 0 | Critical | Trigger immediate refresh/rotation |
| Grace period violations | > 0 | Warning | Increase grace period duration |

### Dashboards

- **Operations:** Credential fetch latency (p50/p95/p99); cache hit rate; rotation success/failure; error rate by provider
- **Security:** Scope violations; decryption failures; manual rotations; audit log coverage
- **Capacity:** Connection pool usage; cache size; storage operation count; encryption throughput

### Incident Triage

1. Check storage backend health (provider connectivity, latency)
2. Verify encryption key availability and correctness
3. Inspect scope context propagation (is `CredentialContext` properly populated?)
4. Review audit logs for unusual access patterns
5. Check cache state (stale entries, size pressure)

## Capacity Planning

### Load Profile

- **Bursty:** High credential fetch volume during workflow execution waves
- **Steady:** Background rotation checks, token refresh, cache maintenance
- **Peak:** Workflow batch execution (10K+ credential fetches in minutes)

### Scaling Constraints

| Resource | Constraint | Mitigation |
|----------|-----------|------------|
| Cache size | Memory per node | Configurable `max_size`; LRU eviction |
| Storage provider | AWS/Vault rate limits | Caching; request batching |
| Lock contention | Rotation parallelism | Per-credential locks; jitter |
| Encryption CPU | Argon2id memory cost | Cache derived keys; limit concurrent derivations |
| Connection pool | DB/HTTP connection limits | Pool sizing; backpressure |

### Sizing Guidelines

| Deployment Size | Credentials | Cache Size | Storage | Notes |
|-----------------|-------------|------------|---------|-------|
| Small (dev) | <100 | 1,000 | Local filesystem | Single node |
| Medium (team) | 100–1K | 10,000 | Local or Vault | Single node or small cluster |
| Large (enterprise) | 1K–100K | 50,000 | AWS/Vault/K8s | Multi-node with L2 cache |
| Scale (SaaS) | 100K+ | 100,000+ | Distributed store + Redis L2 | Fleet with distributed lock |

## Performance Optimization Checklist

### Development

- [ ] Enable caching with appropriate TTL (default: 5 min)
- [ ] Use batch operations for multiple credential fetches
- [ ] Profile hot paths with `tracing` spans
- [ ] Add database indexes (owner, scope, expiration)

### Deployment

- [ ] Configure cache size based on credential count
- [ ] Tune connection pool size for storage backend
- [ ] Enable jitter on periodic rotation policies
- [ ] Set up latency percentile monitoring (p50, p95, p99)
- [ ] Configure alerting for performance degradation
- [ ] Regular load testing with criterion benchmarks

### Benchmarks

```bash
# Run credential operation benchmarks
cargo bench -p nebula-credential

# Expected results:
# get_credential/cache_hit    ~8ms p95
# get_credential/cache_miss   ~90ms p95
# encrypt_decrypt             ~3ms p95
# key_derivation              ~40ms p95
```
