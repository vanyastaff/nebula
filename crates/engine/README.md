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
`Cancel` / `Terminate`. `nebula-worker::WorkerRuntimeBuilder` installs that
pairing and derives its exact-flavor claim filter from the engine's retained
registry. The worker app selects the concrete queue and execution adapters. `Terminate`
also shares the cooperative-cancel body until a distinct forced-shutdown path
is wired (ADR-0016).

For Start, the configured dispatch validates the exact stored contract and
checkpoint before the execution owner atomically completes the control claim
and grants its lease. Action duration therefore does not retain Start delivery.
The typed acceptance phase prevents a second queue write after acceptance or
an uncertain commit acknowledgement. Worker startup and periodic discovery
find accepted turns through durable markers independent of queue retention.
Recovery checks the exact snapshot and obtains a fresh owner fence; unarmed
paused waits remain parked. Resume and Restart still use their existing
delivery lifecycle.

## Input Admission

Named workflow parameters are schema-internal authored fields. The engine retains
explicit `Expression` versus `Template` intent, admits the tree directly with the
selected action schema's `validate`, and evaluates its retained programs once.
A lone Template expression still produces a string. Evaluator output is data and
is never parsed as another expression. Undeclared fields and Any roots do not
implicitly authorize expressions.

Whole predecessor values and raw runtime ingress use the schema's serde-wire
converter instead; internal union parameters are not interpreted as external enum
wire. `ActionInput::Resolved` carries the resulting schema-bound proof through typed
dispatch, including repeated stateful iterations. A mismatched complete schema is
rejected. Cancellation during resolution prevents provider execution. Remote effects
receive an explicit trusted JSON disclosure after proof verification, without changing
operation-ledger authority or admission. Input diagnostics redact evaluator sources.

## Role

*Runtime control.* `WorkflowEngine` drives a supplied workflow and supplied
ports using DAG-level parallelism and bounded concurrency. Deployment roots
under `apps/` remain responsible for adapter selection, plugin catalogs,
configuration, and process lifecycle.

## Public API

- `WorkflowEngine` — entry point: executes workflows level-by-level with bounded concurrency.
  Exposes `cancel_execution(id) -> bool` so control-queue `Cancel` signals reach the live
  frontier loop (ADR-0008 A3; ADR-0016).
  Durable turns require `with_plan_flavor_runtime(loader, frozen_registry, bundles)`
  and an execution-owned immutable contract bundle from the same backend as the
  execution stores. State pins, tenant, workflow revision and plugin set must agree
  with that checked bundle and the retained plan before any factory is instantiated.
  They load and execute only the checked recorded
  graph with factories retained from that frozen snapshot; replacing authoring
  JSON or the mutable action registry cannot change an admitted turn.
  Each processed node commits its complete routing outcome and payload inside
  the fenced execution-state checkpoint. Resume reconstructs outputs and live
  edges from that snapshot; auxiliary node-result stores are technical caches.
  Unsupported or incomplete warm checkpoints fail before factory instantiation.
  `worker_flavor_context()` derives dispatch routing from the same snapshot.
  The direct-definition execution methods remain separate technical entry points.
  Recorded variables, per-node timeouts, non-default checkpoint cadence and
  unresolved binding requirements currently fail closed before instantiation.
- `WorkflowActivationService` — compiles against one frozen registry, installs
  the exact revisions and atomically publishes their workflow activation.
- `WorkflowStartService` — admits an activated workflow and persists its execution,
  contract bundle, live revision references and Start command in one owner transaction.
  Caller keys and trigger event identities occupy separate durable namespaces.
  Replays read the original acceptance before consulting the current workflow;
  an unknown commit permits one retry of the identical original envelope.
- `DurableExecutionEmitter` — delegates trigger fan-out to `WorkflowStartService`.
  It creates one Control Start command; it does not also enqueue a Job Start.
  Unkeyed uncertain acceptance is non-retryable at the action boundary because
  re-entering the emitter would allocate another execution identity.
