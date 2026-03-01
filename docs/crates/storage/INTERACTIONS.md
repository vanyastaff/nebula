# Interactions

## Ecosystem Map (Current + Planned)

### Existing Crates

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-core` | Upstream | Id types (indirect; storage is generic) |
| `nebula-credential` | Sibling | Has own `StorageProvider` trait — separate abstraction for encrypted credentials; does NOT use `Storage` trait |
| `nebula-execution` | Potential consumer | May use storage for execution state (future) |
| `nebula-workflow` | Potential consumer | May use storage for workflow definitions (future) |

### Planned / Potential Consumers

| Crate | Relationship | Description |
|-------|-------------|-------------|
| `nebula-execution` | Downstream | ExecutionRepo impl using Storage |
| `nebula-workflow` | Downstream | WorkflowRepo impl using Storage |
| `nebula-idempotency` | Downstream | IdempotencyStorage impl using Storage |
| `nebula-credential` | Sibling | `StorageProvider` trait is a separate, richer abstraction (handles encryption, rotation); may share backend infra in future |

## Downstream Consumers

### Potential: execution, workflow, idempotency

- **Expectations:** `Storage<Key, Value>` or `MemoryStorageTyped<T>`; get/set/delete/exists
- **Contract:** Async; Result<Option<V>, StorageError> for get
- **Usage:** Key format (e.g. `workflow:{id}`, `execution:{id}`) defined by consumer

## Upstream Dependencies

| Crate | Why needed | Hard contract | Fallback |
|-------|------------|---------------|----------|
| `async-trait` | Async trait methods | — | — |
| `serde` | Serialization for typed storage | — | — |
| `serde_json` | MemoryStorageTyped | — | — |
| `tokio` | RwLock for MemoryStorage | — | — |
| `thiserror` | StorageError | — | — |
| `nebula-core` | (minimal) | — | — |
| `sqlx` | postgres feature | — | Optional |
| `redis` | redis feature | — | Optional |
| `aws-sdk-s3` | s3 feature | — | Optional |

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|----------------------|-----------|----------|------------|------------------|-------|
| storage -> consumer | out | Storage trait | async | Result<StorageError> | Consumer chooses Key/Value |
| storage -> credential | — | None | — | — | Credential has StorageProvider; different abstraction |
| storage -> core | in | (minimal) | — | — | Core for ids if needed |

## Runtime Sequence

1. Application constructs Storage backend (MemoryStorage, or PostgresStorage when implemented).
2. Consumer (e.g. WorkflowRepo) receives `Arc<dyn Storage<WorkflowId, Workflow>>` or `MemoryStorageTyped<Workflow>`.
3. On save: consumer calls `store.set(&key, &value).await`.
4. On load: consumer calls `store.get(&key).await` → Option<Value>.
5. On delete: consumer calls `store.delete(&key).await`.

## Cross-Crate Ownership

| Responsibility | Owner |
|----------------|-------|
| Storage trait, StorageError | `nebula-storage` |
| Key format, value schema | Consumer (workflow, execution, etc.) |
| Backend implementations | `nebula-storage` |
| Credential storage | `nebula-credential` (StorageProvider) |
| Migrations | Repo-root migrations/ |

## Failure Propagation

- **StorageError:** Propagates to consumer; consumer decides retry/fallback.
- **Serialization:** From serde_json in MemoryStorageTyped; non-retryable for invalid data.
- **Backend:** Connection/timeout; may be retryable.

## Versioning and Compatibility

- **Compatibility promise:** Storage trait additive-only; new methods optional.
- **Breaking-change protocol:** Major version bump.
- **Deprecation window:** Minimum 2 minor releases.

## Contract Tests Needed

- [ ] MemoryStorage get/set/delete/exists
- [ ] MemoryStorageTyped roundtrip for serde types
- [ ] StorageError variants
- [ ] (Future) PostgresStorage, RedisStorage, S3Storage integration
