# nebula-engine — Claude Code orientation
> Agent quick-map for `crates/engine/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** Composition root that wires runtime + storage + plugin/credential/resource accessors into one `WorkflowEngine` and drives a workflow DAG from activation to terminal state.
**Layer:** Exec — depends only downward (root CLAUDE.md -> Layered Dependency Map). 14 intra-workspace deps are intentional here; justify every new one against the layer rules.

## Commands
- `cargo check -p nebula-engine`
- `cargo nextest run -p nebula-engine`  ·  doctests: none (`[lib] doctest = false`)
- Features: `out-of-process-plugins` (OFF by default; second runtime gate `out_of_process_plugin_dirs` also required — ADR-0025), `rotation`, `test-util` (never in prod build — ADR-0023), `chaos-full` (nightly).

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
- Not a storage impl, action dispatcher, plugin loader, or expression evaluator — those are `nebula-storage` / `nebula-runtime` / `nebula-sandbox` / `nebula-expression`.
- Two disjoint retry surfaces (ADR-0042): in-action `nebula-resilience::retry_with` (Layer 1, opaque to engine) vs operator-declared `retry_policy` (Layer 2, engine parks node in `WaitingRetry`).
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design, known open debts, architecture notes.
- Canon `docs/PRODUCT_CANON.md` §10/§11.1/§12.2/§13 · `docs/ENGINE_GUARANTEES.md` · ADR-0008/0015/0016/0025/0042/0050.