- `ControlConsumer` — durable control-queue consumer drained via `ControlQueue`
  (canon §12.2, ADR-0008). Its dispatch implementation supports all five
  commands — `Start` / `Resume` / `Restart` / `Cancel` / `Terminate` — via
  `EngineControlDispatch`. First-party workers use `for_flavor`, which
  selects persisted exact revision references before applying the claim limit.
  Optional
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
- `PlanFlavorRevisionInstaller` / `PlanFlavorRevisionLoader` —
  contract-plane/runtime-control bridges. The installer revalidates and
  encodes one immutable authority-free plan/flavor pair for the technical
  catalog; the loader decodes only the requested exact pair, recomputes both
  identities, and checks it against one `FrozenPluginRegistry`. The successful
  load witness retains that exact registry snapshot so compatibility evidence
  cannot outlive or detach from it. Neither type carries tenant, admission,
  or reference authority. SQLite and PostgreSQL retain the exact revisions;
  activation and start composition install and load them through these
  bridges.
- `EngineError` — typed engine-layer error (includes `Telemetry` when metric registration fails at
  `WorkflowEngine::new` time).
- `EffectExecutionError` — bounded remote-effect failures including durable unknown
  outcome and known applied output unavailability. The private execution-owned driver
  binds canonical requests to the admitted effect contract, tenant, node and operation
  slot. Only acknowledged ledger grants reach the adapter; outcome acknowledgement
  recovery uses database reads and exact evidence recommits. Generic `ActionRuntime`
  accepts only explicitly declared `ReadOnly` factories: a caller-supplied context may
  carry any resource accessor, so its public entry points refuse `Journaled` actions
  (`EffectRequiresOwner`). The engine's own node dispatch admits `Journaled` factories
  without a remote-effect capability. Only handle-routed effects are journaled: raw
  egress the action opens itself is outside the journal. Since 0.27.0 the engine's
  `ResourceAccessor` serves resource handles only — there is no raw-lease route
  (`ResourceGuard<R>` slots, `acquire_resource_by_id` and `ResourceAccessor::acquire_any`
  are removed) — so `ReadOnly` and `Remote` actions get read-only handles, as does a
  `Journaled` one without a node journal. A key a branch scope holds fails closed with a
  scope violation instead of reaching a global row. Plans recorded without an effect
  field stay refused.
