---
name: nebula-storage
role: Storage Port (Repository Implementations + CAS + Outbox)
status: partial
last-reviewed: 2026-04-17
canon-invariants: [L2-11.1, L2-11.3, L2-11.5, L2-12.2, L2-12.3]
related: [nebula-execution, nebula-engine, nebula-core, nebula-error]
---

# nebula-storage

## Purpose

A durable workflow engine needs a persistence seam that the engine and API can drive without
coupling to a specific database. More critically, it needs a place where optimistic CAS state
transitions, journal appends, and outbox writes can share the same logical operation — the
"two truths" anti-pattern (canon §14) forbids splitting those writes across separate
transactions. `nebula-storage` is that seam: it exposes `ExecutionRepo` and `WorkflowRepo` as
the production persistence interfaces for execution state, the append-only journal, idempotency
keys, checkpoints, leases, and the durable `execution_control_queue` outbox — all backed by
SQLite (dev / test) or PostgreSQL (production).

## Role

*Storage Port.* Implements the Repository pattern (DDD) with Optimistic Concurrency Control
(DDIA ch 7) on `ExecutionRepo::transition` and a Transactional Outbox (EIP "Guaranteed
Delivery", DDIA ch 11) via `execution_control_queue`. Provides the single persistence layer
the knife scenario (canon §13) exercises end-to-end.

## Public API

The contract is the spec-16 storage **port** in `nebula-storage-port`
(`ExecutionStore` + the atomic `TransitionBatch`, `ExecutionJournalReader`,
`NodeResultStore`, `CheckpointStore`, `IdempotencyGuard` /
`IdempotencyStore`, `WorkflowStore` / `WorkflowVersionStore`,
`ControlQueue`, `WebhookActivationStore`, `RefreshClaimStore`, and the
identity-zoo stores; `StorageError`; the plain-data `Scope`). This crate
provides the adapters:

- `inmem::*` — in-memory adapters (tests, local single-process, the loom
  probe).
- `sqlite::*` (feature `sqlite`) — single-writer-correct adapters over a
  port-scoped schema; `init_schema` installs it for `:memory:` / test
  pools.
- `postgres::*` (feature `postgres`) — production multi-process adapters
  (real tx + `FOR UPDATE SKIP LOCKED`) over the same port-scoped schema.
- `repos::*` — the non-port backend traits that still have live
  consumers: `ControlQueueRepo` (+ `InMemoryControlQueueRepo`,
  `pg::PgControlQueueRepo`), `IdempotencyStoreRepo`,
  `WebhookActivationRepo`, and the identity-row glue the Postgres
  backend implements.
- `StorageError` (re-exported from the port), `StorageFormat`
  (serialization format abstraction).

Execution / workflow persistence goes through the port adapters; the
legacy `ExecutionRepo` / `WorkflowRepo` / `Pg*Repo` surface and the
never-implemented spec-16 trait placeholders were deleted (ADR-0068).

Layer 2 — planned / experimental (`repos` module):

- `repos::ControlQueueRepo` + `repos::InMemoryControlQueueRepo` — **implemented**; produced by
  the API cancel path and consumed by `nebula_engine::ControlConsumer` (skeleton — dispatch
  lands with ADR-0008 follow-ups A2 / A3). All other `repos::*` traits are spec-16 design
  placeholders with no implementations — see Appendix.

Credential coordination — durable refresh claim (П2 / ADR-0041):

- `credential::refresh_claim::RefreshClaimRepo` — cross-replica claim seam for the engine's
  two-tier `RefreshCoordinator` (L1 in-process coalescer + L2 durable claim). Provides
  CAS-based `try_claim` (one acquirer wins under contention), `heartbeat` (TTL extension
  validated against `ClaimToken` generation), idempotent `release`, and `reclaim_stuck`
  (sweeps expired claims past TTL). `mark_sentinel` flags an in-flight IdP POST so a
  reclaim sweep can detect mid-refresh crashes; `record_sentinel_event` +
  `count_sentinel_events_in_window` back the engine's N=3-in-1h `ReauthRequired`
  escalation per sub-spec
  [`2026-04-24-credential-refresh-coordination.md`](../../docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md)
  §3.4-§3.6.
- `RefreshClaim`, `ClaimAttempt`, `ClaimToken`, `RepoError`, `HeartbeatError`,
  `ReclaimedClaim`, `SentinelState`, `ReplicaId` — DTO surface re-exported at
  `nebula_storage::{RefreshClaim, ClaimAttempt, ClaimToken, …}`.
- `InMemoryRefreshClaimRepo` — production-shaped reference impl for tests + single-replica
  deploys.
- Feature `sqlite` adds `SqliteRefreshClaimRepo` (default local backend; `SQLITE` migrations
  `0022_credential_refresh_claims` + `0023_credential_sentinel_events`).
- Feature `postgres` adds `PgRefreshClaimRepo` (production multi-replica backend; `POSTGRES`
  migrations `0022_credential_refresh_claims` + `0023_credential_sentinel_events`).

## Contract

- **[L2-§11.1]** `ExecutionRepo::transition` is the **single source of truth** for execution
  state. Applies CAS on `version`. If persistence is unavailable, the operation fails — it does
  not silently mutate in-memory state. Seam: `crates/storage/src/execution_repo.rs`.

- **[L2-§11.3]** Idempotency enforcement (check-and-mark) lives here via
  `ExecutionRepo`. Key shape `{execution_id}:{node_id}:{attempt}` is defined in
  `nebula-execution`. Seam: `crates/storage/src/execution_repo.rs`.

- **[L2-§11.5]** `ExecutionRepo::append_journal` backs the durable `execution_journal`
  (append-only, replayable). `ExecutionRepo::save_stateful_checkpoint` is **best-effort**: a
  checkpoint write failure may log and not abort execution; work since the last checkpoint may
  be replayed or lost. Seam: `crates/storage/src/execution_repo.rs`.

- **[ADR-0009]** Resume-persistence schema foundation. `ExecutionRepo::set_workflow_input` /
  `get_workflow_input` persist the workflow trigger payload alongside the execution row
  (issue #311). `save_node_result` / `load_node_result` / `load_all_results` persist the full
  `ActionResult<Value>` variant per node attempt (issue #299) so resume can replay edge
  decisions through `evaluate_edge` (foundation for #324, #336). `NodeResultRecord` carries a
  `schema_version`; an unknown version surfaces as `ExecutionRepoError::UnknownSchemaVersion`
  rather than a silent fall-back. Engine consumers land in downstream chips B2 / B3 / B4.

- **[L2-§12.2]** The `execution_control_queue` outbox is written in the **same logical
  operation** as the state transition it accompanies. Cancel signals must be enqueued atomically
  with the `cancelling` transition. A handler that transitions state without enqueueing, or
  enqueues without transitioning, violates this invariant.

- **[L2-§12.3]** The default local storage path is **SQLite** (file or `sqlite::memory:`).
  In-process tests use `nebula_storage::test_support` (`sqlite_memory_*` helpers), not a
  separate HashMap "memory backend." There is **one** local storage path.

- **[ADR-0041 / sub-spec §3]** `RefreshClaimRepo::try_claim` MUST be atomic under
  contention — exactly one of N concurrent acquirers across N replicas wins. Implementations
  achieve this via a CAS-shaped `INSERT … ON CONFLICT DO UPDATE WHERE expires_at < now()`
  predicate (Postgres + SQLite) or a per-key `parking_lot::Mutex` guarded `HashMap` swap
  (in-memory). `heartbeat` MUST validate the `ClaimToken.generation` so a stale holder
  cannot extend a reclaimed claim (reclaim sweep bumps generation). `reclaim_stuck` MUST
  return reclaimed credentials atomically — a partial reclaim that releases the row but
  fails to surface it to the caller would leave the sentinel state un-observed and the
  N=3-in-1h escalation count short. Seam: `crates/storage/src/credential/refresh_claim/`.

## Non-goals

- Not the execution state machine — see `nebula-execution` (state types, transition legality).
- Not the engine orchestrator — see `nebula-engine` (drives `ExecutionRepo`).
- Not an action dispatcher — see `nebula-runtime`.
- Not a KV cache (Redis) as a production execution backend — Redis feature is KV only, not
  execution state.

## Maturity

See `docs/MATURITY.md` row for `nebula-storage`.

- API stability: `stable` — the single architecture is the spec-16
  storage **port** (`nebula-storage-port`), implemented here for
  InMemory + SQLite + Postgres and rewired through `engine` / `api`
  (ADR-0068). The legacy `ExecutionRepo` / `WorkflowRepo` dual layer was
  deleted.
- Lease fencing is **enforced**: `acquire_lease` returns a monotone
  `FencingToken` that gates every committed `TransitionBatch`, so a
  superseded holder is rejected even on a matching CAS version (the
  zombie-runner hole; ADR-0068). Verified by
  `crates/engine/tests/lease_takeover.rs`, the lease-handoff loom probe
  at `crates/storage-loom-probe/src/lease_handoff.rs`, and the
  conformance matrix's lease cases.
- The retained `repos::*` surface (`ControlQueueRepo`,
  `IdempotencyStoreRepo`, `WebhookActivationRepo`, identity-row glue)
  keeps live consumers (the API idempotency middleware and the Postgres
  glue) and is no longer "planned spec-16".
- S3 and Redis features are optional and experimental; local filesystem
  backend is `planned`.
- Postgres adapter + identity stores are compile-verified and structurally
  identical to the runtime-verified SQLite tree, but Postgres runtime
  coverage is `DATABASE_URL`-gated and skip-clean — not claimed as
  pg-verified (ADR-0068 "Verification status").

## Database migrations

Migrations live in two per-backend trees: `migrations/postgres/` and
`migrations/sqlite/` (logically identical tables; dialect types differ).
There is no flat top-level migration tree.

The spec-16 storage-port adapters persist through the `port_*` tables in
`0027_port_adapter_schema.sql`, which is byte-identical to the embedded
`src/{postgres,sqlite}/schema.sql` that `init_schema` applies for
in-memory / test pools. The migration is the canonical source for a real
database rebuild; the embedded schema is the test/`:memory:` path. Keep
the pair in lockstep (regenerate the migration with `cp` from the
embedded schema — see the per-tree README).

`task db:migrate` applies pending Postgres migrations
(`--source crates/storage/migrations/postgres`, `DATABASE_URL`-gated).
`task db:reset` **drops and recreates the database** then re-runs every
migration — it destroys all local dev data.

## Related

- Canon: `docs/PRODUCT_CANON.md` §11.1, §11.3, §11.5, §12.2, §12.3.
- Engine guarantees: `docs/ENGINE_GUARANTEES.md`.
- ADR: `docs/adr/0068-nebula-storage-spec16-port-adapter-tenancy.md`
  (port / adapter / tenancy decision, supersession, the three
  correctness bugs, the migration-gap history).
- Glossary: `docs/GLOSSARY.md` §2 (`execution_journal`,
  `execution_control_queue`, `IdempotencyKey`, Transactional Outbox, OCC).
- Siblings: `nebula-storage-port` (the port contract), `nebula-tenancy`
  (scope-enforcing decorators), `nebula-execution` (state types),
  `nebula-engine` (transitions via the port `ExecutionStore` +
  `TransitionBatch`), `nebula-core` (ID types).

## Appendix

### Single storage architecture — the spec-16 port (ADR-0068)

There is one architecture: the spec-16 storage **port**
(`nebula-storage-port`, Core tier — ISP-segregated object-safe traits,
port-local DTO rows, `StorageError`, the atomic `TransitionBatch`, the
plain-data `Scope`). This crate implements it for **InMemory + SQLite +
Postgres**; `nebula-tenancy` wraps it with scope-enforcing decorators;
`engine` / `api` consume only the port. The legacy
`ExecutionRepo` / `WorkflowRepo` dual layer and the never-implemented
`repos::{execution,workflow,execution_node,journal}` placeholders were
deleted.

The retained `repos::*` traits (`ControlQueueRepo`,
`IdempotencyStoreRepo`, `WebhookActivationRepo`, and the identity-row
glue the Postgres backend implements) are not part of the deleted dual
model — they keep live consumers (the API idempotency middleware, the
`pg::*` glue) and persist through the same per-backend schema.

### Persistence durability matrix (reference from §11.5)

| Artifact | Status | Notes |
|---|---|---|
| `port_executions` row + state JSON | **Durable** (CAS via `ExecutionStore` + `TransitionBatch`) | Source of truth |
| `port_execution_journal` (append-only) | **Durable** | Replayable history; appended in the same commit as state |
| `port_control_queue` (outbox) | **Durable** | At-least-once cancel/dispatch; written in the same `TransitionBatch` (§12.2) |
| stateful checkpoints | **Best-effort** | Write failure logs, does not abort; may replay |
| lease holder / expiry + `fencing_generation` | **Durable + enforced** (ADR-0068) | `acquire_lease` → `FencingToken`; a superseded holder is rejected even on a matching CAS version. Verified by `crates/engine/tests/lease_takeover.rs`, the loom probe at `crates/storage-loom-probe/src/lease_handoff.rs`, and the conformance lease cases |
| idempotency dedup | **Durable** | First-writer-wins via the port `IdempotencyGuard` / `IdempotencyStore`; sweep drives `evict_expired`. Verified by the conformance matrix + `crates/storage/tests/pg_idempotency.rs` (`DATABASE_URL`-gated) |
| In-process `mpsc` / channels | **Ephemeral** | Never authoritative |

### Supported backends

| Backend | Feature flag | Status |
|---|---|---|
| SQLite (file or `sqlite::memory:`) | `sqlite` | `implemented` — local + test default; feature-gated since the wave-2 review (driver footprint not unconditional) |
| PostgreSQL | `postgres` | `implemented` — production path |
| Redis | `redis` | `experimental` — KV only, not execution state |
| S3 / MinIO | `s3` | `experimental` — blob storage |
| Local filesystem | — | `planned` |
