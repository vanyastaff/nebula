# 07 — Module Layout & Implementation Plan

---

## Module layout

```
nebula-resource/
├── Cargo.toml
├── src/
│   ├── lib.rs                      // pub use, feature gates
│   │
│   │   ┌─── Core ───┐
│   ├── resource.rs                 // Resource, ResourceConfig traits (type Lease)
│   ├── ctx.rs                      // Ctx trait, BasicCtx, Extensions (uses ScopeLevel from nebula-core)
│   ├── error.rs                    // Error, ErrorKind, ErrorScope
│   ├── classify.rs                 // ClassifyError derive macro support
│   ├── metadata.rs                 // ResourceMetadata
│   ├── credential.rs              // CredentialType re-export, minimal bridge
│   │
│   │   ┌─── Topology traits ───┐
│   ├── topology/
│   │   ├── mod.rs                  // TopologyKind enum, re-exports
│   │   ├── pooled.rs              // Pooled, BrokenCheck, RecycleDecision
│   │   ├── resident.rs            // Resident (where Lease: Clone)
│   │   ├── service.rs             // Service (acquire_token → Lease)
│   │   ├── transport.rs           // Transport (open_session → Lease)
│   │   ├── exclusive.rs           // Exclusive (reset framework-managed)
│   │   ├── event_source.rs        // EventSource (type Subscription)
│   │   └── daemon.rs              // Daemon (single CancellationToken)
│   │
│   │   ┌─── Extension traits (v2, deferred) ───┐
│   │   // ConnectionAware, InfraProvider — deferred to v2.
│   │   // Will be added as extension/ module when v1 is stable.
│   │
│   │   ┌─── Lease & Handle ───┐
│   ├── lease/
│   │   ├── mod.rs
│   │   ├── guard.rs               // LeaseGuard<L> (pool-internal)
│   │   ├── poison.rs              // PoisonToken
│   │   └── options.rs             // AcquireOptions, AcquireIntent
│   │
│   ├── handle.rs                  // ResourceHandle<R>, HandleInner (3 variants: Owned/Guarded/Shared)
│   │
│   │   ┌─── Runtime impls ───┐
│   ├── runtime/
│   │   ├── mod.rs                  // TopologyRuntime<R> enum (7 variants)
│   │   ├── managed.rs             // ManagedResource<R>, AnyManagedResource trait
│   │   │
│   │   ├── pool/
│   │   │   ├── mod.rs             // pool::Runtime<R>
│   │   │   ├── config.rs          // pool::Config, Strategy, WarmupStrategy, CheckPolicy
│   │   │   ├── entry.rs           // PoolEntry<R>, InstanceMetrics (3 pub + 3 pub(crate))
│   │   │   ├── idle_queue.rs      // IdleQueue (LIFO/FIFO)
│   │   │   ├── acquire.rs         // checkout + create + prepare (is_broken + is_retryable loop)
│   │   │   ├── release.rs         // framework policy THEN recycle dispatch via ReleaseQueue
│   │   │   └── maintenance.rs     // maintenance loop (reap, probe, memory pressure shrink)
│   │   │
│   │   ├── resident/
│   │   │   ├── mod.rs             // resident::Runtime<R> (ArcSwap-based Cell)
│   │   │   ├── config.rs          // resident::Config
│   │   │   └── health.rs          // is_alive polling loop
│   │   │
│   │   ├── service/
│   │   │   ├── mod.rs             // service::Runtime<R> (Arc-based, natural drain)
│   │   │   └── config.rs          // service::Config
│   │   │
│   │   ├── transport/
│   │   │   ├── mod.rs             // transport::Runtime<R> (close_session via ReleaseQueue)
│   │   │   ├── config.rs          // transport::Config
│   │   │   └── session.rs         // session management
│   │   │
│   │   ├── exclusive/
│   │   │   ├── mod.rs             // exclusive::Runtime<R> (Arc<Semaphore> + OwnedSemaphorePermit)
│   │   │   └── config.rs          // exclusive::Config
│   │   │
│   │   ├── event_source/
│   │   │   ├── mod.rs             // event_source::Runtime<R>
│   │   │   ├── config.rs          // event_source::Config
│   │   │   └── handle.rs          // EventStreamHandle
│   │   │
│   │   └── daemon/
│   │       ├── mod.rs             // daemon::Runtime<R>
│   │       ├── config.rs          // daemon::Config, RestartPolicy (Never/OnFailure/Always)
│   │       └── runner.rs          // framework restart loop (see 02-topology.md)
│   │
│   │   ┌─── Primitives ───┐
│   ├── cell.rs                    // Cell<T>: lock-free ArcSwap-based
│   ├── release_queue.rs           // ReleaseQueue + ReleaseQueueHandle (shared per ManagedResource)
│   ├── state.rs                   // AtomicRuntimeState, RuntimeState enum
│   │
│   │   ┌─── Recovery ───┐
│   ├── recovery/
│   │   ├── mod.rs
│   │   ├── gate.rs                // RecoveryGate, GateState, RecoveryTicket
│   │   ├── group.rs               // RecoveryGroup, RecoveryGroupRegistry
│   │   └── watchdog.rs            // WatchdogHandle, WatchdogConfig
│   │
│   │   ┌─── Integration (cross-cutting crates) ───┐
│   ├── integration/
│   │   ├── mod.rs
│   │   ├── resilience.rs          // AcquireResilience + ResilienceError → Error (← nebula-resilience)
│   │   ├── config.rs              // AsyncConfigurable impl for Manager (← nebula-config)
│   │   └── memory.rs              // Adaptive pool sizing + lookup cache (← nebula-memory)
│   │
│   │   ┌─── Registry & Manager ───┐
│   ├── registry/
│   │   ├── mod.rs                 // Registry: DashMap-based, AnyManagedResource
│   │   ├── lookup.rs              // Scope-aware lookup (typed fast + erased cold)
│   │   └── scoped.rs              // ScopedRuntime
│   │
│   ├── manager/
│   │   ├── mod.rs                 // Manager struct (+TelemetryService, +EventBus, +MemoryMonitor)
│   │   ├── builder.rs             // Typestate RegistrationBuilder (7 finishers)
│   │   ├── acquire.rs             // acquire → TopologyRuntime dispatch → ResourceHandle
│   │   └── shutdown.rs            // ShutdownOrchestrator
│   │   // ResourceGroup, FallbackChain — deferred to v2.
│   │
│   │   ┌─── Observability ───┐
│   ├── scope.rs                   // ResourceScope, capability-based
│   ├── dependency.rs              // Dependencies trait, static TypeId arrays
│   ├── health.rs                  // HealthStatus, HealthChecker
│   ├── events.rs                  // ResourceEvent catalog + ScopedEvent impl (← nebula-eventbus)
│   └── metrics.rs                 // ResourceMetrics wrapper (← nebula-metrics + nebula-telemetry)
│
├── macros/
│   └── src/
│       ├── lib.rs                 // proc macros
│       ├── derive_resource.rs     // #[derive(Resource)]
│       └── derive_classify.rs     // #[derive(ClassifyError)]
│
└── tests/
    ├── pool_lifecycle.rs
    ├── resident_lifecycle.rs
    ├── service_lifecycle.rs
    ├── transport_lifecycle.rs
    ├── exclusive_lifecycle.rs
    ├── event_source_lifecycle.rs
    ├── daemon_lifecycle.rs
    ├── recovery_gate.rs
    ├── release_queue.rs
    ├── scope_lookup.rs
    ├── config_reload.rs
    └── shutdown_order.rs
```

