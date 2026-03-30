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

<!-- reviewed: 2026-03-30 -->
