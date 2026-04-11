# nebula-eventbus
Generic typed pub/sub event bus — transport infrastructure only, no domain event types.

## Invariants
- Transport-only: no domain event types defined here. Domain crates own their event types.
- Best-effort delivery: producers never block; no delivery guarantee; no global ordering.
- In-memory only (Phase 2). No persistence. Events are lost on restart or buffer overflow.
- No nebula deps. External deps: parking_lot, futures-core, tokio-stream.

## Key Decisions
- Backed by `tokio::sync::broadcast` — bounded, Lagged semantics, zero-copy clone.
- `BackPressurePolicy` controls buffer-full behavior (DropOldest / DropNewest / Block).
- `Block` policy uses exponential backoff (50µs base, 1ms cap) instead of fixed 1ms polling.
- `EventBusRegistry` uses `parking_lot::RwLock` (no poisoning, no recovery code).
- `FilteredSubscriber` + `EventFilter` for predicate-based selective subscription.
- `SubscriptionScope` + `ScopedEvent` for targeted subscriptions.
- `Subscriber::into_stream()` returns `SubscriberStream` implementing `futures_core::Stream`.
- `FilteredSubscriber::into_stream()` returns `FilteredStream` implementing `futures_core::Stream`.
- Single emit API: `emit()` (non-blocking) and `emit_awaited()` (Block policy).
- No aliases: `send()`, `send_async()`, `emit_async()`, `EventSubscriber` were removed.

## Traps
- Slow subscribers don't block producers — they lag and auto-skip. Check `lagged_count()`.
- Dropping `Subscriber` auto-decrements count — no explicit close needed.
- `emit()` with `Block` policy behaves as `DropOldest` — use `emit_awaited()` for blocking.
- `EventSubscriber<E>` type alias was removed — use `Subscriber<E>` directly.
- `send()` / `send_async()` / `emit_async()` were removed — use `emit()` / `emit_awaited()`.

## Relations
- No nebula deps. Used by nebula-telemetry, nebula-resource, nebula-credential.
