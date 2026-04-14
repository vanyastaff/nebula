# nebula-resource

Topology-agnostic resource management. RPITIT-based, 7 topologies, Manager with
topology-specific acquire dispatch.

## Invariants

- `ErrorKind` determines retry: Transient / Exhausted / Backpressure = retryable.
- `execute_with_resilience` respects `retry_after()` as backoff floor.
- Release queue fallback is bounded (4096) — drops tasks with `tracing::error!` when full.
- `register()` takes 6 params — convenience methods require `Auth = ()` bound.
- `ResourceHandle` RAII — guarded returns lease on drop, tainted destroys.
- Guarded permit drops **after** `catch_unwind` in `Drop` — prevents semaphore leak on panic.
- `TopologyTag` is a `#[non_exhaustive]` enum, not `&str`.
- `acquire_*_default` helpers constrain `Auth = ()` — all 5 topologies plus erased.
- `Resource::Auth` uses `nebula_core::AuthScheme` (not a local trait).
- **`Manager::graceful_shutdown` returns `Result<ShutdownReport, ShutdownError>` (#302).** On `DrainTimeoutPolicy::Abort` (default), drain timeout returns `Err(DrainTimeout { outstanding })` **without** clearing the registry — live handles stay valid. The shutdown CAS flag is reset on this abort path so callers can retry (for example, with a longer timeout or `Force`) instead of getting stuck on `AlreadyShuttingDown`. `#[non_exhaustive]` on `ShutdownConfig` — use `ShutdownConfig::default().with_drain_timeout(...)`.
- **Recovery gate admits healthy traffic without serialization.** `admit_through_gate` only issues a `RecoveryTicket` when the gate is in `Failed` and backoff has elapsed; `Idle` admits as open path to preserve pool concurrency. Retryable errors on open path transition the gate to transient-failed backoff; non-retryable errors do not permanently fail the gate.
- **`DaemonRuntime` is restart-safe (#318) and its backoff is cancel-responsive (#323).** Per-run cancel token is a child of the parent built on every `start()`; stale finished handles are dropped on restart. The restart-backoff sleep runs inside a `biased tokio::select!` against the per-run token.

## Traps

- Use `ctx_ext::<T>()`, not `Ctx::ext_raw()`.
- `ResourceHandle::detach()` returns `None` on Shared.
- Pool permit returns on handle drop **before** async recycle — a new caller can acquire during recycle.
- Async destroy cannot run in `Drop` — cancel-safety guards (`CreateGuard`, `SessionGuard`) cover this.
- `AcquireRetryConfig::max_attempts` is **total** attempts (including the initial try), not retries.
- `Registry::get_typed<R>` keys on `TypeId::of::<ManagedResource<R>>()`, not `TypeId::of::<R>()`.
- `WatchdogHandle::drop` cancels but does **not** await — use `stop()` for graceful shutdown.

## Decisions

- Metrics come from `ResourceOpsMetrics` backed by `MetricsRegistry` (nebula-telemetry). Per-resource atomics were removed — a single aggregate `Option<ResourceOpsMetrics>` lives on Manager.
- `ManagerConfig.metrics_registry: Option<Arc<MetricsRegistry>>` — opt-in, zero overhead when `None`.
- `ResourceHealthSnapshot.metrics: Option<ResourceOpsSnapshot>` — `None` when no registry.
- Runtime `acquire()` methods take `Option<ResourceOpsMetrics>` by clone (no `Arc`).

## Relations

Depends on: `nebula-core`, `nebula-metrics`, `nebula-telemetry`, `nebula-resource-macros`.
Depended on by: `nebula-action`, `nebula-plugin`, `nebula-engine`, `nebula-api` (webhook module lives there — there is no `nebula-webhook` crate).

<!-- reviewed: 2026-04-14 -->
