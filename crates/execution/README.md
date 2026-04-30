---
name: nebula-execution
role: Execution State Machine + Journal + Idempotency Types (WAL, Idempotent Receiver)
status: stable
last-reviewed: 2026-04-17
canon-invariants: [L2-11.1, L2-11.2, L2-11.3, L2-11.5, L2-12.2]
related: [nebula-storage, nebula-engine, nebula-workflow, nebula-resilience, nebula-core, nebula-error]
---

# nebula-execution

## Purpose

A durable workflow engine needs an authoritative model of what a run *is*: its state machine,
its append-only event history, its pre-computed parallel schedule, and the key that makes
individual action invocations idempotent. Without a shared model, the engine orchestrator and
the storage layer each invent their own state representation, producing the "two truths"
anti-pattern canon §14 forbids. `nebula-execution` is that shared model. It defines the
8-state `ExecutionStatus` machine with validated transitions, the `JournalEntry` type that
backs the durable `execution_journal` table, the `IdempotencyKey` shape, and the
`ExecutionPlan` that the engine derives from the workflow DAG. It deliberately does not own a
repository interface — persistence is `nebula-storage::ExecutionRepo`'s job.

## Role

**Execution State Machine + Journal + Idempotency Types.**

Patterns:
- *Write-Ahead Log* (DDIA ch 3, 11) — `JournalEntry` backs the `execution_journal`
  append-only durable timeline.
- *Idempotent Receiver* (EIP) — `IdempotencyKey` shape `{execution_id}:{node_id}:{attempt}`
  is the deterministic per-attempt key checked through `ExecutionRepo` before side effects.
- *Optimistic Concurrency Control* (DDIA ch 7) — `ExecutionStatus` transitions are guarded
  by CAS on `version` in `nebula-storage::ExecutionRepo::transition`.

## Public API

- `ExecutionStatus` — 8-state execution state machine: `Pending`, `Running`, `Paused`,
  `Completed`, `Failed`, `Cancelled`, `Cancelling`, `TimedOut`. Transitions validated by
  the `transition` module.
- `ExecutionState`, `NodeExecutionState` — persistent state tracking per execution and per
  node; serialized into the `executions` table row.
- `ExecutionPlan` — pre-computed parallel execution schedule derived from `DependencyGraph`.
  Feeds the engine scheduler.
- `ReplayPlan` — resume plan for restarting from a checkpoint.
- `ExecutionContext` — lightweight runtime context: `execution_id`, `ExecutionBudget`.
- `ExecutionResult` — post-execution summary: status, timing, node counts, outputs.
- `JournalEntry` — audit log entry type. Each entry is appended to the durable
  `execution_journal` table via `ExecutionRepo::append_journal`.
- `NodeOutput`, `ExecutionOutput` — node output data with metadata.
- `NodeAttempt` — individual attempt tracking (attempt number, started/finished timestamps,
  node status). Used as the shape of attempt-keyed output rows by
  `nebula-storage::ExecutionRepo::save_node_output`.
- `IdempotencyKey` — deterministic key `{execution_id}:{node_id}:{attempt}`. The actual
  dedup enforcement (check-and-mark) lives in `nebula-storage::ExecutionRepo`.
- `ExecutionError` — typed error for state machine violations and execution failures.

## Contract

- **[L2-§11.1]** `nebula-execution` defines the state machine; `ExecutionRepo` in
  `nebula-storage` is the **single source of truth** for persisted execution state.
  Transitions use optimistic CAS on `version`. No handler may mutate execution state
  except through `ExecutionRepo::transition`. Seam: `crates/storage/src/execution_repo.rs`.
  The `transition` module in this crate validates state-machine legality; storage enforces
  persistence and CAS.

- **[L2-§11.2]** Engine-level node re-execution is **out of scope for this crate**. The
  engine does not retry nodes; the canonical retry surface is `nebula-resilience` inside
  an action around outbound calls. `NodeAttempt` exists to seed the idempotency-key shape
  `{execution_id}:{node_id}:{attempt}` so storage rows stay attempt-keyed even though the
  attempt counter never advances past `1` from engine-driven flow.

- **[L2-§11.3]** `IdempotencyKey` shape is `{execution_id}:{node_id}:{attempt}`. Seam:
  `crates/execution/src/idempotency.rs`. Enforcement (check before side effect, mark after)
  lives in `nebula-storage::ExecutionRepo`. For non-idempotent actions (payments, writes
  without upsert) the idempotency guard must be applied before calling the remote system.

- **[L2-§11.5]** `JournalEntry` type backs the durable `execution_journal` (append-only,
  replayable). Seam: `crates/storage/src/execution_repo.rs` — `ExecutionRepo::append_journal`.
  Checkpoint state is best-effort: a checkpoint write failure logs and does not abort; work
  since the last successful checkpoint may be replayed or lost.

- **[L2-§12.2]** `ExecutionStatus` machine defines what states exist and what transitions
  are legal. The `transition` module enforces legality. Persistence and CAS are in
  `nebula-storage`. No handler invents a parallel lifecycle.

## Non-goals

- Not the engine orchestrator — see `nebula-engine` (drives these types).
- Not the storage implementation — see `nebula-storage` (`ExecutionRepo`, `executions`
  table, `execution_journal`, `execution_control_queue`). The `ExecutionControlQueue`
  (durable outbox for cancel/dispatch signals) and the `Transactional Outbox` pattern live
  in `nebula-storage`, not here.
