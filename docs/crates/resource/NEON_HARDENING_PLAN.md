# Neon-Inspired Hardening Plan (Resource)

Related spec: `NEON_HARDENING_SPEC.md`

Last reviewed: **2026-03-14** — re-studied `neondatabase/neon` proxy and pageserver internals.

## Status Summary

- completed:
	- poison guard model integrated for pool mutable state
	- create/recycle circuit breaker integration via `nebula-resilience`
	- breaker-open/closed events and typed error surface
	- create/recycle timeout envelope
	- tests for poison, breaker behavior, timeout behavior
	- WP6: `Gate`/`GateGuard` shutdown fence in `nebula-resilience::gate`; wired into `Pool` (maintenance task + shutdown())
	- WP7: RAII `CounterGuard` replaces manual `fetch_add`/`fetch_sub` in `acquire_inner`
	- WP8: dedicated `nebula_resource_circuit_breaker_opened_total` + `nebula_resource_circuit_breaker_closed_total` counters with `operation` label
- in-progress:
	- (none)
- next:
	- WP5 (docs): ops-facing dashboard guidance for breaker saturation

## Work Packages

## WP1: Pool Cancel-Safety Baseline

Objective:
- ensure interrupted critical sections cannot silently reuse corrupted state

Acceptance:
- pool state accesses use poison arm/disarm flow
- poisoned state produces deterministic failure with diagnostic context

## WP2: Pool Local Failure Storm Protection

Objective:
- stop repeated expensive failures in create/recycle loops

Acceptance:
- create and recycle are breaker-guarded
- breaker-open returns explicit error and emits explicit event
- half-open success transitions back to closed

## WP3: Timeout Envelope and Cleanup Integrity

Objective:
- cap worst-case stall time per create/recycle operation

Acceptance:
- zero timeout is rejected by config validation
- timeout in create returns timeout error
- timeout in recycle triggers cleanup path (no idle leak)

## WP4: Layered Resilience Contract

Objective:
- keep policy ownership clear across crates

Acceptance:
- docs clearly define:
	- pool local breaker protection in `nebula-resource`
	- action-level retry/backoff/rate-limit in engine/runtime

## WP5: Observability and Operational Guidance

Objective:
- make breaker and poisoning behavior easy to operate in production

Acceptance:
- docs include event/error mapping for alerting
- cookbook includes recommended policy profile usage
- circuit breaker saturation dashboard guidance added to `docs/cookbook.md`

## WP6: Gate/GateGuard Shutdown Fence for Background Subtasks

Motivation (from Neon study):
Neon's pageserver and storage-controller use a `Gate / GateGuard` pattern alongside
`CancellationToken` for every background subtask: signal with token, _block_ with gate.
Currently `Pool<R>` only tracks `maintenance_handle: JoinHandle<()>` and `autoscale.rs`
exposes its own separate `JoinHandle`. There is no unified fence that prevents shutdown
from proceeding while any in-flight operation holds a reference into pool internals.

Placement: **`nebula-resilience`** (cross-cutting crate, not `nebula-resource`).
- `Gate` is a general concurrency primitive for lifecycle management, not resource-specific.
- Neon keeps it in `libs/utils/src/sync/gate.rs` — a shared utility used by pageserver,
  storage-controller, and proxy alike.
- In Nebula, `nebula-engine` and `nebula-runtime` will also need shutdown fencing for their
  own background tasks; placing `Gate` in the cross-cutting `nebula-resilience` makes it
  available to all layers without introducing upward dependencies.

Design:
- Add `gate.rs` inside `nebula-resilience/src/` and re-export from `prelude`:
  - Fields: `sem: Arc<Semaphore>` (capacity = `u32::MAX`), `closing: AtomicBool`.
  - `enter() -> Result<GateGuard, GateClosed>`: `try_acquire()` from sem, forget permit.
  - `close() -> impl Future`: set `closing = true`, `acquire_many(MAX).await`, then
    `sem.close()` — blocks until all guards are dropped.
  - `GateGuard::drop`: calls `sem.add_permits(1)` plus logs current span if gate is closing.
