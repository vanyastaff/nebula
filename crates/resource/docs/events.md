# Events

Lifecycle event system for observability and diagnostics.

---

## Overview

The [`Manager`] emits [`ResourceEvent`]s on every significant lifecycle transition.
Events are broadcast via a `tokio::sync::broadcast` channel (256-event buffer).

Subscribe with [`Manager::subscribe_events()`]:

```rust,ignore
let mut rx = manager.subscribe_events();
tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        tracing::info!(?event, "resource lifecycle event");
    }
});
```

---

## Event Catalog

| Variant | Emitted when | Key fields |
|---------|-------------|------------|
| `Registered` | A resource is registered | `key` |
| `Removed` | A resource is removed | `key` |
| `AcquireSuccess` | A handle is acquired | `key`, `duration` |
| `AcquireFailed` | Acquire returns an error | `key`, `error: String` |
| `Released` | A handle is dropped | `key`, `held: Duration`, `tainted: bool` |
| `HealthChanged` | Health status transitions | `key`, `healthy: bool` |
| `ConfigReloaded` | Config is hot-reloaded | `key` |

All variants carry a `key: ResourceKey` accessible via `event.key()`.

The enum is `#[non_exhaustive]` — new variants may be added in minor releases.

---

## Usage Patterns

### Metrics collection

```rust,ignore
while let Ok(event) = rx.recv().await {
    match &event {
        ResourceEvent::AcquireSuccess { duration, .. } => {
            histogram.record(duration.as_millis() as f64);
        }
        ResourceEvent::AcquireFailed { error, .. } => {
            counter.increment(1);
            tracing::warn!(%error, "acquire failed");
        }
        _ => {}
    }
}
```

### Slow consumers

The broadcast channel has a 256-event buffer. Slow consumers will receive
`RecvError::Lagged(n)` — events were dropped. Handle this gracefully:

```rust,ignore
match rx.recv().await {
    Ok(event) => handle(event),
    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
        tracing::warn!(dropped = n, "event consumer lagged");
    }
    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
}
```

---

## Differences from v1

- **No `EventBus`** — events come directly from `Manager::subscribe_events()`.
- **No `HookRegistry`** — pre/post hooks were removed. Use events for observation.
- **No filtered subscriptions** — filter in your consumer logic.
- **No `BackPressurePolicy`** — the broadcast channel drops oldest on overflow.
