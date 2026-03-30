# nebula-resource

v2 complete — topology-agnostic resource management. RPITIT, 7 topologies, Manager with topology-specific acquire dispatch.

## Invariants

- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`
- `ErrorKind` determines retry: Transient/Exhausted = retryable
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

## Relations

- Depends on: nebula-core, nebula-resource-macros
- Depended on by: nebula-action, nebula-plugin, nebula-engine, nebula-webhook

<!-- reviewed: 2026-03-25 — renamed Resource::Credential to Resource::Auth (AuthScheme from nebula-core) -->
<!-- reviewed: 2026-03-29 — fixed timeout-per-attempt bug in execute_with_resilience; fixed ResourceMetrics atomic ordering (Acquire→Relaxed); implemented PoolRuntime::warmup (WarmupStrategy was dead config), PoolRuntime::try_acquire (non-blocking), PoolRuntime::stats → PoolStats{idle,capacity,available_permits,in_use}; fixed InstanceMetrics.error_count (never incremented → now tracks tainted returns); added Manager::try_acquire_pooled, try_acquire_pooled_default, pool_stats, warmup_pool; PoolStats exported from lib.rs -->
