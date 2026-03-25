# nebula-resource

v2 rewrite — topology-agnostic resource management.

## Architecture (v2)
- Phase 1: primitives (Error, Ctx, Resource trait, Handle, Cell, ReleaseQueue, State, Options)
- Phase 2: 7 topology traits in `topology/`
- Phase 3: recovery layer (`recovery/`) + integration config (`integration/`)
- Phase 4: topology runtimes in `runtime/` — Pool, Resident, Service, Transport, Exclusive, EventSource, Daemon
- Phase 5: Manager + Registry + Events + Metrics — registration, scope-aware lookup, topology dispatch
- Resource trait uses RPITIT; `Ctx` is dyn-compatible with `ctx_ext::<T>()` free fn

## Invariants
- `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]`
- `Resource::key()` is a method, not `const KEY`
- `ErrorKind` determines retry: Transient/Exhausted = retryable
- `AnyResource` = dyn-safe marker for heterogeneous registration
- Topology traits extend `Resource` with RPITIT + default impls
- `RecoveryGate` uses CAS via `ArcSwap`; ticket drop auto-fails with backoff
- `Registry` stores `Arc<dyn AnyManagedResource>` with TypeId secondary index for typed downcast via `as_any_arc`
- `Manager` has topology-specific `acquire_pooled`, `acquire_resident`, etc. (not a single generic acquire) because each topology has different trait bounds
- Deprecated `Context`/`Scope` in `compat` (for migration)

## Traps
- `Ctx::ext_raw()` → use `ctx_ext::<T>()` instead
- `ResourceHandle::detach()` on Shared returns None
- `ReleaseQueue::submit` drops silently if all channels full
- `ExclusiveRuntime::acquire` requires `R::Runtime: Clone + Into<R::Lease>`
- `DaemonRuntime::start` returns error if already running (not idempotent); `stop()` is idempotent
- `Registry::get_typed<R>` uses `TypeId::of::<ManagedResource<R>>()`, not `TypeId::of::<R>()` — register must match
- `Manager::lookup` returns `Err(Cancelled)` if shutdown was called

## Relations
- Depends on: nebula-core
- Depended on by: nebula-action, nebula-plugin, nebula-engine, nebula-webhook

<!-- reviewed: 2026-03-25 — Phase 4a runtime implementations complete -->
