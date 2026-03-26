# nebula-resource — Architecture

## Design Goals

- **Topology-agnostic management.** The `Manager` handles registration, lookup,
  resilience, and shutdown without knowing whether a resource is a pool, a shared
  singleton, or a one-at-a-time exclusive. Topology-specific behaviour lives
  entirely in the topology traits and their runtime structs.
- **Type safety without dynamic dispatch on the hot path.** Every acquire method
  is generic over `R`. The type-erased `Registry` is crossed once (at lookup), then
  the handle is fully typed for the rest of the call.
- **Zero-allocation futures.** RPITIT (`impl Future` in trait position) instead of
  `#[async_trait]` — no `Box<dyn Future>` per lifecycle call.
- **Panic safety.** Semaphore permits are held separately from release callbacks
  so a panicking callback cannot strand a slot.
- **No unsafe code.** `#![forbid(unsafe_code)]` is enforced at compile time.

---

## Key Design Decisions

### 1. Topology-per-trait, not one monolithic `Resource` trait

Seven topology traits (`Pooled`, `Resident`, `Service`, `Transport`, `Exclusive`,
`EventSource`, `Daemon`) each define only the lifecycle hooks relevant to their
access pattern. `Pooled` has `recycle` and `BrokenCheck`; `Resident` clones a
shared runtime; `Exclusive` uses a semaphore(1). Merging these into one trait
would either leave most methods as panicking stubs or force every implementor to
reason about all seven access patterns. The `Manager` dispatches per topology via
`acquire_pooled`, `acquire_resident`, etc., matching trait bounds to the variant.

### 2. RPITIT over `async_trait`

`Resource::create`, `check`, `shutdown`, and `destroy` return `impl Future + Send`
directly in the trait. This eliminates the `Box<dyn Future>` heap allocation that
`#[async_trait]` requires on every call. The trade-off is that trait objects
(`dyn Resource`) are not possible — but the `Manager` never needs them; it stores
typed `ManagedResource<R>` behind type erasure at a higher level.

### 3. Type erasure at the `Manager` boundary via `TypeId` keying

`Registry` stores every `ManagedResource<R>` as `Arc<dyn AnyManagedResource>`.
Lookup proceeds in two steps: a `TypeId::of::<ManagedResource<R>>()` secondary
index resolves the `ResourceKey`, then `Arc::downcast` recovers the typed
`Arc<ManagedResource<R>>`. This lets the manager hold heterogeneous resource types
in one `DashMap` without generics, while callers get back a fully typed handle
with no runtime cost after the single downcast.

### 4. RAII handles with a separate semaphore permit field

`ResourceHandle` carries the semaphore `OwnedSemaphorePermit` as a distinct field,
independent of the release callback. In `Drop`, the permit is taken out _before_
`catch_unwind` runs the callback. A panic in the callback cannot destroy the
permit, so the semaphore slot is always returned. Without this split, a panicking
callback would unwind through the `OwnedSemaphorePermit`, permanently leaking a
pool slot.

### 5. `ReleaseQueue` with cancel-safety guards

`Drop` is synchronous; pool recycle and lease destruction are async. The
`ReleaseQueue` bridges this: `Drop` submits a `TaskFactory` (a `FnOnce` that
produces a future) to a round-robin pool of background workers. Workers hold a
bounded channel (primary) plus an unbounded fallback so no task is ever dropped
under backpressure. Workers share the `Manager`'s `CancellationToken`: on
cancellation they close their channels, drain buffered tasks, and exit — without
requiring the senders to be dropped first.

### 6. Recovery gate as CAS state machine, not a health checker

`RecoveryGate` prevents thundering herd on dead backends. It is a pure
`ArcSwap`-based CAS state machine (`Idle → InProgress → Failed →
PermanentlyFailed`) with no background polling task. When an acquire fails
transiently, the caller passively triggers recovery via `try_begin`; subsequent
callers fast-fail with a retry-at hint until the backoff expires. This approach
costs zero threads and has no timer drift — the first caller after the backoff
window acts as the recovery probe.

---

## Module Map

```
nebula-resource/src/
│
├── resource.rs          Resource trait (5 associated types, 4 lifecycle methods)
│                        ResourceConfig, Credential, ResourceMetadata
│
├── topology/            One trait per access pattern
│   ├── pooled.rs        Pooled — N interchangeable instances, checkout/recycle
│   ├── resident.rs      Resident — one shared runtime, clone on acquire
│   ├── service.rs       Service — long-lived runtime, short-lived tokens
│   ├── transport.rs     Transport — shared connection, multiplexed sessions
│   ├── exclusive.rs     Exclusive — one caller at a time (semaphore-1)
│   ├── event_source.rs  EventSource — pull-based event subscription (secondary)
│   └── daemon.rs        Daemon — background run loop with restart policy (secondary)
│
├── runtime/             Stateful counterparts to the topology traits
│   ├── managed.rs       ManagedResource<R> — topology + hot-swap config + metrics
│   ├── pool.rs          PoolRuntime<R> — idle queue, semaphore, fingerprint
│   ├── resident.rs      ResidentRuntime<R> — ArcSwap Cell, lazy init
│   ├── service.rs       ServiceRuntime<R> — token factory around live runtime
│   ├── transport.rs     TransportRuntime<R> — session multiplexer
│   ├── exclusive.rs     ExclusiveRuntime<R> — semaphore(1) wrapper
│   ├── event_source.rs  EventSourceRuntime<R>
│   └── daemon.rs        DaemonRuntime<R> — spawned task with RestartPolicy
│
├── manager.rs           Manager — register, acquire_*, reload_config, shutdown
├── registry.rs          Registry — DashMap<ResourceKey, Vec<RegistryEntry>>
│                                   + TypeId secondary index for typed lookup
├── handle.rs            ResourceHandle<R> — Owned / Guarded / Shared modes, RAII
├── release_queue.rs     ReleaseQueue — background worker pool for async cleanup
│
├── recovery/
│   ├── gate.rs          RecoveryGate — CAS state machine (Idle/InProgress/Failed)
│   ├── group.rs         RecoveryGroupRegistry — shared gates keyed by group
│   └── watchdog.rs      WatchdogHandle — periodic liveness probe
│
├── cell.rs              Cell<T> — lock-free ArcSwap wrapper for Resident runtimes
├── integration.rs       AcquireResilience — timeout + retry config wired into Manager
├── ctx.rs               Ctx trait, ScopeLevel, Extensions
├── error.rs             Error, ErrorKind, ErrorScope
├── events.rs            ResourceEvent — lifecycle events (broadcast::Sender)
├── metrics.rs           ResourceMetrics — atomic counters, MetricsSnapshot
├── options.rs           AcquireOptions, AcquireIntent
├── state.rs             ResourcePhase, ResourceStatus
└── topology_tag.rs      TopologyTag — non_exhaustive enum identifying topology
```

