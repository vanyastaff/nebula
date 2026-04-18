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

Layer 1 — production interfaces (use these today):

- `ExecutionRepo` — repository trait; seam for §11.1 CAS transitions, §11.3 idempotency
  check-and-mark, §11.5 journal + checkpoint writes, §12.2 outbox atomicity. Also carries the
  ADR-0009 resume-persistence seams (`set_workflow_input` / `get_workflow_input` and
  `save_node_result` / `load_node_result` / `load_all_results`).
- `ExecutionRepoError` — typed error for CAS conflicts, not-found, timeout, lease unavailable,
  and `UnknownSchemaVersion` (surfaced when a persisted node-result record carries a schema
  version the binary cannot decode; ADR-0009 §2).
- `InMemoryExecutionRepo` — in-memory implementation for tests (via `test_support`).
- `NodeResultRecord` — persisted `ActionResult<Value>` variant (kind tag + JSON + schema
  version); written by `save_node_result`, read by `load_node_result` / `load_all_results`
  (ADR-0009 §1).
- `MAX_SUPPORTED_RESULT_SCHEMA_VERSION` — highest `NodeResultRecord.schema_version` the current
  binary can decode; callers compare against this on mixed-binary deploys.
- `StatefulCheckpointRecord` — checkpoint record persisted by `ExecutionRepo::save_stateful_checkpoint`.
- `WorkflowRepo` — repository trait for workflow definition persistence.
- `WorkflowRepoError` — typed error for workflow repo operations.
- `InMemoryWorkflowRepo` — in-memory implementation for tests.
- `StorageError` — top-level storage error type.
- `StorageFormat` — serialization format abstraction (JSON / MessagePack).
- `Storage` — generic key-value trait (binary or typed values).
- `MemoryStorage`, `MemoryStorageTyped` — in-memory KV implementations.

Feature `postgres` adds: `PgExecutionRepo`, `PgWorkflowRepo`, `PostgresStorage`,
`PostgresStorageConfig`.

Layer 2 — planned / experimental (`repos` module):

- `repos::ControlQueueRepo` + `repos::InMemoryControlQueueRepo` — **implemented**; produced by
  the API cancel path and consumed by `nebula_engine::ControlConsumer` (skeleton — dispatch
  lands with ADR-0008 follow-ups A2 / A3). All other `repos::*` traits are spec-16 design
  placeholders with no implementations — see Appendix.

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

## Non-goals

- Not the execution state machine — see `nebula-execution` (state types, transition legality).
- Not the engine orchestrator — see `nebula-engine` (drives `ExecutionRepo`).
- Not an action dispatcher — see `nebula-runtime`.
- Not a KV cache (Redis) as a production execution backend — Redis feature is KV only, not
  execution state.

## Maturity

See `docs/MATURITY.md` row for `nebula-storage`.

- API stability: `partial` — Layer 1 (`ExecutionRepo`, `WorkflowRepo`) is `stable` and the
  production contract; Layer 2 (`repos::*` except `ControlQueueRepo`) is `planned` and not
  yet implementable without a broader engine + API refactor.
- `execution_leases` schema may exist before full engine enforcement (§11.5 debt) — do not
  imply lease safety until enforcement is wired.
- S3 and Redis features are optional and experimental; local filesystem backend is `planned`.
- `repos::InMemoryControlQueueRepo` is the only implemented Layer-2 type that should be
  depended on today.

## Related

- Canon: `docs/PRODUCT_CANON.md` §11.1, §11.3, §11.5, §12.2, §12.3.
- Engine guarantees: `docs/ENGINE_GUARANTEES.md`.
- Glossary: `docs/GLOSSARY.md` §2 (`ExecutionRepo`, `execution_journal`,
  `execution_control_queue`, `IdempotencyKey`, Transactional Outbox, OCC).
- Siblings: `nebula-execution` (state types), `nebula-engine` (transitions via `ExecutionRepo`),
  `nebula-core` (ID types).

## Appendix

### Two coexisting trait layers — status per canon §11.6

**Layer 1 — `ExecutionRepo` / `WorkflowRepo` (top-level re-exports) — `implemented`**

The production path. State is stored as opaque `serde_json::Value` blobs with typed ID keys
and `u64` optimistic-CAS versions. This is the layer the knife scenario (§13) exercises
end-to-end. Use Layer 1 for all engine, handler, and test code today.

**Layer 2 — `repos` module (spec-16 row model) — `planned / experimental`**

Structured rows, mandatory multi-tenancy (`workspace_id` / `org_id`), split
`WorkflowRow` + `WorkflowVersionRow`, idempotency as a column on `ExecutionNodeRow`. Trait
definitions only — no in-memory or Postgres implementations exist yet; the engine / API cannot
compile against these signatures without a broader refactor ("Sprint E — adopt spec-16 row
model" in `docs/superpowers/specs/2026-04-16-workspace-health-audit.md`).

**Exception:** `repos::ControlQueueRepo` + `repos::InMemoryControlQueueRepo` are implemented;
the API cancel path produces into them and `nebula_engine::ControlConsumer` (ADR-0008) is the
engine-side consumer (skeleton today; dispatch lands with A2 / A3). They are the only Layer-2
contract consumers should depend on today.

### Persistence durability matrix (reference from §11.5)

| Artifact | Status | Notes |
|---|---|---|
| `executions` row + state JSON | **Durable** (CAS via `ExecutionRepo`) | Source of truth |
| `execution_journal` (append-only) | **Durable** | Replayable history |
| `execution_control_queue` (outbox) | **Durable** | At-least-once cancel/dispatch (§12.2) |
| `stateful_checkpoints` | **Best-effort** | Write failure logs, does not abort; may replay |
| `execution_leases` | **Schema may precede enforcement** | Do not imply lease safety |
| In-process `mpsc` / channels | **Ephemeral** | Never authoritative |

### Supported backends

| Backend | Feature flag | Status |
|---|---|---|
| SQLite (file or `sqlite::memory:`) | built-in | `implemented` — local + test default |
| PostgreSQL | `postgres` | `implemented` — production path |
| Redis | `redis` | `experimental` — KV only, not execution state |
| S3 / MinIO | `s3` | `experimental` — blob storage |
| Local filesystem | — | `planned` |
