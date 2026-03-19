# nebula-storage
Storage trait abstraction — MemoryStorage for tests, PostgresStorage for production.

## Invariants
- `MemoryStorage` is for development and tests only. All data is lost on process restart. Never use in production.
- `Storage` trait is the generic key-value contract. `ExecutionRepo` and `WorkflowRepo` are domain-specific repository traits on top.

## Key Decisions
- `Storage<Key, Value>` is generic — typed by key and value. `MemoryStorageTyped` provides a typed wrapper.
- `ExecutionRepo` / `InMemoryExecutionRepo` for execution state. `WorkflowRepo` / `InMemoryWorkflowRepo` for workflow definitions.
- PostgreSQL backend behind `postgres` feature; Redis and S3 planned but not implemented.
- lib.rs is in Russian — intentional, matches early project language. Do not translate.

## Traps
- `StorageError` is distinct from `ExecutionRepoError` and `WorkflowRepoError` — each layer has its own error type.
- `PostgresStorage` requires the `postgres` feature flag and a database connection string.

## Relations
- Depends on nebula-core (IDs). Used by nebula-engine, nebula-api.
