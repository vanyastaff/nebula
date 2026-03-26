# nebula-resource

v2 complete — topology-agnostic resource management. RPITIT, 7 topologies, Manager with topology-specific acquire dispatch.

## Invariants

- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`
- `ErrorKind` determines retry: Transient/Exhausted = retryable
- `register()` takes 6 params (no credential param) — convenience methods still require `Credential = ()` bound
- Manager has per-topology `acquire_*` methods (not one generic) — different trait bounds per topology
- `AcquireResilience` optional on `register()` — wraps acquire with timeout + retry (no circuit-breaker field; removed as unwired)
- `ResourceHandle` RAII — guarded returns lease on drop, tainted destroys
- Guarded permit drops AFTER catch_unwind in Drop — prevents semaphore leak on panic
- `TopologyTag` is enum (not `&str`), `#[non_exhaustive]`
- `register_pooled/resident/service/exclusive/transport` convenience methods: `Credential = ()`, `ScopeLevel::Global`, no resilience/gate
- `register_*_with` variants accept `RegisterOptions` for scope/resilience/gate without full `register()` signature
- `acquire_pooled_default`/`acquire_resident_default` helpers pass `&()` credential — only for `Credential = ()`
- `ScopeLevel` derives `Default` (= `Global`); `RegisterOptions::default()` works out of the box
- Deprecated `Context`/`Scope` in `compat` for v1 migration

## Traps

- `ctx_ext::<T>()` not `Ctx::ext_raw()`
- `ResourceHandle::detach()` on Shared returns None
- `ReleaseQueue::submit` falls back to unbounded channel if primary full (warns at power-of-two intervals)
- Pool permit returns on handle drop BEFORE async recycle — new caller can acquire during recycle
- Cancel-safety guards (`CreateGuard`, `SessionGuard`) — async destroy cannot run in Drop, logs warning; use `unreachable!()` not `expect()` for invariants
- `graceful_shutdown` Phase 2 is drain-aware — tracks active handles via `AtomicU64 + Notify`, returns immediately when zero
- `ResourceHandle` carries optional `drain_counter` for shutdown coordination (set via `with_drain_tracker` in acquire methods)
- `AcquireRetryConfig::max_attempts` is TOTAL attempts (including initial try), not retries
- `Registry::get_typed<R>` keys on `TypeId::of::<ManagedResource<R>>()` not `TypeId::of::<R>()`
- Manager shares its `CancellationToken` with `ReleaseQueue` workers via `with_cancel()` — `cancel()` triggers worker drain+exit without needing to drop senders
- `ReleaseQueue::close()` cancels workers explicitly for standalone (non-Manager) usage
- `WatchdogHandle` cancels on drop but does NOT await — use `stop()` for graceful

## Relations

- Depends on: nebula-core, nebula-resource-macros (re-exports `ClassifyError` derive)
- Depended on by: nebula-action, nebula-plugin, nebula-engine, nebula-webhook

<!-- reviewed: 2026-03-25 — removed dead _credential param from register(), removed unwired AcquireCircuitBreakerPreset -->
