# Migration

## Versioning Policy

- **Compatibility promise:** Additive changes only in minor releases. Event schema changes are owned by domain crates.
- **Deprecation window:** Minimum 2 minor releases before removal.

## Breaking Changes

### Telemetry Wrapper Alignment

| Change | Old behavior | New behavior | Migration steps |
|--------|--------------|--------------|-----------------|
| Emit return type | `emit(...) -> ()` | `emit(...) -> PublishOutcome` | Keep call sites or explicitly handle outcome |
| Scoped subscriptions | N/A | `subscribe_scoped(...)` and `subscribe_filtered(...)` | Implement `ScopedEvent` for domain events |
| EventSubscriber diagnostics | Minimal | `lagged_count`, `is_closed`, `close` | Optionally update subscriber loops/observability |

### Resource Wrapper Alignment

| Change | Old behavior | New behavior | Migration steps |
|--------|--------------|--------------|-----------------|
| Emit return type | `emit(...) -> ()` | `emit(...) -> PublishOutcome` | Keep call sites or branch on outcome |
| Scoped subscriptions | N/A | `subscribe_scoped(...)` | Use `SubscriptionScope::resource/workflow/execution` |
| Metrics bridge | Per-crate custom counters | `TelemetryAdapter::record_eventbus_stats` | Record snapshots under `nebula_eventbus_*` |

## Rollout Plan

1. **Preparation:** Create nebula-eventbus crate; implement EventBus<E>, BackPressurePolicy, EventSubscriber.
2. **Dual-run:** Add eventbus as dependency to telemetry and resource; keep internal implementations; feature-flag or cfg to switch.
3. **Cutover:** Replace internal EventBus with eventbus in telemetry; replace in resource; remove duplicate code.
4. **Cleanup:** Remove feature flags; update docs; deprecate old paths if any remain.

## Adding New Event Types (T026)

Use this path when adding a new domain event bus (for example `ProjectEvent` in a future crate):

1. Define domain event enum in the domain crate (`nebula-<domain>`), not in `nebula-eventbus`.
2. Add `impl ScopedEvent for <DomainEvent>` if workflow/execution/resource scoping is needed.
3. Wrap `nebula_eventbus::EventBus<DomainEvent>` in a domain-facing `EventBus` API if ergonomic aliases are useful.
4. Expose `emit`, `emit_async`, `subscribe`, and optional `subscribe_scoped`/`subscribe_filtered` from wrapper.
5. Add integration tests that validate:
   - publish outcomes (`Sent` / drops),
   - scoped filtering behavior,
   - subscriber lag handling under pressure.
6. Add metric snapshot recording via `nebula_metrics::TelemetryAdapter::record_eventbus_stats`.
7. Document schema versioning for the new event enum (additive-first policy).

### Minimal Template

```rust
use nebula_eventbus::{EventBus, ScopedEvent, SubscriptionScope};

#[derive(Debug, Clone)]
pub enum ProjectEvent {
	Started { project_id: String, execution_id: String },
}

impl ScopedEvent for ProjectEvent {
	fn execution_id(&self) -> Option<&str> {
		match self {
			Self::Started { execution_id, .. } => Some(execution_id),
		}
	}
}

let bus = EventBus::<ProjectEvent>::new(256);
let mut sub = bus.subscribe_scoped(SubscriptionScope::execution("exec-1"));
let _ = bus.emit(ProjectEvent::Started {
	project_id: "proj-1".into(),
	execution_id: "exec-1".into(),
});
assert!(sub.try_recv().is_some());
```

## Rollback Plan

- **Trigger conditions:** Test failures; performance regression; integration break.
- **Rollback steps:** Revert to internal EventBus in telemetry/resource; remove eventbus dependency.
- **Data/state reconciliation:** N/A; events are ephemeral; no persistent state in eventbus.

## Validation Checklist

- [ ] API compatibility: telemetry EventBus → eventbus EventBus
- [ ] API compatibility: resource EventBus → eventbus EventBus
- [ ] Integration: engine tests pass with telemetry using eventbus
- [ ] Integration: resource tests pass with eventbus
- [ ] Performance: emit latency unchanged or improved
- [ ] No new clippy warnings or test failures
