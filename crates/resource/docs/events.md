# Events

Lifecycle event system for observability and diagnostics.

---

## Overview

The `Manager` emits `ResourceEvent`s on every significant lifecycle
transition. Events are published through a `nebula_eventbus::EventBus<ResourceEvent>`
— see `Manager::subscribe_events()` for the subscriber type.

Subscribe with `Manager::subscribe_events()`:

```rust,ignore
use nebula_resource::Subscriber;

let mut rx: Subscriber<ResourceEvent> = manager.subscribe_events();
tokio::spawn(async move {
    while let Some(event) = rx.recv().await {
        tracing::info!(?event, "resource lifecycle event");
    }
});
```

Lag handling is automatic: when a subscriber falls behind, the
`EventBus` skips to the latest event internally — subscribers never
need to handle a lag error explicitly.

---

## Event Catalog

Fourteen `#[non_exhaustive]` variants; new variants may be added in minor
releases without a major bump. Every variant carries a `ResourceKey` — this
crate reports strictly per-resource (the engine owns cross-resource rotation
aggregation, so there is no aggregate `CredentialRefreshed` /
`CredentialRevoked` event and no `CredentialId` in any payload).

### Generic lifecycle variants (10)

| Variant | Emitted when | Key fields |
|---------|-------------|------------|
| `Registered` | A resource is registered | `key` |
| `Removed` | A resource is removed | `key` |
| `AcquireSuccess` | A handle is acquired | `key`, `duration: Duration` |
| `AcquireFailed` | Acquire returns an error | `key`, `error: String` |
| `Released` | A handle is dropped | `key`, `held: Duration`, `tainted: bool` |
| `HealthChanged` | Health status transitions | `key`, `healthy: bool` |
| `ConfigReloaded` | Config is hot-reloaded | `key` |
| `RetryAttempt` | A retry is about to sleep after a transient acquire failure | `key`, `attempt: u32`, `backoff: Duration`, `error: String` |
| `BackpressureDetected` | Pool backpressure was detected (semaphore full) | `key` |
| `RecoveryGateChanged` | A recovery gate transitioned | `key`, `state: String` |

### Slot-rotation variants (4)

Emitted per `(resource, slot)` by the engine-owned rotation fan-out through
the `Manager::{refresh_slot, revoke_slot}` port, after the
engine has swapped the rotated guard into the slot and invoked the
resource's `on_credential_refresh` / `on_credential_revoke` hook. The
`error` string is already redacted — it never carries credential material.

| Variant | Emitted when | Key fields |
|---------|-------------|------------|
| `SlotRefreshed` | A `#[credential]` slot was refreshed (hook returned `Ok`) | `key`, `slot: String` |
| `SlotRevoked` | A slot's credential was revoked (hook returned `Ok`) | `key`, `slot: String` |
| `SlotRefreshFailed` | The per-resource refresh hook failed or timed out | `key`, `slot: String`, `error: String` (redacted) |
| `SlotRevokeFailed` | The per-resource revoke hook failed | `key`, `slot: String`, `error: String` (redacted) |

Per-resource revocation failures are also signalled inline as
`HealthChanged { healthy: false }`, so subscribers that filter slot events
still see the failure surface.

---

## Reading the resource key

Every variant carries a `key: ResourceKey` — including the four
slot-rotation variants. The convenience accessor therefore always returns
`Some`:

```rust,ignore
fn key(&self) -> Option<&ResourceKey>
```

(It is `Option`-typed for forward compatibility with any future
non-resource-scoped variant, but every variant shipped today returns
`Some(&key)`.)

---

## Usage Patterns

### Metrics collection

```rust,ignore
while let Some(event) = rx.recv().await {
    match &event {
        ResourceEvent::AcquireSuccess { duration, .. } => {
            histogram.record(duration.as_millis() as f64);
        }
        ResourceEvent::AcquireFailed { error, .. } => {
            counter.increment(1);
            tracing::warn!(%error, "acquire failed");
        }
        ResourceEvent::RetryAttempt { attempt, backoff, .. } => {
            tracing::info!(attempt, ?backoff, "retrying acquire");
        }
        _ => {}
    }
}
```

### Slow consumers

The `EventBus` handles buffer overflow internally: when a subscriber
falls behind, the bus skips to the latest event automatically.
Subscribers do **not** need to handle `RecvError::Lagged` explicitly —
the lag recovery is transparent.

```rust,ignore
// Simple consumption loop — no lag error handling needed.
while let Some(event) = rx.recv().await {
    handle(event);
}
```

There is no back-pressure or retry mechanism inside the Manager; the
EventBus drops oldest events on overflow and advances slow subscribers
to the latest position.

### Auditing slot rotation

```rust,ignore
match &event {
    ResourceEvent::SlotRefreshed { key, slot }
    | ResourceEvent::SlotRevoked { key, slot } => {
        rotation_counter.increment(1);
        tracing::info!(%key, %slot, "credential slot rotated");
    }
    ResourceEvent::SlotRefreshFailed { key, slot, error }
    | ResourceEvent::SlotRevokeFailed { key, slot, error } => {
        // `error` is already redacted — safe to log verbatim.
        tracing::warn!(%key, %slot, %error, "slot rotation hook failed");
    }
    _ => {}
}
```

For a fleet-wide rotation roll-up, aggregate these per-resource events on
the consumer side, or read the engine's rotation telemetry — this crate
deliberately emits one event per `(resource, slot)` and no cycle-level
aggregate.

---

## Differences from v1

- **Uses `nebula_eventbus::EventBus`** — events are published through the
  shared EventBus crate; `Manager::subscribe_events()` returns a
  `Subscriber<ResourceEvent>` (re-exported from `nebula_resource`).
- **No `HookRegistry`** — pre/post hooks were removed. Use events for observation.
- **No filtered subscriptions** — filter in your consumer logic.
- **Automatic lag handling** — the EventBus drops oldest on overflow and
  skips slow subscribers to the latest event; no explicit `RecvError::Lagged`
  handling required.
