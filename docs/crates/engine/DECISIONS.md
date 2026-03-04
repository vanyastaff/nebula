# Decisions

## D-001: Engine Depends on Runtime, Not Vice Versa

**Status:** Adopt

**Context:** Need clear dependency direction between orchestration and execution.

**Decision:** Engine calls runtime to execute a node; runtime does not call engine for scheduling.

**Alternatives considered:** Runtime holding reference to engine for callbacks — rejected to avoid cycle and to keep runtime reusable (tests, CLI).

**Trade-offs:** Engine must pass full context to runtime; runtime is stateless with respect to engine.

**Consequences:** All scheduling and lifecycle logic in engine; runtime is executor only.

**Migration impact:** None.

**Validation plan:** Integration tests: engine → runtime → action; no reverse dependency.

---

## D-002: ExecutionContext Built by Engine

**Status:** Adopt

**Context:** Runtime and actions need execution_id, workflow_id, node outputs, options.

**Decision:** Engine builds execution context; runtime receives it and passes to actions.

**Alternatives considered:** Runtime building context from scratch — rejected to avoid duplicating engine state.

**Trade-offs:** Context shape is part of engine contract; runtime and action depend on it.

**Consequences:** Breaking context shape = major; document in API.md and MIGRATION.

**Migration impact:** Any context field change requires coordination with runtime/action.

**Validation plan:** Contract tests for context fields used by runtime and actions.

---

## D-003: EventBus for Observability, Not Control Flow

**Status:** Adopt

**Context:** Need to emit events without blocking execution or failing workflows on subscriber failure.

**Decision:** Events are for logging, metrics, API subscription; execution flow does not depend on delivery. Fire-and-forget; best-effort.

**Alternatives considered:** Synchronous delivery — rejected; would couple engine to subscriber latency and failures.

**Trade-offs:** Subscribers may miss events if they lag; no back-pressure from engine to subscribers.

**Consequences:** EventBus send is non-blocking; engine never waits on emit.

**Migration impact:** None.

**Validation plan:** Tests: emit does not block; slow subscriber does not slow execute_workflow.
