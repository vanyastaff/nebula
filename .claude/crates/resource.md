# nebula-resource

v2 complete — topology-agnostic resource management. RPITIT, 7 topologies, Manager with topology-specific acquire dispatch.

## Invariants

- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`
- `ErrorKind` determines retry: Transient/Exhausted = retryable
- Manager has `acquire_pooled`, `acquire_resident`, etc. (not one generic) — each topology has different trait bounds; all accept `&AcquireOptions`
- `AcquireResilience` optional on `register()` — when set, all `acquire_*` methods wrap topology calls with timeout + retry (exponential backoff, only retries retryable errors)
- `AcquireOptions` threaded through all acquire paths — pool, transport, and exclusive use `options.remaining()` for semaphore timeout
- Transport `Config::acquire_timeout` (default 30s) caps semaphore wait; overridden by `AcquireOptions::deadline` when set
- Exclusive `Config::acquire_timeout` (default 30s) caps semaphore wait; overridden by `AcquireOptions::deadline` when set
- Resident `Config::create_timeout` (default 30s) wraps `resource.create()` in `tokio::time::timeout`; destroy gets a hard 10s timeout — prevents create_lock deadlock when backend hangs
- `ResourceHandle` RAII — guarded returns lease to pool on drop, tainted destroys
- `HandleInner::Guarded` holds `permit: Option<OwnedSemaphorePermit>` — permit drops AFTER catch_unwind in Drop, preventing leak on callback panic
- `TopologyTag` enum (not `&str`) identifies handle origin — `#[non_exhaustive]`, use `as_str()` for display
- `Manager::graceful_shutdown(ShutdownConfig)` — async 3-phase: cancel token, drain wait, log remaining; `shutdown()` stays sync for backward compat
- Manager emits `ResourceEvent` via `broadcast::channel(256)` on register/remove/acquire — subscribe via `subscribe_events()`
- `Manager.metrics` is aggregate `Arc<ResourceMetrics>` — `ManagedResource.metrics` is per-resource; both incremented on acquire/create
- Per-resource metrics passed to topology runtimes; aggregate stays on Manager for rollup
- `Manager::resource_metrics(key, scope)` returns per-resource `Arc<ResourceMetrics>` via `AnyManagedResource::metrics()`
- `ScopeLevel::Workflow(WorkflowId)` and `ScopeLevel::Execution(ExecutionId)` — typed IDs, not String
- Deprecated `Context`/`Scope` in `compat` for v1 migration

## Traps

- `ctx_ext::<T>()` not `Ctx::ext_raw()`
- `ResourceHandle::detach()` on Shared returns None
- `ReleaseQueue::submit` drops silently if all channels full
- Pool semaphore permit returns on handle drop, BEFORE async recycle — new caller can acquire while old entry recycles
- Cancel-safety guards (`CreateGuard`, `SessionGuard`) wrap entries/sessions between creation and handle construction — if the future is cancelled, the guard logs via `tracing::warn` and drops the runtime/session (native `Drop` only; async `destroy`/`close_session` cannot run in `Drop`)
- `ExclusiveRuntime::acquire` requires `R::Runtime: Clone + Into<R::Lease>`
- `DaemonRuntime::start` errors if already running; `stop()` is idempotent
- `Registry::get_typed<R>` keys on `TypeId::of::<ManagedResource<R>>()` not `TypeId::of::<R>()`
- `Manager::lookup` returns `Err(Cancelled)` after shutdown
- Must drop Manager before `ReleaseQueue::shutdown` (holds Arc via ManagedResource)
- `WatchdogHandle` cancels on drop but does NOT await — use `stop()` for graceful shutdown

## Relations

- Depends on: nebula-core
- Depended on by: nebula-action, nebula-plugin, nebula-engine, nebula-webhook
- Webhook still uses deprecated v1 compat types; migration tracked separately

<!-- reviewed: 2026-03-25 — AcquireResilience wired into acquire path with timeout + retry -->