---

## TopologyRuntime — dispatch enum

```rust
pub enum TopologyRuntime<R: Resource> {
    Pool(pool::Runtime<R>),
    Resident(resident::Runtime<R>),
    Service(service::Runtime<R>),
    Transport(transport::Runtime<R>),
    Exclusive(exclusive::Runtime<R>),
    EventSource(event_source::Runtime<R>),
    Daemon(daemon::Runtime<R>),
}

impl<R: Resource> TopologyRuntime<R> {
    /// Returns shared runtime for topologies that have a single instance.
    /// Pool → None (N instances, managed by idle_queue).
    /// All others → Some(Arc<R::Runtime>).
    pub fn shared_runtime(&self) -> Option<Arc<R::Runtime>> { ... }

    /// Per-topology config reload dispatch.
    /// Pool: update fingerprint (lazy eviction at recycle).
    /// Resident: destroy old → create new (ArcSwap swap).
    /// Service: create new, old drains via Arc refcount.
    /// Transport/Exclusive: destroy old → create new.
    /// EventSource: unsubscribe → resubscribe.
    /// Daemon: cancel → restart with new config.
    pub async fn on_config_changed(&self, new_config: &R::Config) -> Result<()> { ... }
}
```

---

**Removed from original layout (duplicates workspace crates):**
- `resilience/circuit_breaker.rs` → `nebula_resilience::CircuitBreaker`
- `resilience/rate_limiter.rs` → `nebula_resilience::RateLimiter`
- `resilience/retry.rs` → `nebula_resilience::RetryStrategy`
- `resilience/bulkhead.rs` → `nebula_resilience::Bulkhead`
- `policy/layers.rs` → `nebula_resilience::compose::LayerBuilder`
- `metrics.rs` (MetricsSink trait) → `ResourceMetrics` wrapper over `TelemetryAdapter`
- `manager/reload.rs` → `integration/config.rs` (AsyncConfigurable)

