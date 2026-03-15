# Neon-Inspired Hardening Spec (Resource)

Last reviewed: **2026-03-14** — re-studied `neondatabase/neon` proxy and pageserver internals.

## Scope

This spec defines reliability hardening for `nebula-resource` pool internals using patterns inspired by Neon.

Goals:
- prevent inconsistent pool state reuse after interrupted critical sections
- avoid create/recycle failure storms under degraded dependencies
- keep action-level resilience policy centralized in engine/runtime
- ensure background subtask shutdown is fence-safe (no dangling subtask after `Pool::shutdown`)
- expose circuit breaker state as first-class metrics for ops alerting

## Non-Goals

- moving all retries into `nebula-resource`
- replacing existing scope/health/quarantine model
- introducing cross-crate dependency violations

## Invariants

1. Pool mutable state is never reused silently after unsafe interruption.
2. Create/recycle expensive paths are guarded by circuit breakers.
3. Breaker-open state is observable via explicit events and classified errors.
4. Action-level retry/backoff/rate-limit remains outside `nebula-resource`.
5. Shutdown and cleanup remain deterministic under concurrent load.
6. `Pool::shutdown()` cannot return while any background subtask holds a `GateGuard`.
7. Counter gauges (waiting, active) are maintained via RAII guards to prevent leak on panic/cancel.
8. Circuit breaker trip/recovery is reflected as monotonic metric counters labelled by operation.

## Architecture Split With `nebula-resilience`

### Inside `nebula-resource`

- Use `nebula-resilience` circuit breaker on pool `create` and `recycle` paths.
- Map breaker-open to resource-level errors/events.
- Keep pool self-protection local to pool operation boundary.
- Use `Gate/GateGuard` from **`nebula-resilience`** (cross-cutting) for background-subtask
  shutdown fencing, mirroring Neon's `libs/utils/src/sync/gate.rs`:
  - `Gate::enter()` — `try_acquire` one permit, forget it; returns `GateClosed` if closing.
  - `Gate::close()` — `acquire_many(MAX_UNITS).await`, then close semaphore.
  - `GateGuard::drop` — `add_permits(1)`; logs span at WARN if gate is actively closing.
  - Used together with `CancellationToken`: token signals intent, gate blocks completion.
  - Rationale: `Gate` is not resource-specific; `nebula-engine` and `nebula-runtime` will
    need the same primitive for their background tasks.

### Outside `nebula-resource` (engine/runtime/action loop)

- Keep retry/backoff/rate-limit orchestration at action execution boundary.
- Consume error category/retry hints from resource layer.

This layered split avoids duplicated policy and keeps pool internals safe under pressure.

## Neon Reference Patterns (2026-03-14 study)

Key patterns extracted from `neondatabase/neon` that inform this hardening:

| Pattern | Neon location | nebula-resource mapping |
|---|---|---|
| Gate/GateGuard | `libs/utils/src/sync/gate.rs` | WP6: new `gate.rs` in **`nebula-resilience`**, imported by `nebula-resource` |
| CancellationToken + Gate together | pageserver `Timeline`, storage-controller `Reconciler` | `Pool::shutdown`: cancel token, then `gate.close().await` |
| RAII permit accounting | `GateGuard::drop` calls `add_permits(1)` | WP7: `CounterGuard` for `waiting_count` |
| Span capture in guard | `GateGuard` stores `tracing::Span::current()` | WP6: log gate-holder span on slow close |
| Circuit breaker metrics | `pageserver_circuit_breaker_broken_total` | WP8: `circuit_breaker_opened/closed` counters |
| Connection health before reuse | proxy `is_closed()` + `reset()` check | existing `Resource::is_valid` path |
| Sharded pool map | proxy `ClashMap` sharding | not needed at current scale; revisit if >10k pools |

## Error and Event Contract

Expected signals:
- `Error::CircuitBreakerOpen { operation, retry_after }`
- `ResourceEvent::CircuitBreakerOpen { operation, retry_after }`
- `ResourceEvent::CircuitBreakerClosed { operation }`
- Poisoned-state failures mapped to internal errors with clear diagnostics.

## Timeout Envelope

Pool supports optional operation timeouts:
- `create_timeout`
- `recycle_timeout`

Timeouts must be `> 0` when configured.

## Test Matrix (minimum)

1. Poison arm/disarm happy path.
2. Poison drop-without-disarm path.
3. Acquire fails after deliberate pool poison.
4. Create breaker opens after threshold failures.
5. Half-open probe closes breaker on success.
6. Recycle breaker-open path skips recycle call.
7. Create timeout returns timeout error.
8. Recycle timeout triggers cleanup path.
9. `Gate::enter()` after `Gate::close()` returns `GateClosed` immediately.
10. `Pool::shutdown()` blocks until all maintenance-task `GateGuard`s are dropped.
11. `CounterGuard` — counter returns to 0 even when holder is dropped on cancelled path.
12. Circuit breaker metric counter increments on trip; decrements on close.

## Documentation Requirements

The following docs must stay in sync:
- `ARCHITECTURE.md`
- `VISION.md`
- `docs/cookbook.md` (include Gate/GateGuard usage guide + breaker saturation alert examples)
- `docs/adapters.md`
- `TASKS.md`
