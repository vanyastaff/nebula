# nebula-eventbus
Generic typed pub/sub event bus — transport infrastructure only, no domain event types.

## Invariants
- Transport-only: no domain event types defined here. Domain crates own their event types (e.g. `EventBus<ExecutionEvent>` lives in nebula-telemetry).
- Best-effort delivery: producers never block; no delivery guarantee; no global ordering.
- In-memory only (Phase 2). No persistence. Events are lost on restart or buffer overflow.

## Key Decisions
- Backed by `tokio::sync::broadcast` — bounded, Lagged semantics, zero-copy clone.
- `BackPressurePolicy` controls what happens when the buffer is full (drop oldest vs drop newest vs block). Block policy uses exponential backoff (50µs base, 1ms cap) instead of fixed polling.
- `EventBusRegistry` provides multi-bus isolation by key (e.g., per-tenant buses). Uses `parking_lot::RwLock` (no poison recovery needed).
- `FilteredSubscriber` + `EventFilter` for predicate-based selective subscription.
- `SubscriberStream` / `FilteredStream` via `into_stream()` — `futures_core::Stream` adapters for use with `StreamExt` combinators. Lagged events auto-skipped (same semantics as `recv()`).
- `SubscriptionScope` + `ScopedEvent` for targeted subscriptions (e.g., listen only for a specific workflow's events).

## API
- `emit()` — non-blocking emit (always). Formerly `send()`.
- `emit_awaited()` — async emit respecting `Block` policy. Formerly `send_async()`.
- Old aliases (`send`, `send_async`, `emit_async`, `EventSubscriber`, `Subscriber::close()`, `FilteredSubscriber::close()`) removed.

## Traps
- Slow subscribers don't block producers — they lag and auto-skip to the latest event. Check `Subscriber::lagged_count()` after receive to detect missed events.
- Dropping `Subscriber` auto-decrements count — no explicit close needed.

## Relations
- No nebula deps. Used by nebula-telemetry (wraps it for `ExecutionEvent`), nebula-resource.

<!-- reviewed: 2026-03-19 -->
