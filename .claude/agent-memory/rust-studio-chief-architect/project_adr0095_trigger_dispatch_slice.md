---
name: adr0095-trigger-dispatch-slice
description: ADR-0095 "first real trigger dispatch" vertical slice — Created-row producer ordering, no prod emitter install site, EngineExecutionSink placement, RoutingResolver seam
metadata:
  type: project
---

ADR-0095 next unit after U1/U2 = a thin VERTICAL SLICE proving the live trigger→queue→orchestrator→engine path on REAL types (a prior "latent U3 emitter" plan was rejected 3/3 by a panel — wired to a fixture it proves nothing U1/U2 didn't already lock). See [[adr0095-u3-durable-emitter]] (superseded framing) and [[project_adr0095_spikes]].

**The real producer the slice must own = the `Created` execution-row write.**
- `resume_execution` (`engine.rs:1711`) and `EngineControlDispatch::dispatch_start` (`control_dispatch.rs:162`) read status first and return `Rejected("execution … not found — start command orphaned")` on `None`. So a trigger-origin Start MUST create the `Created` row before/with the dispatch row, else the sink has nothing to resume.
- API start path proves the contract (`crates/api/src/domain/execution/handler.rs:308–355`): `validate_for_dispatch` → `ExecutionId::new()` → `ExecutionState::new(...)` (seeds `Created`) + `set_workflow_input` → `create_execution_scoped` → `enqueue_start_scoped`.
- **Ordering invariant (exactly-once):** emitter calls `claim_and_enqueue_start` FIRST; creates the `Created` row ONLY on `Dispatched`. Creating before the claim would orphan a `Created` row on `Duplicate`. The `Created` write is a SECOND write outside the dedup+Start tx — a crash between leaves a queued Start with no row → `resume_execution` Rejected → `mark_failed` (at-most-once degrade, NEVER double-spawn). Folding it into the tx is post-slice storage work.

**R2 Duplicate semantics:** `claim_and_enqueue_start` (`inmem/job_dispatch.rs:251`) returns `Duplicate` BEFORE inserting the loser's start row (line 272 returns, 276 skipped); the dedup set stores only the scope+`(trigger_id,event_id)` key, NOT the winner's execution_id. `TriggerDedupRow` carries execution_id but the port returns only the enum. ⇒ slice's `emit -> Result<ExecutionId, ActionError>` returns the just-minted id; on Duplicate the minted id is wasted (acceptable, no orphan). Widening to `DispatchResult{execution_id, outcome}` is deferred behind `DispatchOutcome`'s `#[non_exhaustive]`+`#[must_use]` (D1).

**No prod trigger daemon installs an emitter today.** Every `TriggerRuntimeContext::new` non-test call site is webhook-transport (`api/src/transport/webhook/*`); both engine `emit_execution` sites (`engine.rs:5900`, `runtime/runtime.rs:1654`) are `#[tokio::test]` bodies. Default is `NoopExecutionEmitter` (`context.rs:436`, fail-closed). ⇒ "install the real emitter" is HARNESS-SCOPED until D1; do not claim a prod wiring that doesn't exist.

**Placement decisions (panel-sound, keep):**
- `EngineExecutionSink` impls `nebula_orchestrator::ExecutionSink` in `nebula-engine` (`daemon/execution_sink.rs`) — NEW edge engine→orchestrator, verified ACYCLIC (orchestrator deps = storage-port+core+metrics only, `orchestrator/Cargo.toml`). Mirrors `EngineControlDispatch` (`control_dispatch.rs:72`); reuses its `drive` resume-idempotency (Leased/terminal→Ok).
- `RoutingResolver` PORT (trait) in engine `daemon/routing.rs`, NOT a fixed `DispatchRouting` struct (one emitter serves many triggers; routing is per-workflow). Slice ships `StaticRoutingResolver` (one real mapping; `required_plugin_key` + `capability_tags` + `target_flavor_sha=const SLICE_FLAVOR_SHA`). D1 swaps the impl, not the seam. `workflow_id`/`trigger_id` already live on `TriggerRuntimeContext` (`context.rs:421–429`).
- emit seam reshape: `emit(&self, input, event_id: Option<IdempotencyKey>)`. `IdempotencyKey` already exists (`nebula_action::IdempotencyKey`, lib.rs:124, not a secret). `None`⇒unconditional/`row=None`.

**Ripple (object-safe dyn ⇒ miss = compile error):** trait `capability.rs:34`; impls `NoopExecutionEmitter` capability.rs:187, `SpyEmitter` testing.rs:385, `FailingEmitter`/`DropCountingFailingEmitter` dx_poll.rs:605/728; wrapper `emit_execution` context.rs:518; 2 PROD call sites = engine `daemon/event_source.rs:245` + action `poll/mod.rs:1276` (NOTE: prompt's `action/src/event_source.rs:244` path is WRONG — event-source emit is in ENGINE daemon).

SCOPE GUARD: no CapabilityTag resolver, no flavor-SHA derivation, no nebula-worker crate, no multi-flavor — all D1. Postgres serializability M7-gated, never assert green (InMemory baseline only). Verdict was ACCEPTABLE with 2 binding conditions: (1) Created-row producer in-plan + cancel-safety honesty; (2) StaticRoutingResolver real not stub, harness-honest install.
