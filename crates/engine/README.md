---
name: nebula-engine
role: Runtime Control (Graph Execution)
status: partial
last-reviewed: 2026-07-26
canon-invariants: [L2-10, L2-11.1, L2-12.2]
related: [nebula-execution, nebula-storage, nebula-runtime, nebula-workflow, nebula-resilience, nebula-plugin, nebula-credential, nebula-resource]
---

# nebula-engine

## Purpose

`nebula-engine` is reusable runtime control for graph execution. It builds an
`ExecutionPlan` from the workflow DAG, resolves node inputs from predecessor
outputs, transitions execution state through the storage ports, and delegates
action dispatch to the runtime. First-party deployment composition roots live
under `apps/`; this crate does not select adapters or own process lifecycle.

Canon §12.2 places the durable control-queue consumer implementation here.
`ControlConsumer` provides polling, claim/ack, and graceful shutdown, while
`EngineControlDispatch` implements `Start` / `Resume` / `Restart` and
`Cancel` / `Terminate`. Those five command paths are manually composed in
integration tests. No non-test first-party composition root currently
constructs `ControlConsumer` with `EngineControlDispatch`, so the deployed
server/worker path is not yet an end-to-end control consumer. `Terminate`
also shares the cooperative-cancel body until a distinct forced-shutdown path
is wired (ADR-0016).

## Role

*Runtime control.* `WorkflowEngine` drives a supplied workflow and supplied
ports using DAG-level parallelism and bounded concurrency. Deployment roots
under `apps/` remain responsible for adapter selection, plugin catalogs,
configuration, and process lifecycle.

## Public API

- `WorkflowEngine` — entry point: executes workflows level-by-level with bounded concurrency.
  Exposes `cancel_execution(id) -> bool` so control-queue `Cancel` signals reach the live
  frontier loop (ADR-0008 A3; ADR-0016).
- `ControlConsumer` — durable control-queue consumer drained via `ControlQueueRepo`
  (canon §12.2, ADR-0008). Its dispatch implementation supports all five
  commands — `Start` / `Resume` / `Restart` / `Cancel` / `Terminate` — via
  `EngineControlDispatch` (A2 + A3), but current first-party app roots do not
  install that pairing. Optional
  `ControlQueueEntry::w3c_trace_context` restores an OpenTelemetry parent on the
  dispatch span (`control_trace`, ADR-0050).
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
- `PlanFlavorRevisionInstaller` / `PlanFlavorRevisionLoader` — Task 13A
  contract-plane/runtime-control bridges. The installer revalidates and
  encodes one immutable authority-free plan/flavor pair for the technical
  catalog; the loader decodes only the requested exact pair, recomputes both
  identities, and checks it against one `FrozenPluginRegistry`. The successful
  load witness retains that exact registry snapshot so compatibility evidence
  cannot outlive or detach from it. Neither type carries
  tenant/admission/reference authority. Task 13A is component-only;
  SQLite/PostgreSQL retention and production start wiring remain Tasks
  14A/13B/20.
- `EngineError` — typed engine-layer error (includes `Telemetry` when metric registration fails at
  `WorkflowEngine::new` time).
- `ExecutionEvent` — broadcast event type emitted via `nebula-eventbus`.
- `EngineCredentialAccessor` — scoped credential accessor injected into action contexts.
- `credential` module — **bridge + test-harness only** (ADR-0092). The runtime
  itself (`CredentialResolver`, `execute_resolve`, `execute_continue`,
  `ResolveResponse`, `ExecutorError`, `RefreshCoordinator`, lease, rotation-state)
  was relocated to `nebula_credential::runtime`; this module **re-exports** those
  for backward-compatible `nebula_engine::credential::*` import paths and adds
  `default_in_memory_coordinator()` (constructs `InMemoryRefreshClaimRepo` from
  `nebula-storage` for tests / single-replica desktop mode). The resolver is
  generic over the concrete credential type `C` and calls `C::project(&state)`
  directly, so no type-erased projection registry is needed (the former
  `StateProjectionRegistry` was vestigial and removed in ADR-0088 D3; capability +
  metadata live solely on `nebula_credential::CredentialRegistry`). The per-slot
  rotation **fan-out** moved to `nebula-resource`.
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
  implementation (`ControlConsumer`; wiring decisions in ADR-0008).
  `EngineControlDispatch` implements all five commands — `Start` / `Resume` /
  `Restart` / `Cancel` / `Terminate` — and manually composed integration tests
  exercise them. Current first-party deployment roots do not construct this
  consumer, so those tests are component integration evidence rather than a
  deployed end-to-end claim. When installed, `Cancel` reaches the live frontier
  loop through the per-instance cancel registry
  (`WorkflowEngine::cancel_execution`; ADR-0016); `Terminate` currently shares
  the cooperative-cancel body.

- **[L2-§10]** Engine and API knife tests manually compose in-memory ports,
  `ControlConsumer`, and `EngineControlDispatch` to exercise Start/Cancel
  dispatch. They do not boot the first-party server and worker roots together
  and therefore do not prove the deployed golden path end-to-end.

## Non-goals

- Not a storage implementation — see `nebula-storage` (`ExecutionRepo`, storage backends).
- Not an action dispatcher — delegated to `nebula-runtime`.
- Not a plugin isolator — plugins register and run in-process via `nebula-plugin` (ADR-0091).
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
- Exact plan/flavor loading: `partial` — the checked runtime-control bridge
  exists after Task 13A, but only the InMemory reference model is available.
  It is not a production durability or admission claim until the ordered SQL
  schema, three-backend conformance, and atomic `materialize_start` path ship.
- Downstream-edge gate only blocks local edges, not the full graph (§10 narrower than
  advertised for multi-hop conditional flows).

## Related

- Canon: `docs/PRODUCT_CANON.md` §10, §11.1, §12.2, §13.
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
| Lease lifecycle methods on `PgExecutionRepo` and `InMemoryExecutionRepo` ran silently — no tracing on acquire / renew / release outcomes | ROADMAP §M2.2 / T10 | `tracing::debug!` on success, `tracing::warn!` on contention / holder-mismatch, `tracing::error!` on `renew_lease` rejected (signals heartbeat loss to operators) — added on `acquire_lease` / `renew_lease` / `release_lease` of `PgExecutionRepo` (`crates/storage/src/backend/pg_execution.rs`) and `InMemoryExecutionRepo` (`crates/storage/src/execution_repo.rs`) at parity, all under `target=nebula_storage::lease` |

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
- **14 intra-workspace dependencies** — runtime control spans several lower
  layers, but every new dependency must still be justified against the layer
  rules in `AGENTS.md`.
