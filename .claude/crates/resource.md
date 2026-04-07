# nebula-resource

v2 complete — topology-agnostic resource management. RPITIT, 7 topologies, Manager with topology-specific acquire dispatch.

## Invariants

- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`
- `ErrorKind` determines retry: Transient/Exhausted/Backpressure = retryable
- `execute_with_resilience` respects `retry_after()` as backoff floor
- Release queue fallback is bounded (4096) — drops tasks with `tracing::error!` when full
- `register()` takes 6 params — convenience methods require `Auth = ()` bound
- `ResourceHandle` RAII — guarded returns lease on drop, tainted destroys
- Guarded permit drops AFTER catch_unwind in Drop — prevents semaphore leak on panic
- `TopologyTag` is enum (not `&str`), `#[non_exhaustive]`
- `acquire_*_default` helpers constrain `Auth = ()` — all 5 topologies plus erased
- `Resource::Auth` uses `nebula_core::AuthScheme` (not a local trait)

## Traps

- `ctx_ext::<T>()` not `Ctx::ext_raw()`
- `ResourceHandle::detach()` on Shared returns None
- Pool permit returns on handle drop BEFORE async recycle — new caller can acquire during recycle
- Cancel-safety guards (`CreateGuard`, `SessionGuard`) — async destroy cannot run in Drop
- `AcquireRetryConfig::max_attempts` is TOTAL attempts (including initial try), not retries
- `Registry::get_typed<R>` keys on `TypeId::of::<ManagedResource<R>>()` not `TypeId::of::<R>()`
- `WatchdogHandle` cancels on drop but does NOT await — use `stop()` for graceful

## Decisions

- `Ctx: BaseCtx` supertrait — scope/identity methods come from `nebula_core::BaseCtx`, resource's `Ctx` adds only cancel_token + extensions
- `ScopeLevel` is `nebula_core::ScopeLevel` (typed IDs: `OrganizationId`, `ProjectId`) — local enum deleted
- `BasicCtx` implements `BaseCtx` + `Ctx` separately — BaseCtx gives scope/execution_id, Ctx gives cancel/ext
- `execution_id()` returns `Option<&ExecutionId>` (via BaseCtx) — daemon.rs uses `.copied().unwrap_or_else(ExecutionId::new)` as fallback
- `Resource::parameters() -> ParameterCollection` default method — returns empty collection; overridden to describe setup form fields
- `ResourceMetadata.parameters` — schema stored alongside name/description/tags for UI rendering
- `AnyResource::parameters()` — type-erased access for registry/engine without generics
- Same pattern as `Credential::parameters()` and `ActionMetadata.parameters`
- `ResourceMetrics` (hand-rolled atomics) replaced with `ResourceOpsMetrics` backed by `MetricsRegistry` from nebula-telemetry
- Per-resource metrics removed — single aggregate `Option<ResourceOpsMetrics>` on Manager (all topologies share same registry counters)
- `ManagerConfig.metrics_registry: Option<Arc<MetricsRegistry>>` — metrics are opt-in, zero overhead when None
- `ResourceHealthSnapshot.metrics` is now `Option<ResourceOpsSnapshot>` (None when no registry)
- Runtime `acquire()` methods take `Option<ResourceOpsMetrics>` (Clone, no Arc needed) instead of `Arc<ResourceMetrics>`

## Relations

- Depends on: nebula-core (BaseCtx, ScopeLevel, IDs), nebula-parameter (ParameterCollection), nebula-metrics, nebula-telemetry, nebula-resource-macros
- Depended on by: nebula-action, nebula-plugin, nebula-engine, nebula-webhook

<!-- reviewed: 2026-03-30 (backpressure retryable, retry_hint floor, bounded release queue, new events) -->
<!-- reviewed: 2026-03-31 -->
<!-- reviewed: 2026-04-02 — no architectural changes this session; pre-existing modifications in git status unrelated to resilience work -->
<!-- reviewed: 2026-04-02 -->

<!-- reviewed: 2026-04-02 -->

<!-- reviewed: 2026-04-02 — dep cleanup only: removed unused Cargo.toml deps via cargo shear --fix, no code changes -->
<!-- reviewed: 2026-04-04 — replaced ResourceMetrics with registry-backed ResourceOpsMetrics, removed per-resource metrics -->
<!-- reviewed: 2025-07-25 — BaseCtx supertrait, ScopeLevel dedup from core, execution_id now Option -->
<!-- reviewed: 2026-04-05 — ParameterCollection on Resource + ResourceMetadata + AnyResource -->
