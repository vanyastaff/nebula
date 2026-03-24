# 07 — Module Layout & Implementation Plan

> **Status alignment:** resource stabilization is "Next Up" in active-work.md.
> Engine execution is blocked on this crate stabilizing first.

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
    ├── credential_rotation.rs
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

    /// Per-topology config reload dispatch. Returns ReloadOutcome.
    /// Pool: update fingerprint (lazy eviction at recycle) → SwappedImmediately.
    /// Resident: destroy old → create new (ArcSwap swap) → SwappedImmediately.
    /// Service: create new, old drains via Arc refcount → PendingDrain.
    /// Transport/Exclusive: destroy old → create new → SwappedImmediately.
    /// EventSource: unsubscribe → resubscribe → SwappedImmediately.
    /// Daemon: cancel → restart with new config → Restarting.
    pub async fn on_config_changed(&self, new_config: &R::Config) -> Result<ReloadOutcome> { ... }
}

/// Result of a config reload operation. Returned by on_config_changed().
/// Engine can display this in UI ("applied immediately" vs "draining old connections").
pub enum ReloadOutcome {
    /// New config applied immediately. All new acquires use updated config.
    SwappedImmediately,
    /// Old runtime draining (Arc refcount). New runtime active for new acquires.
    /// old_generation tracks which handles are stale.
    PendingDrain { old_generation: u64 },
    /// Runtime restarting (Daemon). Brief unavailability expected.
    Restarting,
    /// Config unchanged (fingerprint identical). No action taken.
    NoChange,
}
```

---

## ManagedResource — generation tracking and status

```rust
pub struct ManagedResource<R: Resource> {
    // ... existing fields ...

    /// Monotonically increasing generation counter. Incremented on every
    /// reload/recreate. Used for stale handle detection and drain tracking.
    generation: AtomicU64,

    /// Operational status. Updated by lifecycle operations.
    status: ArcSwap<ResourceStatus>,
}

