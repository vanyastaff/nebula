# Interactions

## Ecosystem Map (Current + Planned)

### Existing Crates

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-execution` | Upstream | IdempotencyKey, IdempotencyManager, NodeAttempt, DuplicateIdempotencyKey |
| `nebula-core` | Upstream | ExecutionId, NodeId for key generation |
| `nebula-engine` | Consumer | Uses execution types; may integrate check_and_mark |
| `nebula-resilience` | Sibling | Operation::IDEMPOTENT for retry policy |
| `nebula-credential` | Consumer | create_credential_idempotent (user key) |
| `nebula-storage` | Planned dep | Key-value backend for persistent idempotency |
| `nebula-api` | Consumer | HTTP Idempotency-Key layer (planned) |

### Planned Crates

- **nebula-idempotency:** Standalone crate extracting from execution; adding storage, HTTP layer, action trait

## Downstream Consumers

### nebula-execution (internal)

- **Expectations:** IdempotencyKey in NodeAttempt; IdempotencyManager for per-execution dedup
- **Contract:** Sync key generation; check_and_mark mutates manager

### nebula-credential

- **Expectations:** User-provided idempotency_key for create_credential_idempotent
- **Contract:** Optional key; dedup at credential creation

### nebula-api (target)

- **Expectations:** IdempotencyLayer for Idempotency-Key header; response caching
- **Contract:** Async; storage backend

## Upstream Dependencies

| Crate | Why needed | Hard contract | Fallback |
|-------|------------|---------------|----------|
| `nebula-core` | ExecutionId, NodeId | ID format | — |
| `nebula-storage` | Persistent backend (planned) | Key-value trait | In-memory |
| `serde` | Key serialization | — | — |

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|---------------------|-----------|----------|------------|------------------|-------|
| execution -> idempotency | in | IdempotencyKey, IdempotencyManager | sync | DuplicateIdempotencyKey | Current: in execution |
| engine -> execution | in | NodeAttempt with key | sync | — | Journaling |
| credential -> idempotency | in | User key (planned) | async | — | create_credential_idempotent |
| api -> idempotency | in | IdempotencyLayer (planned) | async | Return 409 or cached | HTTP |
| idempotency -> storage | out | Key-value (planned) | async | Retry/fail | Backend |

## Runtime Sequence

1. Engine creates NodeAttempt with IdempotencyKey::generate(exec_id, node_id, attempt).
2. Before executing node: check IdempotencyManager::check_and_mark; if false, skip or return cached.
3. On success: result associated with key (future: persist).
4. On duplicate: ExecutionError::DuplicateIdempotencyKey or return cached.

## Cross-Crate Ownership

| Responsibility | Owner |
|----------------|-------|
| Key generation (node-level) | `nebula-execution` |
| In-memory dedup | `nebula-execution` |
| Persistent storage | `nebula-idempotency` (planned) or storage |
| HTTP Idempotency-Key | `nebula-idempotency` (planned) |
| Action-level config | `nebula-idempotency` (planned) |
| DB schema (idempotency_keys) | Migrations |

## Failure Propagation

- **DuplicateIdempotencyKey:** Caller returns cached or rejects
- **Storage failure (future):** Retry or fail request; never double-execute

## Versioning and Compatibility

- **Key format:** Stable; change requires major bump
- **Breaking-change protocol:** Migration guide; deprecation window

## Contract Tests Needed

- [ ] IdempotencyKey::generate deterministic for same inputs
- [ ] check_and_mark returns true once, false on duplicate
- [ ] NodeAttempt roundtrip with idempotency_key
- [ ] DuplicateIdempotencyKey error message