---

## Implementation Phases

### Phase 1 — Core primitives (week 1-2)

Foundation. Всё остальное зависит от этого.

```
1.  error.rs           Error, ErrorKind(6), ErrorScope
2.  classify.rs        ClassifyError derive support
3.  ctx.rs             Ctx trait, BasicCtx, Extensions (uses ScopeLevel from nebula-core)
4.  resource.rs        Resource trait (type Config, Runtime, Lease, Error), ResourceConfig trait
5.  metadata.rs        ResourceMetadata
6.  cell.rs            Cell<T> (ArcSwap-based)
7.  state.rs           AtomicRuntimeState
8.  lease/guard.rs     LeaseGuard<L> (pool-internal only)
9.  lease/poison.rs    PoisonToken
10. lease/options.rs   AcquireOptions, AcquireIntent
11. handle.rs          ResourceHandle<R>, HandleInner (3 variants: Owned/Guarded/Shared)
12. release_queue.rs   ReleaseQueue + ReleaseQueueHandle (shared per ManagedResource)
```

**Milestone:** `Error`, `Ctx`, `Resource` (with `type Lease`), `LeaseGuard`, `ResourceHandle` компилируются и тестируются.

### Phase 2 — Topology traits (week 2-3)

Семь trait-ов. Каждый — отдельный файл, отдельные тесты.

```
13. topology/pooled.rs       Pooled (prepare, recycle, is_broken → BrokenCheck)
14. topology/resident.rs     Resident (where Lease: Clone, is_alive, stale_after)
15. topology/service.rs      Service (acquire_token → Lease, release_token)
16. topology/transport.rs    Transport (open_session → Lease, close_session)
17. topology/exclusive.rs    Exclusive (reset — framework-managed)
18. topology/event_source.rs EventSource (type Subscription, subscribe, recv)
19. topology/daemon.rs       Daemon (run with CancellationToken, on_stopped)
```

**Milestone:** все topology traits определены. Можно писать `impl Pooled for Postgres`.

### Phase 3 — Recovery + Integration (week 3)

Recovery primitives, cross-cutting integration.
(ConnectionAware, InfraProvider deferred to v2.)

```
20. recovery/gate.rs                RecoveryGate, GateState, RecoveryTicket
21. recovery/group.rs               RecoveryGroup, RecoveryGroupRegistry
22. recovery/watchdog.rs            WatchdogHandle, WatchdogConfig
23. integration/resilience.rs       AcquireResilience + error mapping (← nebula-resilience)
24. events.rs                       ResourceEvent + ScopedEvent impl (← nebula-eventbus)
25. metrics.rs                      ResourceMetrics wrapper (← nebula-metrics + nebula-telemetry)
```

**Milestone:** RecoveryGate CAS работает. AcquireResilience builds ResilienceChain. ResourceMetrics records counters.

### Phase 4 — Runtime implementations (week 3-4)

Конкретные runtime-ы для каждой topology. Самая большая фаза.

```
28. runtime/pool/             pool::Runtime<R> — full lifecycle
    - config.rs               pool::Config + defaults
    - entry.rs                PoolEntry + InstanceMetrics (3 pub + 3 pub(crate))
    - idle_queue.rs           LIFO/FIFO idle management
    - acquire.rs              checkout + create + prepare (is_broken + is_retryable loop)
    - release.rs              framework policy BEFORE recycle, dispatch via ReleaseQueue
    - maintenance.rs          background loop + memory pressure adaptive sizing

29. runtime/resident/         resident::Runtime<R>
    - config.rs               resident::Config { eager_create }
    - health.rs               is_alive polling loop

30. runtime/service/          service::Runtime<R> (Arc-based, natural drain)
    - config.rs               service::Config

31. runtime/transport/        transport::Runtime<R>
    - config.rs               transport::Config
    - session.rs              session lifecycle, close via ReleaseQueue

32. runtime/exclusive/        exclusive::Runtime<R>
    - config.rs               exclusive::Config
    (Arc<Semaphore> + OwnedSemaphorePermit, HandleInner::Shared)

33. runtime/event_source/     event_source::Runtime<R>
    - config.rs               event_source::Config
    - handle.rs               EventStreamHandle (auto-unsubscribe on drop)

34. runtime/daemon/           daemon::Runtime<R>
    - config.rs               daemon::Config, RestartPolicy
    - runner.rs               run loop + restart (single CancellationToken from framework)

35. runtime/mod.rs            TopologyRuntime<R> enum (7 variants), shared_runtime() → Option<Arc<R::Runtime>>
36. runtime/managed.rs        ManagedResource<R> + AnyManagedResource trait (type erasure with as_any())
```

