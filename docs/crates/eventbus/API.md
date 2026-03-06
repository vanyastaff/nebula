# API

## Public Surface

### Stable APIs (Target)

- `EventBus<E>` — generic broadcast event bus
- `EventBus::new(buffer_size)`, `EventBus::with_policy(buffer_size, policy)`
- `EventBus::send(&self, event) -> PublishOutcome`
- `EventBus::emit(&self, event) -> PublishOutcome`
- `EventBus::send_async(&self, event).await -> PublishOutcome`
- `EventBus::emit_async(&self, event).await -> PublishOutcome`
- `EventBus::subscribe(&self) -> EventSubscriber<E>`
- `EventBus::subscribe_filtered(&self, filter) -> FilteredSubscriber<E>`
- `EventBus::subscribe_scoped(&self, scope) -> FilteredSubscriber<E>` (when `E: ScopedEvent`)
- `EventBus::stats(&self) -> EventBusStats`
- `BackPressurePolicy` — DropOldest, DropNewest, Block { timeout }
- `EventSubscriber<E>::recv(&mut self) -> impl Future<Output = Option<E>>`
- `EventSubscriber<E>::try_recv(&mut self) -> Option<E>`
- `EventSubscriber<E>::lagged_count(&self) -> u64`, `EventSubscriber<E>::is_closed(&self) -> bool`, `EventSubscriber<E>::close(self)`
- `PublishOutcome` — Sent, DroppedNoSubscribers, DroppedByPolicy, DroppedTimeout
- `SubscriptionScope`, `ScopedEvent`, `EventFilter<E>`, `FilteredSubscriber<E>`

### Experimental APIs (Phase 2)

- `EventBus::subscribe_scoped(scope, filter) -> ScopedSubscription`
- `SubscriptionScope` — Workflow(id), Execution(id), Resource(id), Global
- `EventFilter` — EventType, PayloadMatch, Custom predicate

### Hidden/Internal APIs

- Back-pressure policy internals; occupancy tracking for DropNewest

## Usage Patterns

### Producer (Emitter)

```rust
// Domain crate (e.g. nebula-telemetry) constructs bus.
let bus: EventBus<ExecutionEvent> = EventBus::new(1024);

// Emit synchronously — never blocks.
let outcome = bus.emit(ExecutionEvent::Started {
    execution_id: "e1".into(),
    workflow_id: "w1".into(),
});
assert!(outcome.is_sent());

// Or with Block policy, emit_async.
let _ = bus.emit_async(event).await;
```

### Consumer (Subscriber)

```rust
let mut sub = bus.subscribe();

// Async receive loop
while let Some(event) = sub.recv().await {
    match event {
        ExecutionEvent::NodeCompleted { node_id, duration, .. } => {
            metrics::histogram!("node_duration_seconds").record(duration);
        }
        _ => {}
    }
}
```

## Minimal Example

```rust
use nebula_eventbus::EventBus;

#[derive(Clone)]
struct MyEvent { id: String }

let bus = EventBus::<MyEvent>::new(64);
let mut sub = bus.subscribe();

bus.emit(MyEvent { id: "a".into() });
let event = sub.try_recv().expect("received");
assert_eq!(event.id, "a");
```

## Advanced Example

```rust
// Resource crate: EventBus with BackPressurePolicy.
let bus = EventBus::with_policy(
    1024,
    BackPressurePolicy::Block {
        timeout: Duration::from_millis(100),
    },
);

// Emit with back-pressure.
let _ = bus.emit_async(ResourceEvent::Acquired {
    resource_id: "db".into(),
    wait_duration: Duration::ZERO,
}).await;

// Subscriber with stats.
let stats = bus.stats();
assert_eq!(stats.sent_count, 1);
assert_eq!(stats.subscriber_count, 1);
```

## Error Semantics

- Send APIs do not return `Result`; they return [`PublishOutcome`] for explicit drop semantics.

**Subscriber side:**
- `recv()` returns `None` when sender dropped or channel closed
- `RecvError::Lagged` — subscriber fell behind; skip to latest (handled internally in EventSubscriber)

## Compatibility Rules

- **Major version bump:** Removal of event variants; change to BackPressurePolicy semantics; breaking subscribe API
- **Deprecation policy:** Minimum 2 minor releases; deprecation attributes with replacement guidance
