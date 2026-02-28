# Proposals

## P001: Event Metadata Trait for Scoped Subscriptions

**Type:** Non-breaking

**Motivation:** Phase 2 scoped subscriptions require extracting workflow_id, execution_id, resource_id from events. Domain crates own event types; eventbus needs a generic way to get scope.

**Proposal:** Define `ScopedEvent` trait:

```rust
pub trait ScopedEvent {
    fn workflow_id(&self) -> Option<&str>;
    fn execution_id(&self) -> Option<&str>;
    fn resource_id(&self) -> Option<&str>;
}
```

Events implementing this trait can be used with `subscribe_scoped`. Events without the trait use Global scope only.

**Expected benefits:** No schema coupling; domain crates opt-in; flexible.

**Costs:** Trait implementation per event type; some boilerplate.

**Risks:** Trait may need extension for new scope dimensions.

**Compatibility impact:** Additive; existing events work without trait.

**Status:** Draft

---

## P002: Event Schema Registry

**Type:** Non-breaking

**Motivation:** Multiple event types (ExecutionEvent, ResourceEvent, WorkflowEvent) may need a unified taxonomy for filtering and routing. Currently each domain defines its own enum.

**Proposal:** Optional `EventKind` enum or registry for event type names. Used by EventFilter::EventType("execution"), etc. Domain crates register their events.

**Expected benefits:** Consistent filtering; cross-domain tooling.

**Costs:** Registry maintenance; possible global state.

**Risks:** Over-engineering for single-node; defer until multi-domain filtering needed.

**Compatibility impact:** Additive.

**Status:** Defer

---

## P003: At-Least-Once Delivery Option

**Type:** Breaking

**Motivation:** Some consumers (audit log, compliance) may need guaranteed delivery. Current fire-and-forget drops events when no subscribers or when lagging.

**Proposal:** Optional "persistent" mode: events written to a buffer (e.g. disk, Redis) before emit. Subscribers read from buffer. Adds latency and I/O.

**Expected benefits:** Audit trail; replay capability.

**Costs:** Significant complexity; persistence layer; not fire-and-forget.

**Risks:** Changes event semantics; may belong in separate crate (nebula-eventlog).

**Compatibility impact:** Breaking if default behavior changes.

**Status:** Rejected for eventbus scope; consider separate crate.
