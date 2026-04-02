# nebula-resource — Architecture

## Problem Statement

Workflow nodes need stable, efficient access to expensive external clients —
database connections, HTTP clients, queue producers, SDK handles.
Recreating those clients per action invocation is prohibitively expensive.
At the same time, multi-tenant execution must enforce isolation: tenant A's
pool must never serve tenant B's execution context.

`nebula-resource` centralises lifecycle management, pool orchestration,
scope enforcement, health monitoring, and observability behind a single
manager API with no business logic leaked into callers.

---

## Key Design Decisions

### 1. `Resource` trait (bb8-style)

A single trait defines the full lifecycle — create, validate, recycle, cleanup.
There are no closure factories. Callers register a value implementing `Resource`
and the pool calls its methods directly with full access to `Config` and `Context`.

This allows lifecycle methods to access config fields, share state via `Arc`
inside the resource struct, and be independently tested without a pool.

### 2. Type erasure at the `Manager` boundary

`Pool<R>` is fully generic over `R: Resource`. The `Manager` stores pools as
`Arc<dyn AnyPool>` behind an `ArcSwap<HashMap<String, PoolEntry>>`. This lets
the manager hold heterogeneous resource types in one registry that is optimised
for read-heavy acquire paths — readers take a snapshot of the `ArcSwap` with no
locking; writes (register, hot-reload) pay one atomic swap.

### 3. RAII guards with taint support

`Guard<T>` holds a checked-out instance and calls a drop-callback on release.
Callers signal that an instance is broken by calling `guard.taint()` before
dropping; the pool then skips recycling and routes the instance to
`Resource::cleanup`. The internal `Poison<T>` primitive guards pool state
across async critical sections — if a future is cancelled mid-operation,
`Poison` marks the value unusable so a subsequent accessor sees the error
immediately rather than observing corrupt state.

### 4. Scope isolation

Every pool is registered under a `Scope`. At acquire time, `Strategy` checks
whether the caller's `Context::scope` is compatible with the pool's scope.
`Strategy::Hierarchical` (default) allows a broader-scope pool to serve a
narrower caller: a `Global` pool serves `Tenant`, `Workflow`, and `Execution`
contexts. `Strategy::Strict` requires an exact scope match.

### 5. `EventBus` for observability decoupling

All lifecycle transitions emit a `ResourceEvent` on the crate-local `EventBus`
(backed by `nebula-eventbus`). Upstream crates subscribe independently — no
callback registration, no circular imports. Each subscriber gets its own
`broadcast::Receiver`; slow subscribers are isolated by the backpressure policy
configured on the bus and never stall the acquire path.

### 6. Cancellation everywhere

`Context` carries a `CancellationToken`. Pool acquire, health checks, and
auto-scale loops all branch on cancellation via `tokio::select!`. Dropping the
token tree propagates shutdown to every in-flight operation without explicit
signal passing.

---

## Module Map

