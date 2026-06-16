---
name: adr0095-u3-durable-emitter
description: ADR-0095 U3 DurableExecutionEmitter plan decisions — engine placement, latent-emitter scoping fork, and the "no shared Start-msg builder" correction
metadata:
  type: project
---

ADR-0095 U3 (DurableExecutionEmitter) — owning-lead plan ratified at pre-code gate (2026-06-16). Decisions:

- **Crate placement = `nebula-engine`** (`crates/engine/src/daemon/durable_emitter.rs`, new). FORCED by graph: engine is the only crate depending on BOTH `nebula-action` (ExecutionEmitter trait) AND `nebula-storage-port` (TriggerDedupInbox/JobDispatchMsg). `nebula-action` has NO storage-port edge — impl cannot live there. No new crate, no new edge. Producer (emitter writes dispatch row) ≠ consumer (orchestrator claim→Start→mark_dispatched, U2); keep distinct, no engine→orchestrator nor orchestrator→action edge.
- **emit() seam reshape** = positional param `fn emit(&self, input: Value, event_id: Option<IdempotencyKey>)`, NOT an EmitRequest struct (premature DTO for a 2-scalar dyn method). Stays object-safe. `Option` encodes dedup-vs-unconditional (`None`⇒`claim_and_enqueue_start(row=None)`). Ripple: trait + NoopExecutionEmitter + SpyEmitter + 2 test emitters in dx_poll.rs + `emit_execution` wrapper (context.rs) + prod call `event_source.rs:245`. Sweep ALL `.emit(`/`emit_execution(` in action+engine. No shim.
- **Scoping fork = LATENT emitter (option i).** `JobDispatchMsg` needs `target_flavor_sha`/`required_plugin_key`/`capability_tags` — NO trigger-side source today (that resolver is D1 / flavor pipeline, later unit). U3 builds+tests the emitter against InMemory inbox but does NOT install it (daemon stays NoopExecutionEmitter). Routing fields come from an injected typed `DispatchRouting` value (test fixture in U3, D1 supplies real). Matches D2 "register arm zero prod callers" + D6 "resolver tested-but-uncalled" precedent. Named follow-up, not a silent stub.

**CORRECTION (supersedes captured U3 scope):** the captured scope said "reuse a pure Start-msg-BUILDER extracted from `enqueue_start_scoped`". WRONG against ground truth — `enqueue_start_scoped` (api/src/domain/execution/handler.rs:411) builds a **ControlMsg on the legacy ControlQueue**, a different DTO+queue from `JobDispatchMsg`/JobDispatchQueue. `JobDispatchMsg::new` is called ONLY by storage read-side hydration, never producer-side. There is NO shared builder to extract and the build must NOT manufacture one (it would bridge two incompatible queues = false abstraction the gate rejects). Honest one-fact-one-place reading: the orchestrator's `Start→resume_execution` (engine.rs:1710) consumer is the shared SINK; producers (API-origin vs trigger-origin) legitimately differ. Emitter constructs `JobDispatchMsg` directly via public `::new`.

**Why:** ratified D5 contract requires durable trigger→dispatch with source-natural `event_id` (never fresh ULID), exactly-once via U1's atomic `claim_and_enqueue_start`.
**How to apply:** during U3 build/review, reject any `build_start_msg` extraction, any sentinel routing tuple in a durable row, any String-typed emit error (use typed ActionError, retryable for transient storage err), and any payload/credential in the tracing span (event_id is non-secret, OK to log). InMemory inbox = contract baseline; Postgres serializability is ROADMAP-M7-gated, never false-green. See [[project_plugin_dependency_resolver_d6]].
