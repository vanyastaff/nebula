# nebula-execution

Runtime execution state, journals, idempotency, and planning types for the Nebula workflow engine.

**Layer:** Core
**Canon:** §10 (golden path), §11.1 (execution authority), §11.3 (idempotency), §11.5 (durability matrix)

## Status

**Overall:** `implemented` — the state machine, journal entry types, and idempotency key are the authoritative runtime model used by `nebula-engine` and persisted by `nebula-storage::ExecutionRepo`.

**Works today:**

- 8-state execution state machine with validated transitions (`transition` module)
- `ExecutionState` / `NodeExecutionState` durable state tracking
- `ExecutionPlan` — pre-computed parallel schedule derived from a DAG
- `JournalEntry` — append-only audit log feeding the durable `execution_journal` table
- `IdempotencyKey` deterministic shape `{execution_id}:{node_id}:{attempt}`
- 12 unit tests covering state transitions, plan computation, idempotency, and journal

**Known gaps / deferred to other crates:**

- **Idempotency enforcement** — this crate only defines the `IdempotencyKey` shape. Actual dedup (checked-and-marked-through-ExecutionRepo) lives in `nebula-storage`. Canon §11.3.
- **Retry accounting** — `NodeAttempt` tracks attempts, but **engine-level** re-execution from `ActionResult::Retry` is `planned` per canon §11.2 (no persisted attempt bump, no CAS-protected consumer). Until that lands, the canonical retry surface is `nebula-resilience` inside an action.
- **Integration tests** — 0 end-to-end tests in `tests/`; coverage relies on unit tests + engine-level integ tests.

## Architecture notes

- **Clean separation of types vs persistence.** This crate defines the state machine and types; `nebula-storage::ExecutionRepo` persists them. Canon §11.1 makes the persistence layer the single authority — this crate deliberately does not own a repository interface.
- **No cross-layer dependencies.** Only `nebula-core`, `nebula-error`, `nebula-workflow`. No imports from engine, runtime, storage, or API — the layer direction in `CLAUDE.md` is respected.
- **Panics in state code** (5 `panic!` sites) — used as state-machine invariant guards in `transition` and `status` modules. Review periodically: every panic should be an invariant the type system or `#[must_use]` could carry instead.
- **No obvious SRP/DRY violations.** Thirteen modules each own a single concept; no dead compat shims.

## Scope

This crate models **execution-time concepts**. It does not contain the orchestrator (that is `nebula-engine`) and it does not contain the storage implementation (that is `nebula-storage`). It defines the types that the engine drives and that `ExecutionRepo` persists.

## What this crate provides

| Type / module | Role |
| --- | --- |
| `ExecutionStatus` | Execution-level state machine (8 states). Transitions validated by `transition` module. |
| `ExecutionState`, `NodeExecutionState` | Persistent state tracking per execution and per node. |
| `ExecutionPlan` | Pre-computed parallel execution schedule derived from a workflow DAG. |
| `ExecutionContext` | Lightweight runtime context (`execution_id`, budget). |
| `ExecutionResult` | Post-execution summary — status, timing, node counts, outputs. |
| `JournalEntry` | Audit log entry; backs the `execution_journal` append-only durable timeline. |
| `NodeOutput` | Node output data with metadata. |
| `NodeAttempt` | Individual execution attempt tracking. |
| `IdempotencyKey` | Deterministic key `{execution_id}:{node_id}:{attempt}` for deduplication. The actual dedup enforcement lives in `nebula_storage::ExecutionRepo`. |
| `transition` module | Validates `ExecutionStatus` state transitions. |

## Non-goals

- **Not** the engine orchestrator — see `nebula-engine`.
- **Not** the storage implementation — see `nebula-storage` (`ExecutionRepo`, `executions` row, `execution_journal`, `execution_control_queue`).
- **Not** a retry scheduler — canon §11.2 defines engine-level retry as `planned`; the canonical retry surface today is `nebula-resilience` inside an action.

## Where the contract lives

- Source: `src/lib.rs`, `src/status.rs`, `src/state.rs`, `src/plan.rs`, `src/idempotency.rs`, `src/journal.rs`
- Canon: `docs/PRODUCT_CANON.md` §10, §11.1, §11.3, §11.5
- Glossary: `docs/GLOSSARY.md` §2 (execution authority)

## See also

- `nebula-engine` — drives these types
- `nebula-storage` — persists them via `ExecutionRepo`
- `nebula-workflow` — defines the DAG that `ExecutionPlan` derives from

### Idempotency key format (evicted from PRODUCT_CANON.md §11.3)

The deterministic key shape is `{execution_id}:{node_id}:{attempt}`, persisted in `idempotency_keys`. The format string itself is an implementation detail (L4) — changing it requires only this README and the corresponding code; no canon revision. The invariant ("deterministic per-attempt, checked before side-effect") is canonical (L2).
