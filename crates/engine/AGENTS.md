# nebula-engine — Agent orientation
> Agent quick-map for `crates/engine/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** Composition root that wires runtime + storage + plugin/credential/resource accessors into one `WorkflowEngine` and drives a workflow DAG from activation to terminal state.
**Layer:** Exec — depends only downward (root AGENTS.md -> Layered Dependency Map). 14 intra-workspace deps are intentional here; justify every new one against the layer rules.

## Common Tasks

| Task | Steps |
|------|-------|
| Add a new engine control command | 1. Add variant to `EngineControlDispatch` in `src/control_dispatch.rs` 2. Handle in `control_consumer.rs` 3. Make dispatch idempotent per `(execution_id, command)` 4. Add test in `tests/` |
| Change execution state transitions | **Only** through `ExecutionStore::commit(TransitionBatch)` — never mutate state directly. See ADR-0072. |
| Add a new action context field | Wire through `src/credential_accessor.rs` or `src/resource_accessor.rs` — these are the cross-layer bridges. |
| Debug retry behavior | Two disjoint surfaces: Layer 1 (`nebula-resilience::retry_with`, opaque to engine) vs Layer 2 (`retry_policy`, engine parks node in `WaitingRetry`). See ADR-0042. |
| Check if engine compiles | `cargo check -p nebula-engine` |
| Run engine tests | `cargo nextest run -p nebula-engine` |

## Commands
- `cargo check -p nebula-engine`
- `cargo nextest run -p nebula-engine`  ·  doctests: none (`[lib] doctest = false`)
- Features: `rotation`, `test-util` (never in prod build — ADR-0023), `chaos-full` (nightly). (Out-of-process plugin execution was retired — ADR-0091; the engine dispatches actions in-process via `InProcessSandbox`.)

## Key files
- `src/lib.rs` — module map + crate-root re-exports (downstream uses `nebula_engine::X`, not deep paths).
- `src/engine.rs` — `WorkflowEngine`: level-by-level DAG execution, bounded concurrency, `run_frontier`, Layer-2 retry heap, cancel registry. (The largest, load-bearing module.)
- `src/control_consumer.rs` / `src/control_dispatch.rs` — durable `execution_control_queue` consumer + `EngineControlDispatch` (Start/Resume/Restart/Cancel/Terminate; canon §12.2, ADR-0008).
- `src/credential_accessor.rs` / `src/resource_accessor.rs` — scoped accessors injected into action contexts (cross-layer bridges).
- `src/scoped_resources.rs` — per-branch resource storage, layered lookup, RAII cleanup (M6.1/M6.2).
- `src/runtime/` — `ActionRuntime` dispatch, sandbox runner, blob/queue plumbing.

## Conventions & never-do
- **All execution-state transitions go through the spec-16 storage port — `ExecutionStore::commit(TransitionBatch)`, CAS on `version`** — `TransitionBatch` is the only way to apply a transition (ADR-0072); no in-engine state mutation or parallel lifecycle (L2-§11.1).
- **Engine owns the control-queue consumer** — a handler that only logs/discards rows violates canon (L2-§12.2). `Cancel` reaches the live loop via `WorkflowEngine::cancel_execution`; dispatch must be idempotent per `(execution_id, command)`.
- **Credential accessor is deny-by-default**: empty allowlist denies all; populate via `with_action_credentials`. No fail-open. (Resources have no allowlist — scoping is the topology layer's job.)
- Not a storage impl or expression evaluator — those are `nebula-storage` / `nebula-expression`. Action dispatch is in-process (`InProcessSandbox`); plugins register in-process through `nebula-plugin` (ADR-0091).
- Two disjoint retry surfaces (ADR-0042): in-action `nebula-resilience::retry_with` (Layer 1, opaque to engine) vs operator-declared `retry_policy` (Layer 2, engine parks node in `WaitingRetry`).
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design, known open debts, architecture notes.
- Canon `docs/PRODUCT_CANON.md` §10/§11.1/§12.2/§13 · `docs/ENGINE_GUARANTEES.md` · ADR-0008/0015/0016/0025/0042/0050.
