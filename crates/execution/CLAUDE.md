# nebula-execution — Claude Code orientation
> Agent quick-map for `crates/execution/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

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
- `src/plan.rs` / `src/replay.rs` — `ExecutionPlan` (parallel schedule) and `ReplayPlan` (checkpoint resume).

## Conventions & never-do
- This crate defines *types + transition legality only*. It must NOT own a repository interface, persist state, or enforce CAS — `nebula-storage::ExecutionRepo` is the single source of truth for persisted state (canon §11.1).
- Do NOT add engine-level node retry: the engine does not retry nodes (§11.2); `NodeAttempt` only seeds the attempt-keyed idempotency shape (counter stays `1`). Canonical retry is `nebula-resilience` inside an action.
- `IdempotencyKey` here is just the deterministic key shape (§11.3); check-before-side-effect / mark-after enforcement is in storage. The control-queue / outbox also live in storage, not here.
- 5 `panic!` sites in `transition`/`status` are state-machine invariant guards (flagged debt); do not add new ones — use typed `ExecutionError`.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design, durability matrix, lease-enforcement notes.
- Canon `docs/PRODUCT_CANON.md` §11.1/§11.2/§11.3/§11.5/§12.2 · `docs/ENGINE_GUARANTEES.md` · `docs/GLOSSARY.md` §2.