/// Simplified operational status (v1).
/// Full Kubernetes-style conditions set deferred to v2.
pub struct ResourceStatus {
    /// Current lifecycle phase.
    pub phase: ResourcePhase,
    /// Current generation (matches ManagedResource::generation).
    pub generation: u64,
    /// Last error encountered (create failure, check failure, etc.).
    pub last_error: Option<Arc<Error>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcePhase {
    /// Resource registered, not yet created (warmup pending).
    Initializing,
    /// Runtime created and healthy. Normal operation.
    Ready,
    /// Config reload in progress. New runtime being created or old draining.
    Reloading,
    /// Old runtime draining (Service Arc refcount, Pool stale eviction).
    Draining,
    /// Graceful shutdown in progress.
    ShuttingDown,
    /// Create/check failed. May recover via RecoveryGate.
    Failed,
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
4.  resource.rs        Resource trait (type Config, Runtime, Lease, Error, Credential), ResourceConfig trait
5.  credential.rs      Credential trait re-export, minimal bridge to nebula-credential
6.  metadata.rs        ResourceMetadata
7.  cell.rs            Cell<T> (ArcSwap-based)
8.  state.rs           AtomicRuntimeState, ResourceStatus, ResourcePhase
9.  lease/guard.rs     LeaseGuard<L> (pool-internal only)
10. lease/poison.rs    PoisonToken
11. lease/options.rs   AcquireOptions, AcquireIntent
12. handle.rs          ResourceHandle<R>, HandleInner (3 variants: Owned/Guarded/Shared)
                       Guarded/Shared store generation: u64 at acquire time for stale detection
13. release_queue.rs   ReleaseQueue + ReleaseQueueHandle (shared per ManagedResource)
```

**Milestone:** `Error`, `Ctx`, `Resource` (with `type Lease`, `type Credential`), `LeaseGuard`, `ResourceHandle` компилируются и тестируются.

### Phase 2 — Topology traits (week 2-3)

Семь trait-ов. Каждый — отдельный файл, отдельные тесты.

```
14. topology/mod.rs           TopologyKind enum, re-exports
15. topology/pooled.rs        Pooled (prepare, recycle, is_broken → BrokenCheck)
16. topology/resident.rs      Resident (where Lease: Clone, is_alive_sync, stale_after)
17. topology/service.rs       Service (acquire_token → Lease, release_token)
18. topology/transport.rs     Transport (open_session → Lease, close_session)
19. topology/exclusive.rs     Exclusive (reset — framework-managed)
20. topology/event_source.rs  EventSource (type Subscription, subscribe, recv)
21. topology/daemon.rs        Daemon (run with CancellationToken, on_stopped)
```

**Milestone:** все topology traits определены. Можно писать `impl Pooled for Postgres`.

### Phase 3 — Recovery + Integration (week 3)

Recovery primitives, cross-cutting integration.
(ConnectionAware, InfraProvider deferred to v2.)

```
22. recovery/gate.rs                RecoveryGate, GateState, RecoveryTicket
23. recovery/group.rs               RecoveryGroup, RecoveryGroupRegistry
24. recovery/watchdog.rs            WatchdogHandle, WatchdogConfig
25. integration/resilience.rs       AcquireResilience + error mapping (← nebula-resilience)
26. events.rs                       ResourceEvent + ScopedEvent impl (← nebula-eventbus)
27. metrics.rs                      ResourceMetrics wrapper (← nebula-metrics + nebula-telemetry)
```

**Milestone:** RecoveryGate CAS работает. AcquireResilience builds ResilienceChain. ResourceMetrics records counters. CredentialStore is object-safe (resolve_erased + CredentialStoreExt blanket). ScopeResolver is object-safe (BoxFuture, amendment #20).

### Phase 4 — Runtime implementations (week 3-4)

Конкретные runtime-ы для каждой topology. Самая большая фаза.

```
28. runtime/pool/             pool::Runtime<R> — full lifecycle
    - config.rs               pool::Config + defaults
    - entry.rs                PoolEntry + InstanceMetrics (3 pub + 3 pub(crate))
    - idle_queue.rs           LIFO/FIFO idle management
    - acquire.rs              checkout + create + prepare (is_broken + is_retryable loop)
                              Retry blacklist: SmallVec<[InstanceId; 4]> tracks attempted
                              instances within one acquire cycle — prevents re-checkout of
                              the same broken instance. See amendment #7.
    - release.rs              framework policy BEFORE recycle, dispatch via ReleaseQueue
    - maintenance.rs          background loop + memory pressure adaptive sizing
                              Staggered probe: when CheckPolicy::Interval, checks are spread
                              across the interval (delay_per_instance = interval / idle_count)
                              to avoid thundering herd of health checks to the backend.

29. runtime/resident/         resident::Runtime<R>
    - config.rs               resident::Config { eager_create }
    - health.rs               is_alive_sync polling loop (O(1), no I/O — see contracts)

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
                              debug_assert!(state.runtime.is_some()) at loop top —
                              catches if recreate failed but loop was not broken.

35. runtime/mod.rs            TopologyRuntime<R> enum (7 variants), shared_runtime()
36. runtime/managed.rs        ManagedResource<R> + AnyManagedResource trait (type erasure)
                              Includes: generation: AtomicU64, status: ArcSwap<ResourceStatus>
                              HandleInner::Guarded/Shared store generation at acquire time
                              for stale handle detection during reload/drain.
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
47. dependency.rs             Dependencies trait, static TypeId arrays
48. health.rs                 HealthStatus, HealthChecker + EventBus publish
```

**Dependencies:** Phase 5 requires Phase 4 (runtime implementations) and Phase 3 (recovery, events, metrics).
`integration/config.rs` (44) depends on `TopologyRuntime::on_config_changed` (35).
`manager/shutdown.rs` (43) depends on `ReleaseQueue` (13) and `recovery/gate.rs` (22).

**Milestone:** Manager end-to-end. register → acquire → use → drop → recycle. All 7 topologies.

### Phase 6 — Integration + Polish (week 5-6)

Macros, bridges, first resources.

```
49. macros/derive_resource.rs    #[derive(Resource)] macro
50. macros/derive_classify.rs    #[derive(ClassifyError)] macro

51. EventTrigger DX bridge       (in nebula-action crate, on_event uses R::Lease)
52. ResourceAction bridge        (in nebula-action crate)
53. ResourceContext impl          (ActionContext, TriggerContext)

54. Plugin system integration    (PluginRegistry, Descriptors)

55. First official resources:
    - nebula-resource-postgres
    - nebula-resource-redis
    - nebula-resource-http

56. testing/ module (behind `test-support` feature flag):
    - testing/mod.rs              pub use, #[cfg(feature = "test-support")]
    - testing/mock_provider.rs    TestContext::inject::<R>(mock_handle)
    - testing/contract.rs         resource_contract_tests!() macro —
                                  reusable test suite verifying Resource+Topology impls:
                                  create→acquire→release→shutdown happy path,
                                  concurrent acquire/release, cancellation during acquire,
                                  drop path (panic, abort, dropped future)
    - testing/fault.rs            FaultInjector — inject transient/permanent errors
    - testing/harness.rs          TestManager, cancel/drop stress helpers,
                                  Exclusive semaphore cancel-safety tests

    # Cargo.toml:
    # [features]
    # test-support = ["tokio/test-util"]
    #
    # Downstream usage:
    # [dev-dependencies]
    # nebula-resource = { workspace = true, features = ["test-support"] }

57. Documentation
58. Integration tests
```

Note: `credential.rs` moved to Phase 1 (item 5) as a core primitive.
Authenticate<C> design deferred — credential rotation handled via EventBus<CredentialEvent> in integration/config.rs.

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
  - Pool acquire retry blacklist: broken instance not re-checked in same acquire cycle
  - Resident: create → clone Lease → stale_after → is_alive_sync → recreate
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
  - Credential rotation: CredentialEvent::Rotated → pool stale → evict at recycle
  - Hot-reload: config change → TopologyRuntime::on_config_changed → verify per-topology strategy
  - Memory pressure: High → shrink idle, Critical → aggressive shrink

Stress tests:
  - Pool under contention (100 concurrent acquires, max_size=10)
  - ReleaseQueue with heavy recycle (simulate 500ms recycle, 4 workers)
  - RecoveryGate thundering herd (50 concurrent recovery attempts)
  - Resident stale_after with rapid recreation
  - Service natural drain under config reload (Arc refcount → 0)
  - ReloadOutcome: verify each topology returns correct variant
  - Generation tracking: stale handle detection after reload
  - ResourceStatus phase transitions: Initializing → Ready → Reloading → Ready
```

---

## v2 Architectural Directions

Recorded from multi-agent brainstorm. Not for v1 — design exploration for future.

- **Topology decomposition into orthogonal axes:**
  Access mode (Pool/Resident/Exclusive) × Provisioning (Sessioned/Tokenized/Direct) × Side-channel (EventSource/Daemon/None).
  Current 7 traits work but mix these dimensions. v2 may decompose for composability.

- **ResourceHooks trait:** Extensible lifecycle hooks (post_create, pre_checkout, post_release, pre_shutdown). v1 covers this through prepare()/recycle()/shutdown(). Hooks useful for observability wrappers.

- **ResourceConditions:** Full Kubernetes-style conditions set on ResourceStatus (multiple simultaneous conditions with timestamps). v1 uses simplified phase + last_error.

- **Credential pre-expiry refresh:** Proactive token refresh before TTL expiry (OAuth, cloud tokens). v1 is reactive (EventBus<CredentialRotatedEvent>).

- **Cascading reload propagation:** Reload Postgres → temporarily degrade dependent resources. v1 reloads are independent per-resource.

- **Time virtualization for testing:** Virtual clock for deterministic timeout/backoff tests. v1 uses tokio::time::pause().

- **Bounded ReleaseQueue fallback:** Replace current bounded(10k) with configurable max_fallback_capacity + MarkLeakedAndScheduleRecovery policy. v1.1 candidate.
