# nebula-execution — Agent orientation
> Agent quick-map for `crates/execution/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Shared execution-time model — the 8-state `ExecutionStatus` machine, `JournalEntry` (WAL), `IdempotencyKey`, and the `ExecutionPlan` derived from the workflow DAG — so engine and storage share one truth, not two.
**Layer:** Core — depends only downward (`nebula-core`, `nebula-error`, `nebula-workflow`); no engine/storage/api/runtime imports.

## Commands
- `cargo check -p nebula-execution`
- `cargo nextest run -p nebula-execution`  ·  doctests: `cargo test -p nebula-execution --doc`
- Snapshot tests use `insta` (state/plan serialization); review with `cargo insta review` after intentional shape changes.

## Key files
- `src/lib.rs` — module roots + public re-exports (the crate's whole surface).
- `src/status.rs` — `ExecutionStatus` 8-state enum (`Pending`…`TimedOut`).
- `src/transition.rs` — validates state-machine transition *legality* only (persistence/CAS is storage's job).
- `src/state.rs` — `ExecutionState` / `NodeExecutionState`, serialized into the `executions` row (largest module).
- `src/journal.rs` — `JournalEntry`, backs the append-only `execution_journal` table.
- `src/idempotency.rs` — `IdempotencyKey` shape `{execution_id}:{node_id}:{attempt}` (format only; dedup lives in storage).
- `src/revision.rs` — experimental workflow-version and worker-flavor revision-pin aggregate,
  available only under `unstable-revisions`; it is not a supported surface before end-to-end
  runtime/storage/admission adoption.
- `src/plan.rs` / `src/replay.rs` — `ExecutionPlan` (parallel schedule) and `ReplayPlan` (checkpoint resume).

## Conventions & never-do
- This crate defines *types + transition legality only*. It must NOT own a repository interface, persist state, or enforce CAS — the spec-16 storage port (`nebula-storage-port::ExecutionStore` + `TransitionBatch`, implemented by `nebula-storage`) is the single source of truth for persisted state (canon §11.1; ADR-0072).
- Do not enable or advertise `unstable-revisions` as a supported contract until durable state,
  admission, and runtime all consume the pins. `WorkflowVersionId` remains the stable workflow
  revision identity.
- This crate defines retry state shapes only: legal `Failed → WaitingRetry → Ready` node transitions, `next_attempt_at`, `total_retries`, `ExecutionBudget.max_total_retries`, `NodeAttempt`, and idempotency-key shape. The engine owns operator-declared node retry (`retry_policy`) and re-dispatch; `nebula-resilience` remains the in-action outbound-call retry surface. Do not add an `ActionResult::Retry` scheduler here.
- `IdempotencyKey` here is just the deterministic key shape (§11.3); check-before-side-effect / mark-after enforcement is in storage. The control-queue / outbox also live in storage, not here.
- 5 `panic!` sites in `transition`/`status` are state-machine invariant guards (flagged debt); do not add new ones — use typed `ExecutionError`.
- Direct downward domain/port dependencies follow the root layer map; durable cross-crate commands/facts use persisted state or explicit outbox/inbox ports; nebula-eventbus carries only lossy observation and wake hints.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design, durability matrix, lease-enforcement notes.
- Canon `docs/PRODUCT_CANON.md` §11.1/§11.2/§11.3/§11.5/§12.2.