- `nebula-resource` imports `Gate` from `nebula-resilience` (already a dep).
- `PoolInner` gains a `gate: Gate` field.
- Every background subtask acquires a `GateGuard` on entry;
  `Pool::shutdown()` calls `gate.close().await` after cancelling the token.
- `AutoScaler::start` accepts an optional `GateGuard` so the pool can fence the scaler too.

Acceptance:
- `shutdown()` cannot return while any maintenance or autoscaler subtask holds a guard.
- `Gate::enter()` after `close()` returns `GateClosed` immediately (no hang).
- Logging: gate-holder span is emitted at WARN level if close() stalls > 1 s.
- Tests: `gate_enter_after_close_fails`, `shutdown_waits_for_gate_holders`.

## WP7: RAII CounterGuard for Waiting/Active Metrics

Motivation:
The acquire path contains manual `waiting_count.fetch_add(1) … fetch_sub(1)` pairs in
`acquire_inner`. On the early-return error path the sub is correct today, but future
refactors risk double-decrement or missed-decrement bugs.

Design:
- Introduce a lightweight `CounterGuard` in `pool.rs` (or `metrics.rs`):
  ```rust
  struct CounterGuard(Arc<AtomicUsize>);
  impl CounterGuard {
      fn new(counter: &Arc<AtomicUsize>) -> Self {
          counter.fetch_add(1, Ordering::Relaxed);
          Self(Arc::clone(counter))
      }
  }
  impl Drop for CounterGuard {
      fn drop(&mut self) { self.0.fetch_sub(1, Ordering::Relaxed); }
  }
  ```
- Replace the paired fetch_add/fetch_sub in `acquire_inner` with a single `CounterGuard`.
- Apply the same pattern to any future `active_ops` counter.

Acceptance:
- Counter never goes negative under cancellation races (property test).
- No clippy lint regressions.

## WP8: Circuit Breaker Metric Hooks

Motivation:
Neon exposes `pageserver_circuit_breaker_broken_total` and
`pageserver_circuit_breaker_unbroken_total` counters for alerting and dashboards.
nebula-resource emits `ResourceEvent::CircuitBreakerOpen/Closed` but has no per-operation
counters in `metrics.rs` yet.

Design:
- Add `circuit_breaker_opened: IntCounterVec` and `circuit_breaker_closed: IntCounterVec`
  to the resource metrics registry, labelled by `{resource_key, operation}`.
- Increment `opened` counter inside `emit_breaker_open()`.
- Increment `closed` counter inside `emit_breaker_closed()`.
- Document label semantics in `docs/cookbook.md` with example Prometheus alert expression.

Acceptance:
- Metrics are registered and visible in the metrics endpoint.
- `metrics_integration` test asserts counter increments after breaker trip.

## Verification Checklist

- `cargo check -p nebula-resource`
- `cargo test -p nebula-resource`
- `cargo clippy -p nebula-resource -- -D warnings`

## Notes

- This plan intentionally does not move global retry orchestration into `nebula-resource`.
- Breaking changes are allowed in current project stage when they improve long-term API clarity.
- WP6 (Gate) was identified as the highest-priority remaining item from the Neon re-study;
  the Gate pattern is Neon's primary mechanism for safe concurrent shutdown.
- WP7 and WP8 are polish items that improve correctness and observability without API breaks.

## Placement Decisions

| Primitive | Crate | Rationale |
|---|---|---|
| `Gate / GateGuard` | `nebula-resilience` | Generic lifecycle sync primitive; needed by engine/runtime/resource alike. No domain deps. |
| `Poison<T>` | `nebula-resource` | Wraps a concrete `T`; uses `chrono`; pool-internal concern. Low reuse potential outside resource. |
| `CircuitBreaker` | `nebula-resilience` | Already there; resource layer imports it as a dep. |
| `CounterGuard` (WP7) | `nebula-resource` (pool.rs) | Tiny RAII helper local to one acquire path; no value extracting until reuse need arises. |

