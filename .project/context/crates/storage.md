# nebula-storage
Storage trait abstraction — MemoryStorage for tests, PostgresStorage for production.

## Invariants
- `MemoryStorage` is dev/test only — all data lost on restart.
- `WorkflowRepo::save` uses CAS: version 0 = INSERT, non-zero = UPDATE WHERE version = $expected.
- `ExecutionRepo::create` inserts with version=1, fails `Conflict` if ID exists. `transition` uses CAS.
- `ExecutionRepo::append_journal` requires a persisted execution (`NotFound` / Postgres FK `23503` → `NotFound`); in-memory checks `state`/`workflows` maps like a row existing.
- Node outputs keyed by `(execution_id, node_id, attempt)` — loads return highest attempt per node.
- `list_running` non-terminal statuses: `"created"`, `"running"`, `"paused"`, `"cancelling"`.
- Idempotency: string key dedup, `mark_idempotent` is no-op if key exists.
- Stateful checkpoints keyed by `(execution_id, node_id, attempt)` — see `save/load/delete_stateful_checkpoint`. Default impls on the trait return `ExecutionRepoError::Internal("not implemented")` so backends (Postgres) that don't yet implement them compile; runtime logs WARN and falls back to `init_state` on load failure, never silently swallows (#308).

## Key Decisions
- `PgWorkflowRepo` and `PgExecutionRepo` accept `Pool<Postgres>`, not a connection string — get pool from `PostgresStorage::pool()`.
- `PgWorkflowRepo::list` orders by `created_at, id` for deterministic pagination.
- `PgExecutionRepo` computes lease expiry in SQL via `make_interval(secs => $N)` — avoids chrono dependency.
- lib.rs is in Russian — intentional, do not translate.

## Traps
- Three distinct error types: `StorageError`, `ExecutionRepoError`, `WorkflowRepoError`.
- `PgWorkflowRepo` tests use guard pattern: skip when `DATABASE_URL` absent.
- Lease TTL clamped to 1..86400 seconds — zero or extreme durations are safe.
- Migration 00000000000007 adds nullable `lease_holder`/`lease_expires_at` to `executions`.
- `InMemoryExecutionRepo` lease entries are `(holder, tokio::time::Instant)`; expiration uses
  monotonic tokio time so `start_paused` tests can advance virtual time to assert TTL semantics.
  Postgres backend stays correct via `make_interval` in SQL.
- In-memory and Postgres lease semantics must stay aligned: acquire treats a lease expiring *at*
  current time as still held (`>= now`), and renew only requires holder match (even after expiry).

## Relations
- Depends on nebula-core (IDs). Used by nebula-engine, nebula-api.
