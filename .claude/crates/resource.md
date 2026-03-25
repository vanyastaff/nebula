# nebula-resource

v2 complete — topology-agnostic resource management. RPITIT, 7 topologies, Manager with topology-specific acquire dispatch.

## Invariants

- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`
- `ErrorKind` determines retry: Transient/Exhausted = retryable
- Manager has `acquire_pooled`, `acquire_resident`, etc. (not one generic) — each topology has different trait bounds
- `ResourceHandle` RAII — guarded returns lease to pool on drop, tainted destroys
- Deprecated `Context`/`Scope` in `compat` for v1 migration

## Traps

- `ctx_ext::<T>()` not `Ctx::ext_raw()`
- `ResourceHandle::detach()` on Shared returns None
- `ReleaseQueue::submit` drops silently if all channels full
- `ExclusiveRuntime::acquire` requires `R::Runtime: Clone + Into<R::Lease>`
- `DaemonRuntime::start` errors if already running; `stop()` is idempotent
- `Registry::get_typed<R>` keys on `TypeId::of::<ManagedResource<R>>()` not `TypeId::of::<R>()`
- `Manager::lookup` returns `Err(Cancelled)` after shutdown
- Must drop Manager before `ReleaseQueue::shutdown` (holds Arc via ManagedResource)

## Relations

- Depends on: nebula-core
- Depended on by: nebula-action, nebula-plugin, nebula-engine, nebula-webhook
- Webhook still uses deprecated v1 compat types; migration tracked separately

<!-- reviewed: 2026-03-25 — v2 complete (Phase 6) -->
