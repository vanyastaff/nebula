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

let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));

manager.register(
    resource, config, (), ScopeLevel::Global,
    topology, None, Some(gate.clone()),
)?;
```

The Manager automatically:
1. **Checks the gate** before each acquire (`check_recovery_gate`)
2. **Triggers the gate** on transient acquire failures (`trigger_recovery_on_failure`)

This means callers don't need to interact with the gate directly — it works
transparently to prevent thundering herd on the failing backend.

---

## RecoveryGroupRegistry

Groups multiple resources behind a shared recovery gate (e.g., all databases
on the same server). Resources in the same group share gate state, so one
failure blocks all resources in the group.

```rust,ignore
let groups = manager.recovery_groups();
let gate = groups.get_or_create(RecoveryGroupKey::new("db-server-1"));
// Pass this gate to multiple register() calls
```

---

## WatchdogHandle

Opt-in background health probe that periodically calls `Resource::check()`:

```rust,ignore
use nebula_resource::{WatchdogConfig, WatchdogHandle};

let handle = WatchdogHandle::start(
    WatchdogConfig {
        interval: Duration::from_secs(30),
        probe_timeout: Duration::from_secs(5),
        failure_threshold: 3,    // 3 consecutive failures → unhealthy
        recovery_threshold: 1,   // 1 success → healthy again
    },
    probe_fn,
    |healthy| { /* health change callback */ },
);

// Graceful stop (awaits the background task):
handle.stop().await;
// Or cancel-on-drop (does NOT await):
drop(handle);
```

---

## Differences from v1

- **No `HealthChecker`** — use `Resource::check()` directly or `WatchdogHandle`
- **No `QuarantineManager`** — replaced by `RecoveryGate` (simpler, CAS-based)
- **No `HealthState` enum** — health is a `bool` (healthy/unhealthy)
- **No `HealthPipeline`** — multi-stage checks removed
