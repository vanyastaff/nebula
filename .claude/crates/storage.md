# nebula-storage
Storage trait abstraction — MemoryStorage for tests, PostgresStorage for production.

## Invariants
- `MemoryStorage` is for development and tests only. All data is lost on process restart. Never use in production.
- `Storage` trait is the generic key-value contract. `ExecutionRepo` and `WorkflowRepo` are domain-specific repository traits on top.
- `WorkflowRepo::save` uses CAS (compare-and-swap) semantics: callers pass the expected version, and the repo rejects with `VersionConflict` if the stored version differs. Version 0 means "new workflow" (INSERT); non-zero means "update existing" (UPDATE … WHERE version = $expected).

## Key Decisions
- `Storage<Key, Value>` is generic — typed by key and value. `MemoryStorageTyped` provides a typed wrapper.
- `ExecutionRepo` / `InMemoryExecutionRepo` for execution state. `WorkflowRepo` / `InMemoryWorkflowRepo` for workflow definitions.
- `PgWorkflowRepo` is the PostgreSQL-backed `WorkflowRepo`. Constructor accepts `Pool<Postgres>` (not a connection string) — obtain the pool from `PostgresStorage::pool()`.
- `PgWorkflowRepo::list` orders by `created_at, id` for deterministic pagination.
- PostgreSQL backend behind `postgres` feature; Redis and S3 planned but not implemented.
- lib.rs is in Russian — intentional, matches early project language. Do not translate.

## Traps
- `StorageError` is distinct from `ExecutionRepoError` and `WorkflowRepoError` — each layer has its own error type.
- `PostgresStorage` requires the `postgres` feature flag and a database connection string.
- `PgWorkflowRepo` integration tests use a test-guard pattern: `pg_repo()` reads `DATABASE_URL` and returns `Option`; tests skip (pass) when no database is available.

## Relations
- Depends on nebula-core (IDs). Used by nebula-engine, nebula-api.

<!-- reviewed: 2026-03-30 — derive Classify migration -->
