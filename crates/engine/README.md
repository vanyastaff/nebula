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
- `credential` module — engine-owned credential runtime surface:
  - `CredentialResolver`, `ResolveError`
  - `StateProjectionRegistry`, `StateProjectionError` — engine-side
    `state_kind`-keyed projection dispatcher (deserialize stored bytes
    → project to `Scheme`). Distinct from the canonical KEY-keyed
    `nebula_credential::CredentialRegistry` (Tech Spec §3.1, §15.6).
  - `execute_resolve`, `execute_continue`, `ResolveResponse`, `ExecutorError`
  - `rotation` (feature-gated) orchestration facade
- `EngineResourceAccessor` — scoped resource accessor injected into action contexts.
- `NodeOutput` — per-node output threaded between execution levels.
- `DEFAULT_EVENT_CHANNEL_CAPACITY` — default backpressure bound for the event channel.
- `DEFAULT_BATCH_SIZE` / `DEFAULT_POLL_INTERVAL` — tunables for `ControlConsumer`.

Re-exports from `nebula-plugin`: `Plugin`, `PluginKey`, `PluginManifest`, `PluginRegistry`,
`ResolvedPlugin`. The registry holds `Arc<ResolvedPlugin>` — a per-plugin wrapper with eager
action/credential/resource caches enforcing the namespace invariant at construction (ADR-0027).

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
- Two retry surfaces, disjoint by trigger boundary (per ADR-0042):
  - **In-call (Layer 1)** — `nebula-resilience::retry_with` lives inside an action
    around outbound calls. The engine sees only the action's final outcome.
  - **Operator-declared (Layer 2)** — `NodeDefinition.retry_policy` /
    `WorkflowConfig.retry_policy`. After a `Running → Failed` transition the
    engine consults the effective policy, parks the node in
    `NodeState::WaitingRetry` with `next_attempt_at`, and re-dispatches the
    action when the timer fires. Cancel / explicit-terminate / wall-clock
    budget breach drains parked retries to `Cancelled` without re-dispatching.
    Global cap via `ExecutionBudget.max_total_retries` (canon §11.2).

## Maturity

See `docs/MATURITY.md` row for `nebula-engine`.

- API stability: `partial` — `WorkflowEngine` and `ExecutionResult` are in active use;
  known open debts (see Appendix) affect correctness boundaries.
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
| `ExecutionBudget` moved to `nebula-execution` — import cleanup pending | `src/engine.rs` | documentation / import hygiene |

### Recently closed debts (ROADMAP §M0)

| Closed debt | Closed by | Verification |
|---|---|---|
| `ExecutionBudget` not persisted in `ExecutionState` — budget lost on resume | issue #289 | `set_budget` at `state.rs:218`; restored at `engine.rs:1433-1444`; tests `resume_restores_persisted_budget` and `resume_falls_back_to_default_budget_on_legacy_state` |
| Original workflow input not persisted — resume could not replay from input | issue #311 | `set_workflow_input` at `state.rs:206`; restored at `engine.rs:1487-1497`; test `resume_restores_original_workflow_input` |
| `ActionResult::Terminate` not propagated to `ExecutionTerminationReason::ExplicitStop` / `ExplicitFail` — execution audit lost intent vs system-driven termination | ROADMAP §M0.3 | `set_terminated_by` at `state.rs:240`; engine wiring at `engine.rs:1986-area`; `determine_final_status` priority ladder at `engine.rs:3590`; surfaced via `ExecutionResult.termination_reason` and `ExecutionEvent::ExecutionFinished.termination_reason` |

### Recently closed debts (ROADMAP §M2.1)

