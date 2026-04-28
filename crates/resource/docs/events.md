# Events

Lifecycle event system for observability and diagnostics.

---

## Overview

The `Manager` emits `ResourceEvent`s on every significant lifecycle
transition. Events are broadcast via a `tokio::sync::broadcast` channel —
see `Manager::subscribe_events()` for the receiver type.

Subscribe with `Manager::subscribe_events()`:

```rust,ignore
let mut rx = manager.subscribe_events();
tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        tracing::info!(?event, "resource lifecycle event");
    }
});
```

The channel buffer is fixed at construction; slow consumers receive
`tokio::sync::broadcast::error::RecvError::Lagged` when they fall
behind — see [Slow consumers](#slow-consumers) below.

---

## Event Catalog

Twelve `#[non_exhaustive]` variants; new variants may be added in minor
releases without a major bump.

### Per-resource variants (10)

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

### Aggregate (rotation cycle) variants (2)

Emitted by `Manager::on_credential_refreshed` / `_revoked` after every
per-resource dispatch future has completed (see Tech Spec §6.2).

| Variant | Emitted when | Key fields |
|---------|-------------|------------|
| `CredentialRefreshed` | One refresh-cycle fan-out completed | `credential_id: CredentialId`, `resources_affected: usize`, `outcome: RotationOutcome` |
| `CredentialRevoked` | One revoke-cycle fan-out completed | `credential_id: CredentialId`, `resources_affected: usize`, `outcome: RotationOutcome` |

`outcome.total()` always equals `resources_affected`. Per-resource
revocation failures are also signalled inline as `HealthChanged { healthy:
false }` (security amendment B-2 from cascade Phase 6 CP2 review), so
subscribers that miss the aggregate event still see per-resource failure
events.

`RotationOutcome` is `nebula_resource::RotationOutcome` (re-export of
`crate::error::RotationOutcome`) — see `RotationOutcome` for the `ok` /
`failed` / `timed_out` count breakdown.

---

## Reading the resource key

Per-resource variants carry a `key: ResourceKey`; the aggregate rotation
variants do not (they span multiple resources). Use the convenience
accessor:

```rust,ignore
fn key(&self) -> Option<&ResourceKey>
```

Returns `Some(&key)` for the 10 per-resource variants; returns `None` for
`CredentialRefreshed` and `CredentialRevoked` — for those, read the
`credential_id` field directly to identify the rotation.

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
        ResourceEvent::RetryAttempt { attempt, backoff, .. } => {
            tracing::info!(attempt, ?backoff, "retrying acquire");
        }
        _ => {}
    }
}
```

### Slow consumers

Slow consumers receive `tokio::sync::broadcast::error::RecvError::Lagged(n)`
when the channel overruns — *n* events were dropped. Handle it:

```rust,ignore
match rx.recv().await {
    Ok(event) => handle(event),
    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
        tracing::warn!(dropped = n, "event consumer lagged");
    }
    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
}
```

The broadcast channel drops oldest events on overflow — there is no
back-pressure or retry mechanism inside the Manager.

### Auditing rotation cycles

```rust,ignore
match &event {
    ResourceEvent::CredentialRefreshed { credential_id, outcome, .. }
    | ResourceEvent::CredentialRevoked { credential_id, outcome, .. } => {
        rotation_counter.increment(outcome.total() as u64);
        if outcome.failed > 0 || outcome.timed_out > 0 {
            tracing::warn!(
                %credential_id,
                ok = outcome.ok,
                failed = outcome.failed,
                timed_out = outcome.timed_out,
                "rotation cycle had partial failures",
            );
        }
    }
    _ => {}
}
```

---

## Differences from v1

- **No `EventBus`** — events come directly from `Manager::subscribe_events()`;
  no separate bus crate, no subscriber registration.
- **No `HookRegistry`** — pre/post hooks were removed. Use events for observation.
- **No filtered subscriptions** — filter in your consumer logic.
- **No `BackPressurePolicy`** — the broadcast channel drops oldest on overflow.