**Milestone:** Pool lifecycle работает end-to-end. Можно `acquire<Postgres>()`.

### Phase 5 — Manager (week 4-5)

Orchestration layer.

```
37. registry/mod.rs           Registry (DashMap, AnyManagedResource)
38. registry/lookup.rs        ScopeLevel-aware lookup (typed fast path via downcast_ref, erased cold path)
39. registry/scoped.rs        ScopedRuntime (uses ScopeLevel from nebula-core)

40. manager/mod.rs            Manager struct (+TelemetryService, +EventBus<ResourceEvent>, +MemoryMonitor)
41. manager/builder.rs        Typestate RegistrationBuilder (7 finishers, Resident: where Lease: Clone)
42. manager/acquire.rs        acquire → TopologyRuntime dispatch → ResourceHandle
43. manager/shutdown.rs       ShutdownOrchestrator (cancel → reverse topo → drain ReleaseQueues)
    // manager/group.rs, manager/fallback.rs — deferred to v2.

44. integration/config.rs     AsyncConfigurable for Manager (← nebula-config, per-topology reload)
45. integration/memory.rs     Adaptive pool sizing + lookup cache (← nebula-memory)
46. scope.rs                  ResourceScope (capability-based access)
47. dependency.rs             Dependencies trait, static arrays
48. health.rs                 HealthStatus, HealthChecker + EventBus publish
```

**Milestone:** Manager end-to-end. register → acquire → use → drop → recycle. All 7 topologies.

### Phase 6 — Integration + Polish (week 5-6)

Macros, bridges, first resources.

```
51. macros/derive_resource.rs    #[derive(Resource)] macro
52. macros/derive_classify.rs    #[derive(ClassifyError)] macro

53. EventTrigger DX bridge       (in nebula-action crate, on_event uses R::Lease)
54. ResourceAction bridge        (in nebula-action crate)
55. ResourceContext impl          (ActionContext, TriggerContext)

56. Plugin system integration    (PluginRegistry, Descriptors)

57. credential.rs                Credential integration bridge (Authenticate<C> — deferred design)

58. First official resources:
    - nebula-resource-postgres
    - nebula-resource-redis
    - nebula-resource-http

59. Documentation
60. Integration tests
```

**Milestone:** полная система. Plugin install → resource register → action acquire → workflow execution.

---

## Test strategy

```
Unit tests (per module):
  - RecoveryGate state transitions
  - ReleaseQueue parallel workers + ReleaseQueueHandle
  - Cell read/write/take
  - LeaseGuard drop behavior (taint, poison, detach)
  - HandleInner: Owned drop (noop), Guarded drop (on_release), Shared drop (on_release)
  - Scope compatibility + specificity ordering
  - Config fingerprint stability
  - Error classification
  - AcquireResilience chain build
  - ResourceMetrics counter/histogram recording

Integration tests (cross-module):
  - Pool full lifecycle: warmup → acquire → prepare → use → release → recycle → reap
  - Pool acquire with is_broken + is_retryable retry loop
  - Resident: create → clone Lease → stale_after → is_alive → recreate
  - Service: create → acquire_token (Lease) → use → natural drain (Arc refcount)
  - Transport: create → open_session (Lease) → use → close_session via ReleaseQueue
  - Exclusive: create → acquire (Arc + OwnedSemaphorePermit) → use → reset → release
  - EventSource: create → subscribe → recv → unsubscribe
  - Daemon: create → run (CancellationToken) → cancel → restart
  - Recovery: failure → RecoveryGate → probe → resolve → resume
  - Config reload: per-topology strategies (Pool=lazy, Resident/Service=eager, Daemon=restart)
  - Shutdown orchestrator: cancel → reverse order → drain ReleaseQueues → timeout handling
  - Scope lookup: Global < Organization < Project < Workflow < Execution < Action (ScopeLevel, typed + erased paths)
  - AcquireResilience: timeout → retry → circuit breaker → acquire
  - ResilienceError → resource::Error mapping
  - EventBus<ResourceEvent>: register → emit Registered, health change → emit HealthChanged
  - Credential rotation: CredentialEvent::Rotated → pool stale → evict
  - Memory pressure: High → shrink idle, Critical → aggressive shrink

Stress tests:
  - Pool under contention (100 concurrent acquires, max_size=10)
  - ReleaseQueue with heavy recycle (simulate 500ms recycle, 4 workers)
  - RecoveryGate thundering herd (50 concurrent recovery attempts)
  - Resident stale_after with rapid recreation
  - Service natural drain under config reload (Arc refcount → 0)
```