- Not a retry scheduler — engine-level node retry is not part of the engine contract
  (§11.2); the canonical retry surface is `nebula-resilience` inside an action.
- Not a resource lifecycle manager — see `nebula-resource` for `ReleaseQueue` / `Bulkhead`.

## Maturity

See `docs/MATURITY.md` row for `nebula-execution`.

- API stability: `stable` — state machine, journal, idempotency key, and plan types are
  in active use by `nebula-engine` and `nebula-storage`; no known planned breaking changes.
- Layer 1 lease enforcement (`lease_holder`/`lease_expires_at`) shipped via M2.2 — heartbeat-driven via `acquire_and_heartbeat_lease` (see `DEFAULT_EXECUTION_LEASE_TTL` / `DEFAULT_EXECUTION_LEASE_HEARTBEAT_INTERVAL`), verified by `crates/engine/tests/lease_takeover.rs`, `crates/storage/tests/execution_lease_pg_integration.rs`, and the loom probe at `crates/storage-loom-probe/src/lease_handoff.rs`. Layer 2 (`claimed_by`/`claimed_until` from `migrations/postgres/0011_executions.sql`) remains Sprint E (1.1) scaffolding — see the durability matrix below.
- Integration tests: 0 in `tests/`; state machine and plan coverage via unit tests +
  engine-level integration tests.
- 5 `panic!` sites in `transition` and `status` modules serve as state-machine invariant
  guards; these are technical debt (candidates for `#[must_use]` or typed errors).

## Related

- Canon: `docs/PRODUCT_CANON.md` §11.1, §11.2, §11.3, §11.5, §12.2.
- Glossary: `docs/GLOSSARY.md` §2 (execution authority: `ExecutionRepo`, `executions`,
  `execution_journal`, `execution_control_queue`, `IdempotencyKey`, `Cancel`, `Cancelled`).
- Engine guarantees: `docs/ENGINE_GUARANTEES.md`.
- Siblings: `nebula-storage` (persists via `ExecutionRepo`), `nebula-engine` (drives),
  `nebula-workflow` (DAG → `ExecutionPlan`), `nebula-resilience` (in-action retry).

## Appendix

### Idempotency key format (L4 detail, evicted from PRODUCT_CANON.md §11.3)

The deterministic key shape is `{execution_id}:{node_id}:{attempt}`, persisted in
`idempotency_keys`. The format string is an implementation detail (L4) — changing it
requires updating this README and the corresponding code; no canon revision. The invariant
("deterministic per-attempt, checked before side-effect") is canonical (L2-§11.3).

### Persistence durability matrix (reference from §11.5)

| Artifact | Status | Notes |
|---|---|---|
| `executions` row + state JSON | **Durable** (CAS via `ExecutionRepo`) | Source of truth |
| `execution_journal` | **Durable** (append-only) | Replayable history |
| `execution_control_queue` | **Durable** (outbox) | At-least-once cancel/dispatch |
| `stateful_checkpoints` | **Best-effort** | Failure logs, does not abort; may replay |
| `executions.lease_holder` / `lease_expires_at` (Layer 1) | **Durable + enforced** (M2.2, ADR-0008/0015) | Heartbeat-driven; multi-runner takeover via TTL expiry |
| `executions.claimed_by` / `claimed_until` (Layer 2, Sprint E) | **Schema may precede enforcement** | Spec-16 scaffolding, deferred to 1.1 — no engine consumers today |

**Lease enforcement (Layer 1, shipped via M2.2):** the engine's
`acquire_and_heartbeat_lease` (`crates/engine/src/engine.rs:815-859`)
acquires a lease on `execute_workflow` / `resume_execution`, spawns a
heartbeat task that renews every `DEFAULT_EXECUTION_LEASE_HEARTBEAT_INTERVAL`
(10s) within `DEFAULT_EXECUTION_LEASE_TTL` (30s), and tears the runner
down on `Ok(false)` (lease stolen) or `Err(_)` (storage failure). The
durable fence is `lease_holder` string match — a stale runner whose
holder no longer matches gets rejected on `renew_lease`. Multi-runner
takeover is verified by `crates/engine/tests/lease_takeover.rs`,
`crates/storage/tests/execution_lease_pg_integration.rs`, and the
loom probe in `crates/storage-loom-probe/src/lease_handoff.rs`.

**Layer 2 status:** the `claimed_by` / `claimed_until` columns + the
two indexes (`idx_executions_pending_claim`, `idx_executions_stale_lease`)
defined by `migrations/postgres/0011_executions.sql` are **Sprint E
(1.1) scaffolding**, intentionally inert until the spec-16 row-model
engine refactor lands. See ROADMAP "Out of scope for 1.0" → "Storage
Layer 2 / spec-16 multi-tenant row model (Sprint E)". The
`Schema may precede enforcement` warning therefore applies to Layer 2
only — Layer 1 is enforced today.

### Architecture notes

- Clean separation of types vs persistence: this crate defines the state machine and types;
  `nebula-storage::ExecutionRepo` persists them. Canon §11.1 makes the persistence layer
  authoritative — this crate deliberately does not own a repository interface.
- No cross-layer dependencies: only `nebula-core`, `nebula-error`, `nebula-workflow`.
  No imports from engine, runtime, storage, or API.
