# Recovery

Thundering-herd prevention and health monitoring for resource backends.

---

## Overview

When a backend fails, naive retry logic causes all callers to independently
hammer the dead service — a thundering herd. The recovery layer serializes
recovery attempts through a CAS-based state machine ([`RecoveryGate`]) so
only one caller probes at a time.

---

## RecoveryGate

### State machine

```text
Idle ──try_begin──▶ InProgress ──resolve──▶ Idle
 ▲                       │
 │                  fail_transient
 │                       │
 │                       ▼
 └──(retry_at expired)── Failed ──(max_attempts)──▶ PermanentlyFailed
```

### States

| State | Meaning | What callers see |
|-------|---------|-----------------|
| `Idle` | Backend presumed healthy | Acquire proceeds normally |
| `InProgress` | One caller is probing | Others get `Transient` error immediately |
| `Failed` | Last probe failed | Callers get `Exhausted` with `retry_after` hint |
| `PermanentlyFailed` | Max attempts exceeded | Callers get `Permanent` error |

### API

```rust,ignore
use nebula_resource::{RecoveryGate, RecoveryGateConfig};

let gate = RecoveryGate::new(RecoveryGateConfig {
    max_attempts: 5,       // then PermanentlyFailed
    base_backoff: Duration::from_secs(1), // exponential: 1s, 2s, 4s, 8s...
});

// Only one caller wins the CAS race:
match gate.try_begin() {
    Ok(ticket) => {
        // This caller is the recovery probe.
        match attempt_connection().await {
            Ok(_) => ticket.resolve(),           // → Idle, notify waiters
            Err(e) => ticket.fail_transient(e),  // → Failed with backoff
        }
    }
    Err(TryBeginError::AlreadyInProgress(waiter)) => {
        // Wait for the probe caller to finish.
        let state = waiter.wait().await;
    }
    Err(TryBeginError::RetryLater { retry_at }) => {
        // Backoff not expired yet.
    }
    Err(TryBeginError::PermanentlyFailed { message }) => {
        // Manual intervention required.
    }
}
```

### RecoveryTicket (RAII)

`RecoveryTicket` is `#[must_use]` — you **must** call one of:
- `ticket.resolve()` — backend is healthy again
- `ticket.fail_transient(msg)` — backend still down, schedule retry
- `ticket.fail_permanent(msg)` — give up, require manual reset

If the ticket is dropped without calling any of these, it auto-fails
with a transient error to prevent the gate from being stuck in `InProgress`.

### Backoff schedule

| Attempt | Backoff (`base = 1s`) |
|---------|----------------------|
| 1 | 1s |
| 2 | 2s |
| 3 | 4s |
| 4 | 8s |
| 5 | 16s (capped at 5min) |

### Reset

Call `gate.reset()` to return from `PermanentlyFailed` to `Idle`.

---

## Integration with Manager

When registering a resource, pass an optional `RecoveryGate`:

```rust,ignore
use std::sync::Arc;
use nebula_resource::{
    PoolConfig, RecoveryGate, RecoveryGateConfig, RegisterOptions,
};

let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));

manager.register_pooled_with(
    PostgresResource,
    pg_config,
    PoolConfig::default(),
    RegisterOptions {
        recovery_gate: Some(gate.clone()),
        ..RegisterOptions::default()
    },
)?;
```

For credential-bound resources, also pass `credential_id: Some(...)` in
`RegisterOptions`. Use `Manager::register` (positional) for full control
over scope and topology.

The Manager automatically:
1. **Checks the gate** before each acquire (admission helper in
   `crate::manager::gate`).
2. **Triggers the gate** on transient acquire failures, surfacing the
   `Failed`/`PermanentlyFailed` state to subsequent callers.

Callers don't interact with the gate directly — it works transparently to
prevent thundering herd on the failing backend.

---

## RecoveryGroupRegistry

Groups multiple resources behind a shared recovery gate (e.g., all databases
on the same server). Resources in the same group share gate state, so one
failure blocks all resources in the group.

```rust,ignore
let groups = manager.recovery_groups();
let gate = groups.get_or_create(
    RecoveryGroupKey::new("db-server-1"),
    RecoveryGateConfig::default(),
);
// Pass `gate` (Arc<RecoveryGate>) to RegisterOptions::recovery_gate
// for every resource on the same backend.
```

---

## WatchdogHandle

Opt-in background health probe. Spawns a Tokio task that runs a
user-supplied async `check_fn` on a fixed interval. After
`failure_threshold` consecutive failures it calls
`on_health_change(false)`; after `recovery_threshold` consecutive
successes it calls `on_health_change(true)`.

```rust,ignore
use std::time::Duration;
use nebula_resource::{WatchdogConfig, WatchdogHandle};
use tokio_util::sync::CancellationToken;

let parent_cancel = CancellationToken::new();

let handle = WatchdogHandle::start(
    WatchdogConfig {
        interval: Duration::from_secs(30),
        probe_timeout: Duration::from_secs(5),
        failure_threshold: 3,    // 3 consecutive failures → unhealthy
        recovery_threshold: 1,   // 1 success → healthy again
    },
    || async {
        // Your async probe — return Result<(), nebula_resource::Error>.
        Ok(())
    },
    |healthy| {
        tracing::info!(healthy, "watchdog health transition");
    },
    parent_cancel,
);

// Graceful stop (awaits the background task):
handle.stop().await;
// Or cancel-on-drop (does NOT await):
drop(handle);
```

The `parent_cancel` token lets the watchdog participate in tree-style
shutdown — the task exits as soon as the parent is cancelled.

---

## Differences from v1

- **No `HealthChecker`** — use `Resource::check()` directly or `WatchdogHandle`
- **No `QuarantineManager`** — replaced by `RecoveryGate` (simpler, CAS-based)
- **No `HealthState` enum** — health is a `bool` (healthy/unhealthy)
- **No `HealthPipeline`** — multi-stage checks removed
