# Decisions

## D001: Runtime as separate crate from engine

**Status:** Adopt

**Context:** Engine schedules workflow nodes; something must execute actions. Could be in-engine or separate.

**Decision:** `nebula-runtime` is a separate crate. Engine depends on runtime; runtime receives registry, sandbox, telemetry via constructor.

**Alternatives considered:**
- In-engine execution — would bloat engine; harder to test in isolation
- Monolith — rejected; Nebula is multi-crate

**Trade-offs:** Extra crate; clear boundary. Engine focuses on DAG/scheduling; runtime on action execution.

**Consequences:** Engine and runtime have distinct responsibilities; integration tests in engine.

**Migration impact:** None; current state.

**Validation plan:** Engine integration tests; resource_integration tests.

---

## D002: SandboxRunner trait from ports

**Status:** Adopt

**Context:** Actions may need isolation (untrusted code). Sandbox provides capability checks, resource limits.

**Decision:** Runtime defines SandboxRunner and provides InProcessSandbox. Sandbox and task queue (TaskQueue, MemoryQueue) live in nebula-runtime.

**Alternatives considered:**
- Runtime owns sandbox impl — couples runtime to specific isolation
- No sandbox — unacceptable for untrusted plugins

**Trade-offs:** Pluggable; TODO: isolation level routing not yet implemented.

**Consequences:** All actions currently run directly; sandbox used in tests only.

**Migration impact:** Phase 2 adds isolation routing.

**Validation plan:** Sandbox integration tests.

---

## D003: DataPassingPolicy with Reject/SpillToBlob

**Status:** Adopt

**Context:** Large node outputs can cause OOM. Need bounded data flow.

**Decision:** DataPassingPolicy with max_node_output_bytes, max_total_execution_bytes, LargeDataStrategy (Reject, SpillToBlob). Reject implemented; SpillToBlob TODO.

**Alternatives considered:**
- No limits — OOM risk
- Always spill — adds latency; Reject is simpler default

**Trade-offs:** Reject works; SpillToBlob deferred.

**Consequences:** Large outputs fail with DataLimitExceeded until SpillToBlob implemented.

**Migration impact:** None.

**Validation plan:** data_limit_enforcement test.

---

## D004: Telemetry events from runtime

**Status:** Adopt

**Context:** Observability for action execution. Engine emits workflow-level events; runtime emits node-level.

**Decision:** Runtime emits NodeStarted, NodeCompleted, NodeFailed via EventBus; records actions_executed_total, actions_failed_total, action_duration_seconds via MetricsRegistry.

**Alternatives considered:**
- Engine emits all — engine would need to know execution details
- No events — observability gap

**Trade-offs:** Runtime owns node-level telemetry; consistent with engine owning workflow-level.

**Consequences:** Runtime depends on telemetry; fire-and-forget emit.

**Migration impact:** None.

**Validation plan:** telemetry_events_emitted test.

---

## D005: ActionRegistry uses InternalHandler from plugin

**Status:** Adopt

**Context:** Handlers come from plugins. Need a common trait for registry.

**Decision:** ActionRegistry stores `Arc<dyn InternalHandler>`. Plugin crate defines InternalHandler; actions implement it. Context passed to `execute` is currently **NodeContext** (deprecated in nebula-action); target is **ActionContext** / `&impl Context` (see INTERACTIONS, CONSTITUTION P-001).

**Alternatives considered:**
- ProcessAction/StatefulAction adapters — commented out; may return when runtime aligns with StatelessAction
- Typed registry per action — complex; dynamic key lookup needed

**Trade-offs:** InternalHandler is the single handler type; metadata().key for registration.

**Consequences:** All handlers must implement InternalHandler. Context type migration (NodeContext → ActionContext) is a future breaking change.

**Migration impact:** None.

**Validation plan:** Registry tests; engine with registered handlers.
