# Recovery

Thundering-herd prevention for a flapping backend.

---

## Overview

When a backend fails, naive retry logic causes every caller to independently
hammer the dead service — a thundering herd. `RecoveryGate` (`crate::recovery`)
serializes recovery attempts through a CAS-based state machine so only one
caller probes the backend at a time; everyone else gets a typed, immediate
answer instead of piling onto a dying connection.

---

## State machine

```text
Idle ──try_begin──▶ InProgress ──resolve──▶ Idle
 ▲                       │
 │                  fail_transient
 │                       │
 │                       ▼
 └──(retry_at expired)── Failed ──(max_attempts)──▶ PermanentlyFailed
```

| State | Meaning | What callers see |
|-------|---------|-------------------|
| `Idle` | Backend presumed healthy | Acquire proceeds normally |
| `InProgress` | One caller is probing | Others get a `Transient` error immediately |
| `Failed` | Last probe failed | Callers get `Exhausted` with a `retry_after` hint |
| `PermanentlyFailed` | `max_attempts` exceeded | Callers get `Permanent`; requires `gate.reset()` |

## `RecoveryTicket` (RAII)

`try_begin()` hands the winning caller a `#[must_use]` `RecoveryTicket`. The
caller **must** resolve it:

- `ticket.resolve()` — backend is healthy again → `Idle`.
- `ticket.fail_transient(msg)` — still down → `Failed`, schedule retry.
- `ticket.fail_permanent(msg)` — give up → `PermanentlyFailed`, requires
  manual `gate.reset()`.

A dropped ticket (no explicit resolution) auto-fails with a transient error
so the gate can never get stuck in `InProgress`.

## Backoff

Failed attempts back off exponentially (`base_backoff` doubled per attempt,
capped at 5 minutes), then randomized within an **equal-jitter** band —
`[nominal / 2, nominal]` — so a cohort of callers that all failed at the same
instant does not all retry in lockstep (the classic thundering-herd-on-retry
failure mode). `RecoveryGateConfig::max_attempts` (default `5`) and
`base_backoff` (default `1 s`) are the two tunables.

---

## Wiring into `Manager`

Pass an optional `Arc<RecoveryGate>` via `RegistrationSpec::recovery_gate`
when registering a resource — see the doctest on `Manager::register` for the
registration shape. The manager checks the gate before each acquire and
triggers it on transient acquire failures; callers never interact with the
gate directly.

To share one gate across multiple resources on the same backend (so one
failure fast-fails acquires for all of them), construct a single
`Arc::new(RecoveryGate::new(config))` and pass a clone of it in each
resource's `RegistrationSpec::recovery_gate`.

---

## Background health probes

`nebula-resource` does **not** ship a built-in background health-probe type.
The framework maintenance reaper already probes idle pool entries via
`Provider::check` (see `topology-reference.md`'s "Background health probes"
section for the `CheckCost`-driven cadence); for anything beyond that, drive
`Provider::check()` from an application-owned `tokio::spawn` loop. The
manager publishes `ResourceEvent::HealthChanged` on every observed
transition, so consumers can react without polling.

---

## See also

- [`pooling.md`](pooling.md) — the pool topology this most commonly guards.
- [`events.md`](events.md) — `RecoveryGateChanged` and `HealthChanged` events.
- The crate-root "Tuning" rustdoc section — the config-knob table incl. `RecoveryGateConfig`.