```
nebula-resource/src/
│
│  ── Public API ────────────────────────────────────────────────────────────
│
├── resource.rs         Resource + Config traits.
│                       No runtime deps — usable without tokio.
│
├── lifecycle.rs        Lifecycle enum:
│                         Created → Initializing → Ready → InUse →
│                         Idle → Maintenance → Draining → Cleanup →
│                         Terminated | Failed
│                       State machine validated by can_transition_to().
│
├── guard.rs            Guard<T, F> — RAII acquire handle.
│                       taint() marks instance for cleanup on drop (skips recycle).
│                       into_inner() extracts the value without invoking the callback.
│
├── context.rs          Context — scope, workflow_id, execution_id, tenant_id,
│                       cancellation token, arbitrary metadata, telemetry Recorder.
│                       ResourcePoolHandle<R> — typed handle for direct pool
│                       access stored inside Context by the engine.
│
├── scope.rs            Scope enum (Global / Tenant / Workflow / Execution /
│                       Action / Custom).
│                       Strategy (Strict / Hierarchical / Fallback).
│                       hierarchy_level() and contains() power Strategy::is_compatible.
│
├── pool.rs             Pool<R> — bounded semaphore pool.
│                         PoolConfig, PoolStrategy, PoolBackpressurePolicy,
│                         AdaptiveBackpressurePolicy, PoolStats, LatencyPercentiles.
│                       Internal: Gate/GateGuard, CounterGuard (RAII helpers).
│
├── manager.rs          Manager — ArcSwap registry of Arc<dyn AnyPool>.
│                       ManagerBuilder — preferred construction path.
│                       ShutdownConfig (drain / cleanup / terminate timeouts).
│                       ResourceStatus, ResourcePoolStatus (observability snapshots).
│                       Re-exports: DependencyGraph, AnyGuard, TypedResourceGuard,
│                         AnyGuardTrait, ResourceHandle, TypedPool.
│
├── health.rs           HealthState, HealthStatus, HealthRecord.
│                       HealthCheckable trait.
│                       HealthChecker — background Tokio task per monitored instance.
│                       HealthStage, HealthPipeline — composable multi-stage checks.
│                       ConnectivityStage, PerformanceStage — built-in stages.
│                       ResourceHealthAdapter — wraps Resource as a health probe.
│
├── quarantine.rs       QuarantineManager — isolates unhealthy resources.
│                       QuarantineEntry, QuarantineReason, RecoveryStrategy
│                       (exponential backoff: base_delay * multiplier^attempt,
│                        capped at max_delay).
│
├── events.rs           EventBus (thin wrapper around nebula-eventbus).
│                       ResourceEvent enum — full lifecycle event catalog.
│                       CleanupReason, QuarantineTrigger.
│                       Re-exports from nebula-eventbus:
│                         BackPressurePolicy, EventFilter, EventSubscriber,
│                         ScopedSubscriber, SubscriptionScope, PublishOutcome.
│
├── hooks.rs            HookRegistry — ordered pre/post callbacks for
│                       acquire, release, create, cleanup.
│                       ResourceHook trait. HookEvent, HookFilter, HookResult.
│                       Built-ins: AuditHook (priority 10),
│                                  SlowAcquireHook (priority 90).
│
├── autoscale.rs        AutoScalePolicy (high/low watermarks, step sizes, cooldown).
│                       AutoScaler — Tokio task that polls utilisation and calls
│                       caller-provided scale_up / scale_down closures.
│                       AutoScalerHandle — returned by AutoScaler::start(); exposes
│                       shutdown().await (graceful) and cancel() (fire-and-forget).
│
├── metadata.rs         ResourceMetadata — display name, description, icon,
│                       icon_url, tags. ResourceMetadataBuilder.
│
├── reference.rs        ResourceRef<R> — typed wrapper around ResourceKey.
│                       ErasedResourceRef — type-erased version for storage.
│                       ResourceProvider trait — typed and dynamic acquire.
│
├── error.rs            Error (non-exhaustive), ErrorCategory
│                       (Retryable / Fatal / Validation), FieldViolation.
│                       Implements nebula_resilience::Retryable.
│
├── poison.rs           Poison<T> — arm/disarm guard for async critical sections.
│                       PoisonGuard — RAII disarm. PoisonError — access attempt
│                       on a poisoned value.
│
├── components.rs       HasResourceComponents — declares credential and
│                       sub-resource dependencies for a Resource type.
│                       ResourceComponents builder. TypedCredentialHandler<I>.
│
├── instrumented.rs     InstrumentedGuard — wraps AnyGuard, records usage via
│                       Recorder on drop (DropReason: Released / Panic / Detached).
│
├── metrics.rs          MetricsCollector — counter/histogram bridge to nebula-metrics.
│
│  ── Internal ──────────────────────────────────────────────────────────────
│
├── dependency_graph.rs DependencyGraph — directed graph for topological
│                       startup/shutdown ordering. Cycle detection via DFS.
│                       Re-exported publicly on Manager.
│
├── manager_guard.rs    AnyGuardTrait, AnyGuard, TypedResourceGuard<R>,
│                       ResourceHandle — type-erased guard layer returned
│                       by Manager to callers.
│                       ReleaseHookGuard — runs after-release hooks on drop.
│
└── manager_pool.rs     TypedPool<R>, AnyPool (object-safe pool trait),
                        PoolEntry, RotatablePool (credential rotation dispatch).
                        The manager's private pool wrappers.
```

---

## Data Flow

### Acquire path