---

## Data Flow

### Acquire path

```
Manager::acquire_pooled(credential, ctx, options)
  │
  ├─ lookup<R>(ctx.scope())
  │    Registry: TypeId → ResourceKey → downcast Arc<ManagedResource<R>>
  │    Returns Err::Cancelled if manager is shut down
  │
  ├─ check_recovery_gate(&managed.recovery_gate)
  │    Idle           → proceed
  │    InProgress     → Err::Transient (fast-fail)
  │    Failed         → Err::Exhausted with retry_at hint (unless backoff expired)
  │    PermanentlyFailed → Err::Permanent
  │
  ├─ execute_with_resilience(managed.resilience, || ...)
  │    Dispatches to TopologyRuntime::Pool(rt)::acquire(...)
  │    On transient failure: exponential backoff retry up to max_attempts
  │    On timeout: wraps each attempt in tokio::time::timeout
  │
  ├─ on failure: trigger_recovery_on_failure (passive gate trigger)
  ├─ record_acquire_result (metrics + ResourceEvent)
  │
  └─ handle.with_drain_tracker(manager.drain_tracker)
       Increments AtomicU64; decrements on handle drop; notifies shutdown waiter
```

### Release path (handle drop)

```
ResourceHandle<R>::drop
  │
  ├─ [Guarded] take OwnedSemaphorePermit out of field (before catch_unwind)
  ├─ catch_unwind(on_release(lease, tainted))
  │    on_release submits TaskFactory to ReleaseQueue
  │    ReleaseQueue::submit → round-robin primary channel, fallback if full
  ├─ _permit_guard drops → semaphore slot returned (even if callback panicked)
  │
  └─ drain_counter.fetch_sub(1) → notify_waiters() if reaches zero
```

### Shutdown path

```
Manager::graceful_shutdown(config)
  │
  ├─ Phase 1 — SIGNAL: cancel.cancel()
  │    Rejects new acquire calls (lookup checks is_cancelled)
  │    Signals ReleaseQueue workers to drain buffered tasks and exit
  │
  ├─ Phase 2 — DRAIN: wait_for_drain(config.drain_timeout)
  │    Polls AtomicU64 drain counter via Notify; returns immediately if zero
  │    Logs warning on timeout with remaining active handle count
  │
  ├─ Phase 3 — CLEAR: registry.clear()
  │    Drops all Arc<ManagedResource<R>>, releasing Arc<ReleaseQueue> refs
  │
  └─ Phase 4 — AWAIT WORKERS: ReleaseQueue::shutdown(handle)
       Joins all worker JoinHandles; bounded by 10s internal timeout
```

---

## Invariants

1. **`Manager` is cancel-aware before lookup.** Every `acquire_*` call checks
   `cancel.is_cancelled()` before touching the registry. Once shut down, the
   manager returns `Err::Cancelled` for all acquire calls.

2. **Semaphore permits are returned even on callback panic.** The
   `OwnedSemaphorePermit` field in `Guarded` handles is taken before
   `catch_unwind`, so pool capacity is never permanently reduced by a panicking
   release callback.

3. **Config hot-reload is generation-stamped.** `reload_config` atomically swaps
   the `ArcSwap<Config>` and increments an `AtomicU64` generation counter. Pool
   runtimes compare the generation at acquire time to detect stale idle instances
   and evict them on next recycle.

4. **`ReleaseQueue` never drops tasks under backpressure.** Primary bounded
   channels (256 slots per worker) overflow to an unbounded fallback channel. A
   task is only lost if the fallback channel is closed (which only happens after
   worker exit).

5. **Recovery gate transitions are CAS-only.** `RecoveryGate` state is stored in
   an `ArcSwap`; all transitions use compare-and-swap so concurrent callers never
   corrupt gate state. Only one caller wins `try_begin`; others receive typed
   errors.

6. **Type erasure is crossed exactly once per acquire.** `Registry::get_typed`
   performs one `TypeId` lookup and one `Arc::downcast`. The returned
   `Arc<ManagedResource<R>>` is fully typed for the rest of the acquire call.

7. **Events are best-effort, never blocking.** `event_tx.send()` is
   fire-and-forget on a `broadcast::Sender` with a 256-event buffer. Slow
   subscribers receive `RecvError::Lagged` and never stall the acquire path.

8. **No unsafe code.** `#![forbid(unsafe_code)]` is a compile-time guarantee
   across the entire crate.