- **Node effect journal** (`effect_driver::journal`, crate-private). A frozen,
  stateless or stateful `Journaled` action on a durable turn (operation ledger and
  execution fence present) runs under one `NodeEffectJournal` per node attempt, through
  `ActionRuntime::execute_journaled_action` and an engine-private admission witness.
  Its resource handles (`Manager::handle_any_journaled`) drive every `Idempotent` /
  `Write` unit through the journal, which records it as one operation-ledger slot via
  the shared `LedgerSlot` core: prepare (natural key `(scope, execution, node,
  occurrence)`), grant, explain (not crossed / ambiguous), settle (exact evidence
  recommit). Building the journal costs nothing durable: no ledger write happens until
  the first effect is prepared, and reads are never prepared; `conclude` always reads
  the node's occurrences once (a crash before an attempt was recorded leaves the next
  attempt at the same generation), and a node that prepares an effect reads them once
  more before its first prepare. Each grant carries what is left of the ledger's
  window for the call, and the resource runtime stops the unit there; a grant with
  nothing left is withheld. A slot's contract identity binds the destination (resource
  key, credential slot identity, configuration fingerprint) and `RECORD_OUTPUT`, so a
  reload to another endpoint or a changed recording policy is a mismatch. Occurrences are the
  resource runtime's positional `unit/v1/#{ordinal:06}`, one sequence for all the node
  attempt's effect units (every resource, operations and sessions) restarting per node
  attempt, taken when a unit starts preparing (its first poll — a submission dropped
  unpolled takes none); the resource, unit kind, operation (or session) name and version
  belong to the contract identity, so a redeploy that changes the effect at a recorded
  position is a mismatch with nothing sent, not a fresh slot. A run that reaches its
  effects in another order (across resources, or a session before an operation) or adds/removes one
  before recorded ones meets other intents' slots: a mismatch (identical intents are
  interchangeable). Before its first prepare the journal reads the node's earlier
  occurrences once and refuses a fresh slot at a position an earlier attempt left empty
  below one it recorded (a mismatch, nothing written), since the effect may be one
  recorded further on. An engine retry reuses the occurrences:
  a settled effect replays its recorded output with no provider call, an opaque
  ambiguous one is unknown, a retryable failure may be granted again within the
  slot's budget (`Operation::max_attempts`). **Stateful actions** are journaled per
  iteration: the journal hands the resource runtime its labels
  (`EffectJournal::next_occurrence`), `it{n}/unit/v1/#{k:06}` with `n` the iteration in
  decimal without leading zeros (0 to 9999) and `k` restarting per iteration. All
  iterations run inside one node attempt and every attempt replays them from
  iteration 0 — a journaled stateful action takes no checkpoint sink (the runtime
  refuses both together). The runtime brackets each iteration with the journal's
  barrier (`IterationGate`): `begin_iteration(n)` requires no unit in flight (else
  `ENGINE:EFFECT_ITERATION_BARRIER`, with nothing waited for: the node's conclusion
  still drains the unit within its full limit) and opens the label namespace;
  `end_iteration` — after the iteration returned, `Ok` or `Err` — drains its units
  within the node drain limit and stops the loop (`RuntimeError::EffectJournal`,
  replaced by the verdict) when the journal holds a failure: an unknown outcome in
  the iteration (even one the action swallowed), a mismatch, a deferring ledger or
  lease failure, or — for an iteration that returned `Ok` — an effect an earlier
  attempt recorded in it (or before it) that this attempt never met (a mismatch; a
  failing iteration keeps its own failure for the conclusion to judge). A unit still
  in flight past the drain limit fails the barrier
  (`ENGINE:EFFECT_ITERATION_BARRIER`): the journal closes and the node's conclusion
  does not wait for it again; if the unit had been granted a call, the verdict
  records the call as ambiguous and the node fails `ENGINE:EFFECT_OUTCOME_UNKNOWN`
  instead. Positions order by `(iteration, ordinal)` (a flat label is iteration 0)
  and a fresh slot is prepared only when it is consistent with what earlier attempts
  recorded: it must not lie below a recorded position of its family (a gap), nor
  above a recorded consequential one this attempt has not met, passed by on another
  path (a mismatch; a fresh slot above a recorded position another unit is still
  preparing waits for it). A position a unit took and gave up on before its ledger
  prepare answered (past its deadline, cancelled, dropped; the resource runtime
  releases every position with `EffectJournal::release_occurrence`) is *abandoned*:
  a fresh slot above it, and a barrier past a recorded effect above it, are refused
  deferring (`AcknowledgementUnknown`, nothing sent), so the retry meets the position
  again instead of halting on a mismatch. Labels of the other family (flat
  versus `it{n}/`) recorded by an earlier attempt are refused as a changed action
  kind; labels are parsed strictly (no leading zeros). The first barrier reads the
  node's occurrences, if no prepare did. **Determinism contract**: a replayed
  iteration must submit the same effects in the same order — inputs a replay does
  not reproduce (clocks, randomness, unrecorded reads) diverge, and the divergence
  halts the node `ENGINE:EFFECT_OCCURRENCE_MISMATCH` before any recorded effect is
  sent again; only effects past everything recorded are sent. **Order**: within a
  family a lower effect is never applied after a higher one. In one attempt, a unit
  whose ledger prepare began and never answered (cancelled or past its deadline
  mid-call, or the acknowledgement lost) leaves its position uncertain — its row may
  exist — and no fresh slot above it is prepared: the prepare is refused as a
  deferring `AcknowledgementUnknown`, nothing is sent, and the node defers so the next
  attempt replays in order (only a fresh prepare leaves its position uncertain).
  Across attempts, a recorded slot is never sent again when an earlier attempt
  recorded an effect that may have been applied — a recorded success, or a call that
  may have crossed with no recorded outcome (a definitive rejection applied nothing
  and orders nothing) — at a higher position of its family that the program ran after
  it: in a later iteration, or one that does not list it as **concurrent**. A lower
  slot that changed nothing (only prepared, or every call explained not crossed)
  failed unsent before the program moved on: it is refused `superseded` (`Permanent`
  / `NotSent`, no failure of the journal's own), so a deterministic program that
  handled that failure handles it again and replays on (the replayed kind is
  permanent whatever the original was). A lower stable-key slot whose call crossed
  without an outcome may have applied before or after: its outcome is recorded
  unknown and the node halts `ENGINE:EFFECT_OUTCOME_UNKNOWN`. With nothing ordered
  after it, an unsettled slot is granted again on retry. Every fresh slot records, at its
  first prepare (`EffectSlotBinding::concurrent_with`, kept in the protocol record),
  the exact lower positions of its iteration whose unit was still open — handed out
  and not yet settled (`EffectJournal::finish_occurrence`, signalled when the unit
  settles, whoever keeps its handle) — at most 64, the nearest kept (the rest read as
  settled before it). Units awaited together (`join!`, `FuturesUnordered`) are
  concurrent: a recovery replays the unsettled one under its recorded provider key (at
  least once) instead of halting, while a lower slot that settled before the later
  one began — even inside a run of concurrent units — stays refused. Every fresh slot
  records its list, `[]` included; a slot recorded without it (by the journal before
  this rule) orders nothing, so an upgraded node recovers as it would have before.
  Fresh prepares of a family run in position order: a fresh slot's ledger prepare
  waits until every lower position handed out in the attempt resolved its prepare
  (acknowledged, refused, given up, or left uncertain — then it is refused deferring),
  so a higher row is never written while a lower one may or may not exist; provider
  calls stay concurrent (a unit polled once and then parked by the program before its
  prepare resolves holds the fresh prepares above it until it resumes, gives up or is
  dropped). **Replay delays**: a replay that has not reached
  its frontier (an earlier attempt recorded an effect in a later iteration) skips the
  `Continue` delay — that iteration already ran, after it; from the frontier on every
  delay is honoured (an iteration that recorded no effect cannot tell). A node
  cancelled mid-iteration, during the barrier's drain, or
  during the delay between iterations, ends the iteration at once: a
  later detached submission is refused closed, with no failure of its own, so the
  conclusion drains only the units already in flight. A stateful node's journal admits a unit only while an iteration is open:
  admission and the iteration rollover are one transition under the journal's lock,
  and a unit a detached task submits between iterations is refused `between_runs`
  (`Permanent` / `NotSent`) while the verdict records
  `ENGINE:EFFECT_ITERATION_BARRIER`. A barrier's occurrence read is bounded by what
  is left of the drain limit (at least 5 s) and defers the node on timeout. One node
  attempt prepares at most `MAX_NODE_SLOTS` (10 000) fresh journaled effects (replays of
  recorded positions do not count):
  a further fresh prepare is refused `slot_cap_exceeded`, nothing is sent, and the node
  fails `ENGINE:EFFECT_JOURNAL_SLOT_CAP`. Every slot records the provider idempotency key
  `base64url(SHA-256(frame("nebula.idempotency-key.v1") ‖ frame(frame(org) ‖
  frame(workspace)) ‖ frame(resource) ‖ frame(operation) ‖ u32_be(version) ‖
  frame(developer part | frame(execution) ‖ frame(node) ‖ frame(occurrence)))))` — no
  attempt number, no execution id with a developer part — and a unit presents the key
  read back from the prepared record. A call granted and never explained (a crash, a
  unit that outlived its node) is recorded as an ambiguous crossing on the next
  prepare, never from `Drop`. On every exit of the node — after the action returns,
  and on each exit before it runs (cancellation, input resolution, credential
  refresh, rate limit, contract checks), so an earlier dispatch's unknown call is
  never reported as a failure an error strategy could retry or continue past —
  `conclude` drains the units
  for at most `min(OPERATION_DEADLINE_CAP, execution deadline left)`, closes the
  journal and records every unexplained call as ambiguous within the same limit, then
  reads the node's occurrences within what is left of it (at least 5 s; a read that
  does not answer defers the turn like an unavailable ledger); its
  verdict overrides the node's result: a lost lease or unknown acknowledgement
  releases the lease without finalizing, any slot whose call may have crossed without
  a recorded outcome (unknown, outstanding, ambiguous, or held past the limit by a
  stuck unit) fails the node `ENGINE:EFFECT_OUTCOME_UNKNOWN` (even if the action
  swallowed the unit's error), and a changed request, key part, credential binding,
  configuration or recording policy under a recorded occurrence fails it
  `ENGINE:EFFECT_OCCURRENCE_MISMATCH` with nothing sent — as does a node about to
  succeed although an earlier attempt recorded an effect (settled, or a call that
  crossed) this attempt never met again. Such a verdict — like a remote effect's unknown
  outcome or unreadable evidence (`EffectExecutionError::halts_execution`) — takes no
  error strategy: `IgnoreErrors`, `ContinueOnError` and OnError edges never recover or
  route past it; the node fails and the execution stops. A node that fails before
  meeting such an earlier effect again keeps its own error
  (`EngineError::SkippedJournaledEffect`): its retry policy may re-dispatch it (the
  retry replays the effect), but a final failure halts the execution instead of being
  ignored or routed — also when the journal noted a failure of its own that would not
  halt (a detached unit refused between iterations). **Invariants** (module docs of
  `effect_driver::journal`): S1 no effect sent twice under different keys; S2 no
  recorded effect re-sent after divergence; S3 a lower effect never applied after a
  higher one the program ran after it; S4 concurrent units replay at least once under
  their recorded keys; S5 every wait bounded; S6 legacy records order nothing; S7 a
  cancelled node stays cancelled; S8 an unknown outcome is never masked. A correct
  deterministic program is stranded only by a crossed call with no recorded outcome
  that is opaque, or that a later applied effect is ordered after. Counters:
  `nebula_effect_journal_prepares_total{phase}`,
  `nebula_effect_journal_refusals_total{step,refusal}`,
  `nebula_effect_journal_verdicts_total{code}`. Every other `Journaled` node keeps
  read-only handles (reads run, writes are refused `NotSent`), and the refusal says
  why: a control action ("control actions decide flow and must not cause effects;
  move effects to a stateless action"), an agent action ("agent effects are
  not journaled; the agent profile is planned"), a stream or other kind ("effects of
  this action kind are not journaled"), or a stateless or stateful one without
  execution stores ("journaled effects need execution stores"). The crate-private
  `JournalShape` maps a kind to how it is journaled: `Flat` (stateless), `Iterated`
  (stateful, per iteration), `None` (control, agent, stream and the rest).
- `ExecutionEvent` — broadcast event type emitted via `nebula-eventbus`.
- `EngineCredentialAccessor` — scoped credential accessor injected into action contexts.
- `EngineResourceAccessor` — scoped resource accessor injected into action contexts.
- `NodeOutput` — per-node output threaded between execution levels.
- `DEFAULT_EVENT_CHANNEL_CAPACITY` — default backpressure bound for the event channel.
- `DEFAULT_BATCH_SIZE` / `DEFAULT_POLL_INTERVAL` — tunables for `ControlConsumer`.

Re-exports from `nebula-plugin`: `Plugin`, `PluginKey`, `PluginManifest`, `PluginRegistry`,
`ResolvedPlugin`. The registry holds `Arc<ResolvedPlugin>` — a per-plugin wrapper with eager
action/credential/resource caches enforcing the namespace invariant at construction (ADR-0027).

## Contract

An empty node-parameter map preserves the supplied workflow or predecessor root
value; a nonempty map constructs an object. Neither path converts supplied
objects to unit `null`. Default flow connections carry the whole root value.
Named target ports are reserved for declared support bindings; named flow ports
fail with `EngineError::UnsupportedInputPort` before execution. Typed dispatch
preserves scalar values, including literal template-looking strings.

Exact replay retains the archived schema and parameter representation. An archived
empty record remains an object contract, even when a live factory with the same
key now declares unit `null`; incompatible live contracts are rejected without
rewriting the stored plan or selecting a replacement revision.

- **[L2-§11.1]** Execution state transitions go through `ExecutionStore::commit` (CAS on
  `version` plus the lease `FencingToken`; the batch carries state + outbox + journal in one
  transaction). No handler inside the engine mutates execution state in-memory or invents a
  parallel lifecycle. Seam: `crates/storage-port/src/store/execution.rs — ExecutionStore::commit`.

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

- Not a storage implementation — see `nebula-storage-port` (store traits) and
  `nebula-storage` (in-memory / PostgreSQL adapters).
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
- Exact plan/flavor loading: `partial` — InMemory provides the internal
  reference model, while SQLite and PostgreSQL provide deployment adapters.
  The ordered schema, three-backend conformance, and atomic
  `materialize_start` path enforce durable admission for exact revisions.
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
| Sprint E Layer-2 schema (`claimed_by` / `claimed_until` + indexes from `migrations/postgres/0011_executions.sql`) and the planned `repos::ExecutionRepo` trait in `crates/storage/src/repos/execution.rs` lacked inline boundary documentation — research agents could re-misclassify them as legacy | ROADMAP §M2.2 / T1' | **Historical row:** the planned `repos::ExecutionRepo` and `repos::WorkflowRepo` RPITIT siblings were deleted as never-implemented placeholders (ADR-0072) — the deletion and the retained `repos::*` traits are documented in `crates/storage/README.md` §"Single storage architecture — the spec-16 port (ADR-0072)"; the Sprint E header comments in both `migrations/{postgres,sqlite}/0011_executions.sql` flag the lease columns + indexes as Sprint E (1.1) scaffolding |
| Lease lifecycle methods on the execution-store adapters (pre-ADR-0072 names: `PgExecutionRepo` / `InMemoryExecutionRepo`) ran silently — no tracing on acquire / renew / release outcomes. **Historical row:** those types were renamed by ADR-0072; the live adapters are `PgExecutionStore` (`crates/storage/src/postgres/execution.rs`) and `InMemoryExecutionStore` (`crates/storage/src/inmem/execution.rs`), implementing the port trait in `crates/storage-port/src/store/execution.rs` | ROADMAP §M2.2 / T10 | `tracing::debug!` on success, `tracing::warn!` on contention / holder-mismatch, `tracing::error!` on `renew_lease` rejected (signals heartbeat loss to operators) — on `acquire_lease` / `renew_lease` / `release_lease` at parity across both adapters, under `target=nebula_storage::lease` |

**Layer 2 lease enforcement remains scoped to Sprint E (1.1)** per the
ROADMAP "Out of scope for 1.0" entry — M2.2 closes Layer 1 only.

### Recently closed debts (ROADMAP §M1)

| Closed debt | Closed by | Verification |
|---|---|---|
| Skip-propagation correctness on non-trivial topologies (multi-hop chain, diamond, mixed-source aggregate, all-sources-skipped, sibling activation) was undocumented and untested — `propagate_skip` recursion was not exercised beyond a single linear-3-node test | ROADMAP §M1.1 | 5 integration tests at `crates/engine/tests/integration.rs` (`skip_propagates_transitively_through_three_hop_chain`, `diamond_with_one_skipped_branch_still_completes`, `aggregate_with_one_skipped_source_fires`, `aggregate_with_all_sources_skipped_propagates_skip`, `multi_hop_skip_with_sibling_activation_still_runs`); all green |
| Dead `WorkflowEngine.expression_engine` field with misleading `#[expect(dead_code)]` reason ("wired up... but not yet called at runtime"). Spec 28 §2.2 already settled conditional routing via `ControlAction` nodes — no engine-level edge expression to evaluate; the shared `Arc<ExpressionEngine>` lives in `ParamResolver` (the only consumer) | ROADMAP §M1.2 | Field removed at `engine.rs:125-130`; struct init at `engine.rs:262` no longer clones; `cargo clippy --workspace --all-targets -- -D warnings` green |
| Stale Public API listing in `crates/workflow/README.md` advertising removed types (`EdgeCondition`, `ErrorMatcher`, `ResultMatcher`); 880-line `crates/workflow/docs/Architecture.md` pre-Spec-28 planning doc with no stale-marker | ROADMAP §M1.3 | `workflow/README.md` rewritten to describe `Connection` as a pure wire; `Architecture.md` frontmatter status changed to `stale-pre-spec-28` with drift table at top |

### Architecture notes

- **Deny-by-default credential manifest** (`credential_accessor.rs`): only the current node's
  exact, site-qualified entries from the persisted V2 execution contract are resolvable (canon
  §12.5, §4.5). A missing manifest, slot, tenant match, or capability falls through to the deny
  baseline. There is no process-local population or "fail-open" escape hatch.
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
