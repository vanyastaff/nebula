# Reliability

## SLO Targets

- **availability:** Credential fetch p99 < 500ms (local); < 2s (AWS/Vault); cache hit p99 < 10ms
- **latency:** Manager operations: p50 < 50ms, p99 < 500ms (storage-dependent)
- **error budget:** Storage unavailability degrades to cache-only (if enabled); crypto failures are fatal (fail secure)

## Failure Modes

- **dependency outage:** AWS/Vault/K8s unreachable — storage operations fail; cache may serve stale (TTL-dependent)
- **timeout/backpressure:** Storage provider timeout (default 30s) — return `StorageError::Timeout`; no retry at crate level
- **partial degradation:** Batch operation — `BatchError` with partial success; caller decides retry strategy
- **data corruption:** Decryption failure — `CryptoError::DecryptionFailed`; never return partial data

## Resilience Strategies

- **retry policy:** Not applied at credential crate; caller/resilience crate may retry storage operations (idempotent: retrieve, list, exists)
- **circuit breaking:** N/A at crate level; resilience crate may wrap provider
- **fallback behavior:** Cache serves on storage miss if TTL allows; rotation rollback on failure
- **graceful degradation:** Provider unavailable → fail; cache hit → serve; rotation failure → keep old credential, alert

## Operational Runbook

- **alert conditions:** Decryption failure rate > 1%; scope violation spike; storage timeout rate > 5%
- **dashboards:** Credential fetch latency; cache hit rate; rotation success/failure; error rate by provider
- **incident triage:** Check storage backend health; verify encryption key; inspect scope context propagation

## Capacity Planning

- **load profile assumptions:** Bursty during workflow execution; steady background rotation
- **scaling constraints:** Cache size (configurable); storage provider limits (AWS/Vault rate limits); lock contention in rotation
