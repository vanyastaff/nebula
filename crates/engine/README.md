---
name: nebula-engine
role: Composition Root (Orchestrator)
status: partial
last-reviewed: 2026-04-17
canon-invariants: [L2-10, L2-11.1, L2-12.2]
related: [nebula-execution, nebula-storage, nebula-runtime, nebula-workflow, nebula-resilience, nebula-plugin, nebula-credential, nebula-resource]
---

# nebula-engine

## Purpose

A workflow engine needs one component that assembles all the other crates and drives an execution
from "activated workflow" to "terminal state." Without a composition root, callers must wire
`nebula-runtime`, `nebula-storage`, `nebula-execution`, and the plugin registry themselves and
risk diverging from the canon §12.2 control-plane contract. `nebula-engine` is that root: it
builds an `ExecutionPlan` from the workflow DAG, resolves node inputs from predecessor outputs,
transitions execution state through `ExecutionRepo` (CAS on `version`), and delegates action
dispatch to `nebula-runtime`. Canon §12.2 names this crate as the location of the
`execution_control_queue` consumer (`ControlConsumer`). The consumer skeleton — polling,
claim/ack, graceful shutdown — ships today; the engine-side `Start` / `Resume` / `Restart`
dispatch lives in `EngineControlDispatch` (A2, closes #332 / #327); the `Cancel` / `Terminate`
dispatch (A3, closes #330) signals the live frontier loop via `WorkflowEngine::cancel_execution`
and `Terminate` shares the cooperative-cancel body until a distinct forced-shutdown path is
wired (ADR-0016). A demo handler that logs and discards commands does not satisfy the canon.

## Role

*Composition Root.* Wires the exec layer (`nebula-runtime`, `nebula-storage`, plugin registry,
credential/resource accessors) into a single `WorkflowEngine` entry point. Drives the §10
golden path — activate → schedule → transition → observe — using DAG-level parallelism and
bounded concurrency.

## Public API

- `WorkflowEngine` — entry point: executes workflows level-by-level with bounded concurrency.
  Exposes `cancel_execution(id) -> bool` so control-queue `Cancel` signals reach the live
  frontier loop (ADR-0008 A3; ADR-0016).
- `ControlConsumer` — durable control-queue consumer drained via `ControlQueueRepo`
  (canon §12.2, ADR-0008). All five commands — `Start` / `Resume` / `Restart` / `Cancel` /
  `Terminate` — are wired via `EngineControlDispatch` (A2 + A3).
- `ControlDispatch` — engine-owned trait implementors provide to deliver typed commands
  (`ExecutionId` + command kind) to the engine's start / cancel paths. Must be idempotent
  per `(execution_id, command)` pair (ADR-0008 §5).
- `EngineControlDispatch` — the canonical engine-owned `ControlDispatch` impl. For
  `Start` / `Resume` / `Restart`: reads the current `ExecutionStatus` for the ADR-0008 §5
  idempotency guard, then delegates to `WorkflowEngine::resume_execution` under the
  ADR-0015 lease scope. For `Cancel` / `Terminate`: signals
  `WorkflowEngine::cancel_execution` on every non-orphan delivery (idempotent via the
  underlying `CancellationToken`; see ADR-0016 for the cooperative-cancel contract).
- `ControlDispatchError` — typed error returned from `ControlDispatch` methods; recorded on
  the control-queue row via `mark_failed` (no auto-retry — ADR-0008 §5).
- `ExecutionResult` — post-run summary returned to the API layer.
- `EngineError` — typed engine-layer error.
- `ExecutionEvent` — broadcast event type emitted via `nebula-eventbus`.
- `EngineCredentialAccessor` — scoped credential accessor injected into action contexts.
- `EngineResourceAccessor` — scoped resource accessor injected into action contexts.
- `NodeOutput` — per-node output threaded between execution levels.
- `DEFAULT_EVENT_CHANNEL_CAPACITY` — default backpressure bound for the event channel.
- `DEFAULT_BATCH_SIZE` / `DEFAULT_POLL_INTERVAL` — tunables for `ControlConsumer`.

Re-exports from `nebula-plugin`: `Plugin`, `PluginKey`, `PluginManifest`, `PluginRegistry`,
`PluginType`.

## Contract

- **[L2-§11.1]** Execution state transitions go through `ExecutionRepo::transition` (CAS on
  `version`). No handler inside the engine mutates execution state in-memory or invents a
  parallel lifecycle. Seam: `crates/storage/src/execution_repo.rs — ExecutionRepo::transition`.

- **[L2-§12.2]** The engine owns the `execution_control_queue` consumer
  (`ControlConsumer`; wiring decisions in ADR-0008). Cancel signals are written to the outbox in
  the same logical operation as the state transition and the engine's `ControlConsumer` drains
  the queue. All five commands — `Start` / `Resume` / `Restart` / `Cancel` / `Terminate` — are
  wired end-to-end via `EngineControlDispatch` (A2 + A3). `Cancel` reaches the live frontier
  loop through the per-instance cancel registry (`WorkflowEngine::cancel_execution`; ADR-0016);
  `Terminate` shares the cooperative-cancel body until a distinct forced-shutdown path is
  wired. A handler that only logs and discards control-queue rows violates this invariant.

- **[L2-§10]** The golden-path knife scenario (canon §13) — define, activate, start, observe,
  cancel — exercises this crate's integration with `ExecutionRepo` end-to-end. Integration
  tests in `tests/` cover the control-queue cancel path.

## Non-goals

- Not a storage implementation — see `nebula-storage` (`ExecutionRepo`, storage backends).
- Not an action dispatcher — delegated to `nebula-runtime`.
- Not a plugin loader or isolator — see `nebula-sandbox`.
- Not an expression evaluator — see `nebula-expression`.
- Not a retry scheduler — engine-level node re-execution from `ActionResult::Retry` is
  `planned` (§11.2); canonical retry surface is `nebula-resilience` inside an action.

## Maturity

See `docs/MATURITY.md` row for `nebula-engine`.

- API stability: `partial` — `WorkflowEngine` and `ExecutionResult` are in active use;
  known open debts (see Appendix) affect correctness boundaries.
- `ExecutionBudget` is ephemeral (not persisted on resume) — §11.5 debt.
- Downstream-edge gate only blocks local edges, not the full graph (§10 narrower than
  advertised for multi-hop conditional flows).

## Related

- Canon: `docs/PRODUCT_CANON.md` §10, §11.1, §12.2, §13.
- Engine guarantees: `docs/ENGINE_GUARANTEES.md`.
- Glossary: `docs/GLOSSARY.md` §2 (execution authority).
- Siblings: `nebula-execution` (state types), `nebula-storage` (repo), `nebula-runtime`
  (dispatcher), `nebula-workflow` (DAG → `ExecutionPlan`), `nebula-resilience`
  (in-action retry), `nebula-plugin` (registry).

## Appendix

### Known open debts (L4 detail)

| Gap | Location | Canon impact |
|---|---|---|
| `ExecutionBudget` not persisted in `ExecutionState` — budget is lost on resume | `src/engine.rs:796` | §11.5 durability matrix: budget is **ephemeral** |
| Original workflow input not persisted — resume cannot replay from input | `src/engine.rs:809` | §11.5 + §11.2 retry/resume story narrower than optimal |
| Downstream-edge gate blocks only **local** edges, not the full graph | `src/engine.rs:1808` | §10 conditional-flow gate is narrower than advertised |
| `ExecutionBudget` moved to `nebula-execution` — import cleanup pending | `src/engine.rs:20` | documentation / import hygiene |

### Architecture notes

- **Deny-by-default credential allowlist** (`credential_accessor.rs`): an empty allowlist denies
  every request (canon §12.5, §4.5). Per-action allowlists are populated via
  `WorkflowEngine::with_action_credentials`; an action whose credentials were never declared to
  the engine falls through to the deny baseline. There is no "fail-open" escape hatch.
- **No resource allowlist** (`resource_accessor.rs`): unlike credentials, there is no allowlist
  for resources — any registered key may be acquired by any action. Resource scoping is
  intentionally owned by the topology layer (e.g. pool scope, daemon scope), not the engine.
- **Cross-layer bridges**: `credential_accessor.rs` and `resource_accessor.rs` bridge business-
  layer traits into engine concrete types. Architecturally these belong to `nebula-credential`
  / `nebula-resource` as extension points; the move is a candidate refactor when the gaps above
  are fixed.
- **14 intra-workspace dependencies** — intentional for a composition root, but every new dep
  must be justified against the layer rules in `CLAUDE.md`.
