# Migration

## Versioning Policy

- **Compatibility promise:** Additive changes only in minor releases. Event schema changes are owned by domain crates.
- **Deprecation window:** Minimum 2 minor releases before removal.

## Breaking Changes

### Extraction from nebula-telemetry

| Change | Old behavior | New behavior | Migration steps |
|--------|--------------|--------------|-----------------|
| EventBus location | `nebula_telemetry::event::EventBus` | `nebula_eventbus::EventBus` | Update imports; use `EventBus::new()` from eventbus |
| EventSubscriber | `nebula_telemetry::event::EventSubscriber` | `nebula_eventbus::EventSubscriber` | Update imports |
| ExecutionEvent | Stays in telemetry | Stays in telemetry | No change |
| total_emitted() | EventBus method | EventBusStats.emitted | Use `bus.stats().emitted` |

### Extraction from nebula-resource

| Change | Old behavior | New behavior | Migration steps |
|--------|--------------|--------------|-----------------|
| EventBus location | `nebula_resource::events::EventBus` | `nebula_eventbus::EventBus` | Update imports |
| BackPressurePolicy | `nebula_resource::events::BackPressurePolicy` | `nebula_eventbus::BackPressurePolicy` | Update imports |
| ResourceEvent | Stays in resource | Stays in resource | No change |
| EventBusStats | `nebula_resource::events::EventBusStats` | `nebula_eventbus::EventBusStats` | Update imports |
| subscribe() return type | `broadcast::Receiver<ResourceEvent>` | `EventSubscriber<ResourceEvent>` | Use `sub.recv().await` instead of `rx.recv().await`; EventSubscriber handles Lagged |

## Rollout Plan

1. **Preparation:** Create nebula-eventbus crate; implement EventBus<E>, BackPressurePolicy, EventSubscriber.
2. **Dual-run:** Add eventbus as dependency to telemetry and resource; keep internal implementations; feature-flag or cfg to switch.
3. **Cutover:** Replace internal EventBus with eventbus in telemetry; replace in resource; remove duplicate code.
4. **Cleanup:** Remove feature flags; update docs; deprecate old paths if any remain.

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