| Closed debt | Closed by | Verification |
|---|---|---|
| `NodeDefinition.retry_policy` / `WorkflowConfig.retry_policy` were declared and serialised but never read by the engine — operator-level retry was a §4.5 false capability | ADR-0042 + ROADMAP §M2.1 (foundation PR #627 + engine wiring PR) | `NodeState::WaitingRetry` (`crates/workflow/src/state.rs`); `NodeExecutionState::next_attempt_at` + `ExecutionState::total_retries` + `ExecutionBudget::max_total_retries` (`crates/execution/src/state.rs`, `context.rs`); engine retry decision + `tokio::select!` retry-pending heap (`crates/engine/src/engine.rs` `compute_retry_decision`, `effective_retry_policy`, `run_frontier`); 9 integration tests at `crates/engine/tests/retry.rs`; shift-left validation in `validate_workflow` (`crates/workflow/src/validate.rs`) |
| `ExecutionOutput::Inline(Value)` newtype-tagged variant silently failed `serde_json::to_value` for primitive payloads (string / number / bool / null) — surfaced when M2.1 T4 began pushing `NodeAttempt::output` records | ADR-0042 (engine wiring PR) | `Inline { value }` struct variant (`crates/execution/src/output.rs`); wire format moved from object-only `{"type": "inline", ...spread fields...}` to `{"type": "inline", "value": <any>}` |

### Recently closed debts (ROADMAP §M2.2)

| Closed debt | Closed by | Verification |
|---|---|---|
| `executions.lease_holder` / `lease_expires_at` (Layer 1) heartbeat enforcement across runner restarts not verified by integration tests — `crates/execution/README.md:138` warned `Schema may precede enforcement / Do not imply lease safety` | ROADMAP §M2.2 | Engine integration tests in `crates/engine/tests/lease_takeover.rs` (heartbeat-loss takeover, cancel redeliver, replay lease-less invariant); PG integration in `crates/storage/tests/execution_lease_pg_integration.rs` (8 tests covering `acquire_lease` / `renew_lease` / `release_lease` semantics + multi-runner takeover); loom probe at `crates/storage-loom-probe/src/lease_handoff.rs` + `tests/lease_handoff_loom.rs` (3 exhaustive scheduling models); chaos test at `crates/storage/tests/execution_lease_chaos.rs` (high-contention holder-uniqueness invariant) |
| Sprint E Layer-2 schema (`claimed_by` / `claimed_until` + indexes from `migrations/postgres/0011_executions.sql`) and the planned `repos::ExecutionRepo` trait in `crates/storage/src/repos/execution.rs` lacked inline boundary documentation — research agents could re-misclassify them as legacy | ROADMAP §M2.2 / T1' | Module-level `//!` note in `crates/storage/src/repos/execution.rs` cross-references `lib.rs:65-87` Layer-2 docs and ROADMAP "Out of scope for 1.0"; header comments in both `migrations/{postgres,sqlite}/0011_executions.sql` flag the lease columns + indexes as Sprint E (1.1) scaffolding |
| Lease lifecycle methods on `PgExecutionRepo` and `InMemoryExecutionRepo` ran silently — no tracing on acquire / renew / release outcomes | ROADMAP §M2.2 / T10 | `tracing::debug!` on success, `tracing::warn!` on contention / holder-mismatch, `tracing::error!` on `renew_lease: rejected` (signals heartbeat loss to operators) — `crates/storage/src/backend/pg_execution.rs:144-207` and `crates/storage/src/execution_repo.rs:594-641` (parity) |

**Layer 2 lease enforcement remains scoped to Sprint E (1.1)** per the
ROADMAP "Out of scope for 1.0" entry — M2.2 closes Layer 1 only.

### Recently closed debts (ROADMAP §M1)

| Closed debt | Closed by | Verification |
|---|---|---|
| Skip-propagation correctness on non-trivial topologies (multi-hop chain, diamond, mixed-source aggregate, all-sources-skipped, sibling activation) was undocumented and untested — `propagate_skip` recursion was not exercised beyond a single linear-3-node test | ROADMAP §M1.1 | 5 integration tests at `crates/engine/tests/integration.rs` (`skip_propagates_transitively_through_three_hop_chain`, `diamond_with_one_skipped_branch_still_completes`, `aggregate_with_one_skipped_source_fires`, `aggregate_with_all_sources_skipped_propagates_skip`, `multi_hop_skip_with_sibling_activation_still_runs`); all green |
| Dead `WorkflowEngine.expression_engine` field with misleading `#[expect(dead_code)]` reason ("wired up... but not yet called at runtime"). Spec 28 §2.2 already settled conditional routing via `ControlAction` nodes — no engine-level edge expression to evaluate; the shared `Arc<ExpressionEngine>` lives in `ParamResolver` (the only consumer) | ROADMAP §M1.2 | Field removed at `engine.rs:125-130`; struct init at `engine.rs:262` no longer clones; `cargo clippy --workspace --all-targets -- -D warnings` green |
| Stale Public API listing in `crates/workflow/README.md` advertising removed types (`EdgeCondition`, `ErrorMatcher`, `ResultMatcher`); 880-line `crates/workflow/docs/Architecture.md` pre-Spec-28 planning doc with no stale-marker | ROADMAP §M1.3 | `workflow/README.md` rewritten to describe `Connection` as a pure wire; `Architecture.md` frontmatter status changed to `stale-pre-spec-28` with drift table at top |

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