```
Caller
  │  manager.acquire(&key, &ctx)
  ▼
Manager
  │  1. Lock-free snapshot of ArcSwap registry
  │  2. Look up PoolEntry by ResourceKey
  │  3. Strategy::is_compatible(pool.scope, ctx.scope)
  │     └─ incompatible → Error::Unavailable
  │  4. HookRegistry::run_before(HookEvent::Acquire, ...)
  │     └─ HookResult::Cancel → Error (from hook)
  ▼
Pool<R>::acquire_inner(&ctx)
  │  5. CounterGuard increments waiter count (RAII)
  │  6. Semaphore permit per PoolBackpressurePolicy:
  │     ├─ FailFast    → try_acquire; error if unavailable
  │     ├─ BoundedWait → acquire with fixed timeout
  │     └─ Adaptive    → choose timeout from utilisation + waiter stats
  │  7. Pop idle instance from VecDeque:
  │     ├─ Fifo: pop_front  (oldest idle instance)
  │     └─ Lifo: pop_back   (most recently used)
  │  8. Call Resource::is_reusable on idle instance:
  │     └─ false / Err → Resource::cleanup, try next idle instance
  │  9. No idle instance → Resource::create (circuit-breaker guarded)
  │ 10. Record acquire latency in HDR histogram
  │ 11. EventBus::emit(Acquired { wait_duration })
  │ 12. Wrap instance in Guard<T> with on_drop = pool release callback
  ▼
Manager wraps Guard in InstrumentedGuard
  │ 13. Attach ctx.recorder() for per-call telemetry
  ▼
TypedResourceGuard<R> returned to caller (Deref<Target = R::Instance>)
```

### Release path (Guard drop)

```
TypedResourceGuard<R> drops
  │
InstrumentedGuard::drop
  │  1. Emit CallRecord via Recorder (DropReason::Released or Panic)
  ▼
Guard<T>::drop (invokes on_drop callback)
  │  if tainted:
  │    Resource::cleanup(instance)
  │    EventBus::emit(CleanedUp { reason: Tainted })
  │  else:
  │    Resource::recycle(&mut instance)  (circuit-breaker guarded)
  │    VecDeque::push_back (Lifo) or push_front (Fifo) back to idle queue
  │    Semaphore::add_permits(1)
  │    EventBus::emit(Released { usage_duration })
  ▼
ReleaseHookGuard::drop
  │  HookRegistry::run_after(HookEvent::Release, ..., success)
```

### Health and quarantine state machine

```
HealthChecker — background Tokio task spawned per monitored instance
  │
  │  polls Resource::health_check() at HealthCheckConfig::default_interval
  │  (timeout: HealthCheckConfig::check_timeout)
  │
  ├─ consecutive_failures < failure_threshold
  │    └─ update HealthRecord; emit HealthChanged if state differs
  │
  ├─ consecutive_failures >= failure_threshold
  │    └─ QuarantineManager::quarantine(resource_id, QuarantineReason::HealthCheckFailed)
  │         └─ EventBus::emit(Quarantined { trigger, from_health, to_health })
  │
  └─ QuarantineEntry::next_recovery_at reached
       └─ Recovery probe:
            Resource::create → Resource::is_reusable → Resource::cleanup
            ├─ success → QuarantineManager::release(resource_id)
            │              EventBus::emit(QuarantineReleased { recovery_attempts })
            └─ failure → QuarantineEntry::record_failed_recovery
                           ├─ attempts < max → schedule next probe
                           └─ exhausted     → log permanent failure
```

---

## Dependency Graph

```
nebula-resource
  ├── nebula-core         ResourceKey, ResourceId, ExecutionId, WorkflowId,
  │                       CredentialId, CredentialKey, PluginKey
  ├── nebula-credential   CredentialType, ErasedCredentialRef, RotationStrategy
  ├── nebula-eventbus     EventBus, BackPressurePolicy, PublishOutcome,
  │                       EventFilter, SubscriptionScope
  ├── nebula-metrics      counter/histogram bridge
  ├── nebula-telemetry    Recorder, CallRecord, DropReason, ResourceUsageRecord
  └── nebula-resilience   CircuitBreaker, CircuitBreakerConfig, Gate, Retryable
```

No upward dependencies. `nebula-api`, `nebula-engine`, and adapter crates
depend on `nebula-resource`; this crate never imports them.

---

## Invariants

1. **Semaphore permits == idle + active.**
   `Pool` acquires one permit per checked-out instance and releases one permit
   on Guard drop. `Poison<T>` ensures a cancelled future does not strand a permit.

2. **Config is validated before the pool is created.**
   `Manager::register` calls `Config::validate` and returns `Error::Validation`
   before constructing a `Pool`.

3. **Resource key is unique per Manager.**
   Registering the same `ResourceKey` twice returns `Error::Configuration`.
   Hot-reload via `Manager::reload_config` is the only path to update an
   existing pool's config.

4. **Events are best-effort, never blocking.**
   `EventBus::emit` is fire-and-forget. Slow subscribers are isolated by the
   backpressure policy and never stall the acquire path.

5. **Circuit breakers are per-operation, not per-pool.**
   Each `Pool<R>` has independent circuit breakers for `create` and `recycle`.
   A failing `recycle` does not block new `create` calls.

6. **No unsafe code.**
   The crate is compiled with `#![forbid(unsafe_code)]`.
