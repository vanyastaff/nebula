# Decisions

## D001: Generic EventBus<E> per event type

**Status:** Adopt

**Context:** Need to support ExecutionEvent, ResourceEvent, and future event types without duplicating bus logic. Type erasure (dyn Event) adds runtime cost and complexity.

**Decision:** Implement `EventBus<E: Clone + Send>`. Each domain crate constructs its own `EventBus<ExecutionEvent>`, `EventBus<ResourceEvent>`, etc. Event schemas stay in domain crates; eventbus is transport-only.

**Alternatives considered:**
- Single EventBus with enum of all events — couples all domains; large enum
- dyn Event trait object — runtime dispatch; Clone + Send bounds awkward

**Trade-offs:** Multiple bus instances; no cross-domain subscription. Simplicity and zero-cost abstraction win.

**Consequences:** Domain crates depend on eventbus for transport; own event types.

**Migration impact:** Telemetry and resource replace internal EventBus with eventbus crate.

**Validation plan:** Extract telemetry EventBus to use eventbus; run existing tests.

---

## D002: Fire-and-forget emit

**Status:** Adopt

**Context:** Emitters (engine, runtime, resource) must never block. Events are projections, not source of truth.

**Decision:** `emit()` returns `()`; never blocks. If no subscribers, event dropped silently. If buffer full, policy determines behavior (DropOldest/DropNewest/Block).

**Alternatives considered:**
- Emit returns Result — would require callers to handle; adds no value for projections
- Block until delivered — unacceptable for execution hot path

**Trade-offs:** Best-effort delivery; no delivery guarantee. Acceptable for observability events.

**Consequences:** Subscribers may miss events when lagging; no at-least-once.

**Migration impact:** None; matches current behavior.

**Validation plan:** Existing telemetry/resource tests; back-pressure tests.

---

## D003: tokio::sync::broadcast as transport

**Status:** Adopt

**Context:** Need bounded, multi-consumer, async channel. Broadcast provides fan-out, Lagged semantics, Clone requirement.

**Decision:** Use `tokio::sync::broadcast` as the underlying transport. Event type must implement `Clone`.

**Alternatives considered:**
- mpsc multicast — custom implementation; more complex
- crossbeam broadcast — sync; would need wrapper for async recv

**Trade-offs:** Clone per subscriber; bounded buffer. Standard, well-tested.

**Consequences:** Events must be Clone; large payloads may have cost.

**Migration impact:** None; already used in telemetry and resource.

**Validation plan:** Benchmark emit latency; subscriber throughput.

---

## D004: BackPressurePolicy from resource crate

**Status:** Adopt

**Context:** Resource crate has DropOldest, DropNewest, Block. Telemetry uses default (DropOldest) only.

**Decision:** Include BackPressurePolicy in eventbus; support all three. Default: DropOldest. Block requires `emit_async`.

**Alternatives considered:**
- DropOldest only — simpler; resource needs Block for critical events
- Custom back-pressure — reinventing; use proven policies

**Trade-offs:** More API surface; flexibility for different domains.

**Consequences:** emit_async needed for Block policy.

**Migration impact:** Resource migrates to eventbus; keeps policy behavior.

**Validation plan:** Resource EventBus tests; policy-specific tests.

---

## D005: Scoped subscriptions in Phase 2

**Status:** Defer

**Context:** Archive specifies `subscribe_scoped(scope, filter)`. Reduces handler logic; enables workflow/execution isolation.

**Decision:** Defer to Phase 2. Phase 1: extract generic EventBus, migrate telemetry/resource. Phase 2: add SubscriptionScope, EventFilter, subscribe_scoped.

**Alternatives considered:**
- Phase 1 scoped — delays extraction; scope design needs more thought
- Never — archive suggests value; defer is pragmatic

**Trade-offs:** Phase 1 simpler; Phase 2 adds scope metadata to events.

**Consequences:** Events may need workflow_id, execution_id for scoping.

**Migration impact:** Phase 2 may require event schema additions.

**Validation plan:** Design scope/filter API; prototype with ExecutionEvent.
