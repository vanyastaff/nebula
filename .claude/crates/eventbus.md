# nebula-eventbus
Generic typed pub/sub event bus — transport infrastructure only, no domain event types.

## Invariants
- Transport-only: no domain event types defined here. Domain crates own their event types (e.g. `EventBus<ExecutionEvent>` lives in nebula-telemetry).
- Best-effort delivery: producers never block; no delivery guarantee; no global ordering.
- In-memory only (Phase 2). No persistence. Events are lost on restart or buffer overflow.

## Key Decisions
- Backed by `tokio::sync::broadcast` — bounded, Lagged semantics, zero-copy clone.
- `BackPressurePolicy` controls what happens when the buffer is full (drop oldest vs drop newest vs block).
- `EventBusRegistry` provides multi-bus isolation by key (e.g., per-tenant buses).
- `FilteredSubscriber` + `EventFilter` for predicate-based selective subscription.
- `SubscriptionScope` + `ScopedEvent` for targeted subscriptions (e.g., listen only for a specific workflow's events).

## Traps
- Slow subscribers don't block producers — they lag and auto-skip to the latest event. Check `Subscriber::lagged_count()` after receive to detect missed events.
- Dropping `Subscriber` auto-decrements count — no explicit close needed.
- `EventSubscriber<E>` is just a type alias for `Subscriber<E>`.

## Relations
- No nebula deps. Used by nebula-telemetry (wraps it for `ExecutionEvent`), nebula-resource.
