# Roadmap

## Phase 1: Contract and Safety Baseline

- deliverables:
  - finalize and publish `resource` cross-crate contracts (`INTERACTIONS`, `API`, `MIGRATION`).
  - formalize error handling guidance (`retryable`, fatal, validation).
  - lock scope invariants and deny-by-default behavior.
- risks:
  - hidden assumptions in action/runtime integration paths.
- exit criteria:
  - contract docs map 1:1 with implementation.
  - no unresolved scope or error taxonomy ambiguities.

## Phase 2: Runtime Hardening

- deliverables:
  - strengthen shutdown and reload behavior tests under in-flight load.
  - improve health-to-quarantine propagation observability.
  - add operational guardrails for invalid config reload attempts.
- risks:
  - regressions in pool swap and long-tail create/cleanup failures.
- exit criteria:
  - deterministic shutdown in stress tests.
  - no leaked permit/instance in CI race scenarios.

## Phase 3: Scale and Performance

- deliverables:
  - benchmark-driven tuning for acquire latency and pool contention.
  - back-pressure policy profiles (`fail-fast`, bounded wait, adaptive) if accepted.
  - high-cardinality metrics hygiene for multi-tenant workloads.
- risks:
  - policy complexity and incorrect defaults under burst traffic.
- exit criteria:
  - p95 acquire latency and exhaustion rates within SLO targets.
  - no perf regressions versus current throughput baseline.

## Phase 4: Ecosystem and DX

- deliverables:
  - adapter crate guidance (`resource-postgres`, `resource-redis`, etc.).
  - typed key migration path (if approved) and developer ergonomics updates.
  - cookbook for runtime/action integration patterns.
- risks:
  - ecosystem fragmentation if adapter contracts diverge.
- exit criteria:
  - at least one reference adapter and end-to-end sample integration.

## Phase 5: Pool Safety Hardening ✅

- deliverables:
  - `Gate`/`GateGuard` cooperative shutdown barrier in `nebula-resilience::gate`; wired into `PoolInner` so maintenance task holds a `GateGuard` and `shutdown()` calls `gate.close().await` before `semaphore.close()`.
  - `CounterGuard` RAII wrapper replacing manual `fetch_add`/`fetch_sub` pairs in `acquire_inner`.
  - `NEBULA_RESOURCE_CIRCUIT_BREAKER_OPENED_TOTAL` / `NEBULA_RESOURCE_CIRCUIT_BREAKER_CLOSED_TOTAL` metric constants; `MetricsCollector` emits dedicated CB open/close counters with `{resource_id, operation}` label.
- risks:
  - none remaining — all delivered and verified.
- exit criteria:
  - all gate unit tests and doctest pass.
  - `cargo check --workspace --all-targets` clean.
  - hardening checklist rows in `ARCHITECTURE.md` updated to Implemented.

## Phase 6: Adapter-Grade Safety and Instance-Aware Lifecycle ✅

- deliverables:
  - `Resource::is_broken` — synchronous broken check; wired as the first gate on the release path, before taint detection and recycle.
  - `Resource::prepare` — per-acquire async hook for token refresh, lease renewal, and pre-flight validation.
  - `Guard::detach()` — transfers ownership, fires `on_detach` (semaphore permit returned), skips `on_drop`/recycle.
  - `Guard::leak()` — transfers ownership without firing any callback; permit is forfeited permanently.
  - `PoolSharingMode { Exclusive, Shared }` enum; `PoolConfig::sharing_mode` field (default `Exclusive`).
  - `PoolConfig::warm_up` — spawns background warm-up on pool construction when `true`.
  - `Pool::acquire_shared` (where `R::Instance: Clone`) — returns a cloned instance without consuming a semaphore permit.
  - `HookEvent` extended: `PostCreate`, `PreAcquire`, `PostRecycle`, `PostRelease` (8 total hook points).
  - `InstanceMetadata { created_at, idle_since, acquire_count }` threaded through the acquire/return hotpath.
  - `Resource::is_reusable_with_meta` / `Resource::recycle_with_meta` called throughout acquire and release paths.
  - `Pool::retain(predicate)` — conditional idle eviction; returns count removed.
  - `Pool::set_max_size(n)` — live pool size resizing; returns `Err` if n < current idle count.
  - Dynamic `context_enricher` in `Manager` — `pools` wrapped in `Arc<ArcSwap<...>>`; enricher resolves dep handles from live snapshot at acquire time.
- risks:
  - none remaining — all delivered and verified (355 tests pass, 0 warnings).
- exit criteria:
  - 355 tests pass; `cargo check -p nebula-resource` → 0 errors, 0 warnings.
  - All new API surface documented in `crates/resource/docs/`.

## Metrics of Readiness

- correctness:
  - invariant tests pass for scope, shutdown, health cascade, and dependency graph consistency.
- latency:
  - acquire p95/p99 tracked and bounded in benchmark profile.
- throughput:
  - sustained concurrent acquire/release without leak or starvation.
- stability:
  - no flaky critical-path tests in release CI.
- operability:
  - actionable events/metrics for triage and capacity planning.
