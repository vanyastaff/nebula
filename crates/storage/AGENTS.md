# nebula-storage — Agent orientation
> Agent quick-map for `crates/storage/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** The sole adapter implementation (InMemory + SQLite + Postgres) of the spec-16 `nebula-storage-port` contract — execution CAS state, append-only journal, control-queue outbox, idempotency, leases, identity stores, and the durable credential refresh-claim repo.
**Layer:** Exec — depends only downward (root AGENTS.md -> Layered Dependency Map).

## Common Tasks

| Task | Steps |
|------|-------|
| Add a new port method | 1. Define on the trait in `nebula-storage-port` (Core layer) 2. Implement in `src/inmem/`, `src/sqlite/`, `src/postgres/` 3. Add migration if SQL changes needed |
| Add a SQL migration | Create `migrations/{postgres,sqlite}/NNNN_description.sql`. Must stay byte-identical to embedded `src/{postgres,sqlite}/schema.sql`. Run `task db:migrate`. |
| Test Postgres adapter | Needs `DATABASE_URL` env var. Tests are skip-clean without a live DB. |
| Understand CAS transitions | `ExecutionStore::commit` uses CAS on `version` + lease `FencingToken`. If persistence is unavailable it FAILS — never silently mutate in-memory state. |
| Understand outbox atomicity | Control-queue writes share the SAME `TransitionBatch` as state transition (§12.2). Never transition without enqueueing. |
| Check if storage compiles | `cargo check -p nebula-storage --features sqlite,postgres` |

## Commands
- `cargo check -p nebula-storage`  (backends are feature-gated: add `--features sqlite,postgres`)
- `cargo nextest run -p nebula-storage`  ·  doctests: n/a (`doctest = false` in Cargo.toml)
- Postgres runtime tests are `DATABASE_URL`-gated + skip-clean (e.g. `tests/pg_idempotency.rs`); not pg-verified without a live DB.
- Migrations: per-backend trees `migrations/{postgres,sqlite}/`; `0027_port_adapter_schema.sql` must stay byte-identical to embedded `src/{postgres,sqlite}/schema.sql`. `task db:migrate` (Postgres), `task db:reset` (drops data).

## Key files
- `src/lib.rs` — adapter re-exports (`InMemory*`, `StorageError`, `StorageFormat`); module/feature map.
- `src/inmem/` — in-memory port adapters (tests / single-process / loom probe).
- `src/sqlite/` · `src/postgres/` — feature-gated port adapters over the port-scoped schema (Postgres uses real tx + `FOR UPDATE SKIP LOCKED`).
- `src/repos/` — residual non-port traits with live consumers (`ControlQueueRepo`, `IdempotencyStoreRepo`, `WebhookActivationRepo`, identity glue).
- `src/credential/refresh_claim/` — ADR-0041 CAS refresh-claim repo (`try_claim`/`heartbeat`/`release`/`reclaim_stuck`); in_memory + sqlite + postgres.
- `src/credential/layer/` — encryption / audit / cache decorators around credential persistence.

## Conventions & never-do
- `ExecutionStore::commit` is the single source of truth: CAS on `version` + lease `FencingToken` gating; if persistence is unavailable it FAILS — never silently mutate in-memory state.
- Outbox atomicity (§12.2): control-queue writes share the SAME `TransitionBatch` as the state transition. Never transition without enqueueing, or enqueue without transitioning.
- `try_claim` must be atomic under contention (exactly one winner of N replicas); `heartbeat` must validate `ClaimToken.generation` so a stale holder can't extend a reclaimed claim.
- This crate is NOT the state machine (`nebula-execution`), orchestrator (`nebula-engine`), or tenant-scope enforcer (`nebula-tenancy` decorators wrap these adapters). Do NOT re-add the deleted legacy `ExecutionRepo`/`WorkflowRepo` surface (ADR-0072).
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`StorageError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full durability matrix + backend status table.
- `docs/adr/0072-nebula-storage-spec16-port-adapter-tenancy.md` (port/adapter/tenancy); ADR-0041 (refresh claim); `docs/PRODUCT_CANON.md` §11.1/§11.3/§11.5/§12.2/§12.3.
