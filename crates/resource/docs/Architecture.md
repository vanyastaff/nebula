# nebula-resource Architecture

## Overview

`nebula-resource` provides resource lifecycle management for the Nebula workflow
engine. It manages the creation, pooling, health checking, scoping, and cleanup
of external resources (database connections, HTTP clients, message queues, etc.)
used by workflow actions.

Key design decisions:

- **bb8-style `Resource` trait** -- a single trait defines the full lifecycle
  (create, validate, recycle, cleanup) with associated `Config` and `Instance`
  types. No closure factories.
- **Type erasure at the Manager boundary** -- `Pool<R>` is fully generic, but
  `Manager` stores pools as `Arc<dyn AnyPool>` so it can hold heterogeneous
  resource types in one `DashMap`.
- **RAII guards** -- `Guard<T>` returns instances to the pool on drop. The
  manager wraps these in `AnyGuard` (type-erased) and `ResourceHandle`
  (downcasting API for consumers).
- **Feature-gated modules** -- the core types (`Resource`, `Guard`, `Scope`,
  `Context`, `Lifecycle`) require no async runtime. The `tokio` feature gates
  all pool, manager, health, events, hooks, quarantine, and autoscale modules.
- **Cancellation everywhere** -- `Context` carries a `CancellationToken`.
  Pool acquire, health checks, and autoscale all respect cancellation via
  `tokio::select!`.

## Module Map

```
nebula-resource/src/
|
|-- lib.rs              Re-exports, feature gates, prelude
|
|-- resource.rs         Resource trait, Config trait (no async runtime needed)
|-- lifecycle.rs        Lifecycle state machine (Created -> ... -> Terminated)
|-- guard.rs            Guard<T> RAII wrapper with drop callback
|-- context.rs          Context (scope, IDs, cancellation, metadata, credentials)
|-- scope.rs            Scope enum (Global/Tenant/Workflow/Execution/Action/Custom)
|                       Strategy enum (Strict/Hierarchical/Fallback)
|-- error.rs            Error enum, FieldViolation, Result alias
|
|-- [tokio] pool.rs          Pool<R>, PoolConfig, PoolStrategy, PoolStats
|-- [tokio] manager.rs       Manager, DependencyGraph, AnyGuard, ResourceHandle
|-- [tokio] events.rs        EventBus (broadcast), ResourceEvent variants
|-- [tokio] health.rs        HealthChecker, HealthPipeline, HealthStage trait,
|                             HealthCheckable trait, built-in stages
|-- [tokio] hooks.rs         HookRegistry, ResourceHook trait, AuditHook,
|                             SlowAcquireHook
|-- [tokio] quarantine.rs    QuarantineManager, RecoveryStrategy, backoff
|-- [tokio] autoscale.rs     AutoScaler, AutoScalePolicy (watermark-based)
|-- [tokio+metrics] metrics.rs  MetricsCollector (EventBus subscriber)
|
|-- [credentials] credentials.rs  CredentialProvider trait, SecureString
```

Module dependency flow:

```
                    resource.rs
                    lifecycle.rs
                    guard.rs
                    context.rs ---------> scope.rs
                    error.rs                 |
                        |                    |
      +-----------------+--------------------+
      |                 |
      v                 v
  pool.rs --------> events.rs
      |                 |
      v                 v
  manager.rs <---- health.rs
      |                 |
      +-----+-----------+
      |     |           |
      v     v           v
  hooks.rs  quarantine.rs  autoscale.rs
                            |
                            v
                        metrics.rs (subscribes to EventBus)
```

## Core Abstractions

### Resource trait (`resource.rs`)

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: Config;
    type Instance: Send + Sync + 'static;

    fn id(&self) -> &str;
    fn create(&self, config: &Self::Config, ctx: &Context) -> impl Future<Output = Result<Self::Instance>> + Send;
    fn is_valid(&self, instance: &Self::Instance) -> impl Future<Output = Result<bool>> + Send;
    fn recycle(&self, instance: &mut Self::Instance) -> impl Future<Output = Result<()>> + Send;
    fn cleanup(&self, instance: Self::Instance) -> impl Future<Output = Result<()>> + Send;
    fn dependencies(&self) -> Vec<&str>;
}
```

No `async_trait` -- uses `impl Future` in return position (Rust 2024 RPITIT).
Default implementations return `Ok(true)` for `is_valid`, `Ok(())` for
`recycle` and `cleanup`, and empty `Vec` for `dependencies`.

### Pool (`pool.rs`)

`Pool<R>` manages a bounded set of `R::Instance` objects. Internally:

- **`PoolInner<R>`** holds `Arc<R>`, `R::Config`, `PoolConfig`, a `Mutex<PoolState<T>>`
  (idle queue + stats + shutdown flag), a `Semaphore` (limits concurrent active
  instances), a `CancellationToken`, and an optional `EventBus`.
- **`PoolConfig`** controls: `min_size`, `max_size`, `acquire_timeout`,
  `idle_timeout`, `max_lifetime`, `validation_interval`, `maintenance_interval`,
  and `strategy` (FIFO or LIFO).
- **`PoolStrategy`**: `Fifo` (even distribution, default) or `Lifo` (hot
  working set, lets cold instances expire).
- **`PoolStats`**: counters for acquisitions, releases, active, idle, created,
  destroyed.

Acquire flow within the pool:

1. Acquire a semaphore permit (with `acquire_timeout`).
2. Pop an idle entry (FIFO or LIFO per strategy).
3. If expired -- cleanup and retry from step 2.
4. If present -- validate via `is_valid()`. Invalid entries are cleaned up.
5. If no idle entries -- create a new instance via `Resource::create()`.
6. Track `created_at` on the entry so lifetime expiration works correctly.
7. Wrap in `Guard<R::Instance>` whose drop callback spawns `return_instance()`.

Release flow (`return_instance`):

1. Call `Resource::recycle()` on the instance.
2. If recycle succeeds and pool is not shut down, push back as idle `Entry`.
3. Otherwise call `Resource::cleanup()` and destroy.
4. Update stats, emit Released event, return semaphore permit.

Optional background maintenance task (if `maintenance_interval` is set) runs
`maintain()` on a timer with cancellation support.

### Manager (`manager.rs`)

`Manager` is the central registry. It stores:

| Field            | Type                                    | Purpose                              |
|------------------|-----------------------------------------|--------------------------------------|
| `pools`          | `DashMap<String, PoolEntry>`            | Type-erased pools keyed by ID        |
| `deps`           | `RwLock<DependencyGraph>`               | Initialization/shutdown ordering     |
| `health_checker` | `Arc<HealthChecker>`                    | Background instance monitoring       |
| `event_bus`      | `Arc<EventBus>`                         | Lifecycle event broadcasting         |
| `quarantine`     | `QuarantineManager`                     | Unhealthy resource isolation         |
| `health_states`  | `DashMap<String, HealthState>`          | Per-resource health cache            |
| `hooks`          | `HookRegistry`                          | Lifecycle hooks                      |

Key methods:

- **`register<R>(resource, config, pool_config)`** -- creates a `Pool<R>`,
  wraps it in `TypedPool` -> `Arc<dyn AnyPool>`, validates dependencies against
  the graph (clone-validate-swap for atomicity), emits `Created` event.
- **`register_scoped<R>(..., scope)`** -- same, with explicit scope.
- **`acquire(resource_id, ctx)`** -- checks quarantine, health state, scope
  compatibility, runs before-hooks, delegates to pool, runs after-hooks, emits
  events.
- **`shutdown_scope(scope)`** -- collects pools within the scope, shuts them
  down in reverse topological order to respect dependencies.

**Type erasure chain:**

```
Pool<R>::acquire(ctx) -> Guard<R::Instance>
  |
  v  (wrapped by TypedPool)
TypedGuard<R> { guard: Guard<R::Instance> }
  |
  v  (boxed as)
AnyGuard = Box<dyn AnyGuardTrait>
  |
  v  (wrapped for consumer API)
ResourceHandle { guard: AnyGuard }
  |-- get::<T>() -> Option<&T>     (downcast via Any)
  |-- get_mut::<T>() -> Option<&mut T>
```

### DependencyGraph (`manager.rs`)

Directed graph with two adjacency lists (`dependencies` and `dependents`).
Supports:

- `add_dependency(resource, depends_on)` -- rejects cycles (DFS-based).
- `topological_sort()` -- Kahn's algorithm for initialization order.
- `get_init_order(resource)` -- transitive dependencies for one resource.
- `remove_all_for(resource)` -- clean removal on re-registration.

### Guard (`guard.rs`)

```rust
pub struct Guard<T> {
    resource: Option<T>,
    on_drop: Option<Box<dyn FnOnce(T) + Send>>,
}
```

Implements `Deref`, `DerefMut`, and `Drop`. The drop callback typically spawns
an async task to return the instance to the pool. `into_inner()` consumes the
guard without triggering the callback.

### Context (`context.rs`)

Flat struct carrying all per-operation context:

- `scope: Scope` -- visibility scope for this operation.
- `execution_id`, `workflow_id`, `tenant_id` -- identity.
- `cancellation: CancellationToken` -- cooperative cancellation.
- `metadata: HashMap<String, String>` -- arbitrary key-value pairs.
- `credentials: Option<Arc<dyn CredentialProvider>>` -- (feature-gated).

Builder pattern via `with_tenant()`, `with_metadata()`, `with_cancellation()`,
`with_credentials()`.

### Scope (`scope.rs`)

Hierarchical visibility model:

```
Global (level 0)
  |-- Tenant { tenant_id } (level 1)
       |-- Workflow { workflow_id, tenant_id? } (level 2)
            |-- Execution { execution_id, workflow_id?, tenant_id? } (level 3)
                 |-- Action { action_id, execution_id?, workflow_id?, tenant_id? } (level 4)
Custom { key, value } (level 5)
```

`Scope::contains(other)` checks parent chain consistency with deny-by-default
semantics: if the parent specifies a field but the child does not, containment
is denied. This prevents scope leaks.

`Strategy` controls how scope matching works during acquire:

| Strategy       | Behavior                                              |
|----------------|-------------------------------------------------------|
| `Strict`       | Exact scope match only                                |
| `Hierarchical` | Resource scope must contain caller scope (default)    |
| `Fallback`     | Exact match first, then fall back to containment      |

### Lifecycle (`lifecycle.rs`)

State machine enum with 10 states:

```
Created -> Initializing -> Ready -> InUse -> Idle
                             |        |       |
                             v        v       v
                          Draining  Ready  Maintenance
                             |                |
                             v                v
                          Cleanup          Cleanup -> Terminated
                             |
                             v
                          Terminated

(Any state) -> Failed -> Cleanup -> Terminated
```

`can_transition_to(target)` enforces valid transitions. `is_available()` returns
true for `Ready` and `Idle`. `is_terminal()` returns true for `Terminated` and
`Failed`.

## Cross-Cutting Concerns

### Events (`events.rs`)

`EventBus` wraps `tokio::sync::broadcast`. Fire-and-forget: no backpressure on
emitters. Subscribers receive cloned `ResourceEvent` variants.

Event variants: `Created`, `Acquired`, `Released`, `HealthChanged`,
`PoolExhausted`, `CleanedUp`, `Quarantined`, `QuarantineReleased`, `Error`.

### Hooks (`hooks.rs`)

`HookRegistry` stores `Vec<Arc<dyn ResourceHook>>` sorted by priority (lower
runs first), protected by `RwLock`.

`ResourceHook` trait:
- `before()` can return `HookResult::Cancel(Error)` to abort the operation.
- `after()` is always called (success or failure), errors logged but never
  propagated.
- `filter()` controls which resources the hook applies to (`All`, `Resource(id)`,
  `Prefix(prefix)`).

Built-in hooks:
- **`AuditHook`** (priority 10) -- logs all events via `tracing::info!`.
- **`SlowAcquireHook`** (priority 90) -- warns when acquire exceeds a duration
  threshold. Stores timers in a `Mutex<HashMap>` keyed by resource+execution.

### Health (`health.rs`)

Three layers:

1. **`HealthCheckable` trait** -- per-instance health check with configurable
   interval and timeout.
2. **`HealthChecker`** -- background monitor. Spawns a per-instance tokio task
   that runs health checks at intervals. Tracks `HealthRecord` per instance in
   a `DashMap`. Emits `HealthChanged` events on state transitions. Uses
   per-instance child `CancellationToken`s for clean shutdown.
3. **`HealthPipeline` + `HealthStage`** -- composable pipeline of stages.
   Stages run in order; short-circuits on `Unhealthy`. Returns worst status.

Built-in stages:
- **`ConnectivityStage<F>`** -- user-provided async probe function.
- **`PerformanceStage`** -- latency threshold checks (`warn_threshold`,
  `fail_threshold`) with optional probe function.

Health states: `Healthy`, `Degraded { reason, performance_impact }`,
`Unhealthy { reason, recoverable }`, `Unknown`. `HealthStatus::score()` maps
to 0.0-1.0. `is_usable()` allows Degraded up to 0.8 impact.

### Quarantine (`quarantine.rs`)

`QuarantineManager` isolates unhealthy resources. Thread-safe via `DashMap`.

- Resources are quarantined when consecutive health failures exceed
  `failure_threshold`.
- Recovery attempts use exponential backoff via `RecoveryStrategy`
  (`base_delay`, `max_delay`, `multiplier`).
- `QuarantineEntry` tracks attempts, scheduled next retry, and exhaustion.
- Manager's `acquire()` checks quarantine status before pool access -- returns
  `Error::Unavailable { retryable: true }` for quarantined resources.

### AutoScale (`autoscale.rs`)

`AutoScaler` monitors pool utilization via watermark thresholds.

- Decoupled from `Pool` via closures (`get_stats`, `scale_up`, `scale_down`).
- Checks utilization every `evaluation_window / 2`.
- Scales up when utilization exceeds `high_watermark` (default 0.8) for
  `evaluation_window` (default 30s).
- Scales down when utilization drops below `low_watermark` (default 0.2) for
  the same window.
- Respects a `cooldown` (default 60s) between scale operations.
- Cancelled via `CancellationToken`.

`Pool<R>` exposes `scale_up(count)` and `scale_down(count)` methods that the
scaler callbacks can call.

### Metrics (`metrics.rs`)

`MetricsCollector` subscribes to the `EventBus` and translates events into
counters, gauges, and histograms via the `metrics` crate. Gated behind the
`metrics` feature (requires `tokio` as well). Runs as a background task until
the event bus is dropped.

## Feature Gates

| Feature        | Modules Enabled                                           | Dependencies Added                        |
|----------------|-----------------------------------------------------------|-------------------------------------------|
| `std` (default)| --                                                        | tokio/rt, tracing/std                     |
| `tokio` (default)| pool, manager, events, health, hooks, quarantine, autoscale | tokio                                   |
| `serde` (default)| serde derives on config types                            | serde, serde_json                         |
| `metrics`      | metrics.rs                                                | metrics, metrics-exporter-prometheus      |
| `tracing`      | structured logging throughout                             | tracing, tracing-opentelemetry, opentelemetry |
| `credentials`  | credentials.rs, Context.credentials field                 | nebula-credential                         |
| `full`         | all of the above                                          | all of the above                          |

Core types that work without any feature flags: `Resource`, `Config`, `Guard`,
`Context`, `Scope`, `Strategy`, `Lifecycle`, `Error`.

## Data Flow: Acquire/Release End to End

### Acquire

```
ActionContext::resource("postgres")
  |
  v
engine::Resources::acquire("postgres")   [crates/engine/src/resource.rs]
  |-- builds Context from workflow_id, execution_id, scope, cancellation
  |
  v
Manager::acquire("postgres", &ctx)       [crates/resource/src/manager.rs]
  |-- 1. Check quarantine status          -> Error::Unavailable if quarantined
  |-- 2. Check health_states map          -> Error::Unavailable if Unhealthy
  |-- 3. Look up pool in DashMap          -> Error::Unavailable if not registered
  |-- 4. Validate scope (Hierarchical)    -> Error::Unavailable if scope mismatch
  |-- 5. Run before-hooks                 -> hook can Cancel with Error
  |-- 6. pool.acquire_any(ctx)            (see below)
  |-- 7. Emit Acquired event
  |-- 8. Run after-hooks (success=true)
  |-- 9. Return AnyGuard
  |
  v
Pool<R>::acquire(ctx)                    [crates/resource/src/pool.rs]
  |-- tokio::select! { acquire_inner OR ctx.cancellation }
  |-- acquire_inner:
  |     1. Semaphore::acquire (with timeout)
  |     2. Pop idle entry (FIFO/LIFO)
  |     3. Skip expired (cleanup), retry
  |     4. Validate via is_valid(), cleanup invalid, retry
  |     5. If no idle: Resource::create(config, ctx)
  |     6. Wrap in Guard<R::Instance> with return callback
  |
  v
Guard<R::Instance> returned to caller (via AnyGuard -> ResourceHandle)
```

### Release (Guard Drop)

```
Guard<R::Instance> dropped
  |
  v
on_drop callback fires
  |-- spawns tokio task: Pool::return_instance(pool, instance, created_at, usage_duration)
       |
       |-- 1. Resource::recycle(&mut instance)
       |-- 2. If OK and not shutdown: push Entry back to idle queue
       |-- 3. If recycle failed or shutdown: Resource::cleanup(instance)
       |-- 4. Emit Released event (and CleanedUp if destroyed)
       |-- 5. Update stats (active--, idle count, releases++)
       |-- 6. Semaphore::add_permits(1)
```

## Integration: Engine Bridge

`crates/engine/src/resource.rs` defines `Resources`, a per-execution adapter
that bridges `nebula_resource::Manager` to `nebula_action::ResourceProvider`.

```
nebula-action                    nebula-engine                   nebula-resource
+---------------------+         +-------------------+           +------------------+
| ResourceProvider    |<--------|  Resources        |---------->| Manager          |
|   acquire(key)      |  impl   |   manager: Arc    |  calls    |   pools          |
|   -> Box<dyn Any>   |         |   scope           |           |   deps           |
+---------------------+         |   workflow_id     |           |   health_checker |
                                |   execution_id    |           |   event_bus      |
                                |   cancellation    |           |   quarantine     |
                                +-------------------+           |   hooks          |
                                                                +------------------+
```

The engine constructs a `Resources` per workflow execution with:
- An `Arc<Manager>` shared across the engine.
- Workflow/execution IDs and a `CancellationToken`.
- An `Execution`-level `Scope` built from the IDs.

When an action calls `ctx.resource("postgres")`:
1. `Resources::acquire()` builds a `Context` with the execution scope and
   cancellation token.
2. Delegates to `Manager::acquire()` which returns an `AnyGuard`.
3. Wraps the guard in a `ResourceHandle` and returns it as `Box<dyn Any + Send>`.
4. The action downcasts via `ResourceHandle::get::<T>()`.
5. When the action drops the handle, the guard drops, returning the instance to
   the pool.

## Error Handling

`Error` is a comprehensive enum with 13 variants covering configuration,
initialization, unavailability, health checks, credentials, cleanup, timeouts,
circuit breaker, pool exhaustion, dependency failures, circular dependencies,
state transitions, validation, and internal errors.

`Error::is_retryable()` returns true for: `Unavailable { retryable: true }`,
`Timeout`, `PoolExhausted`, `CircuitBreakerOpen`.

`Error::resource_id()` returns the associated resource ID when available
(all variants except `Configuration`, `CircularDependency`, `Validation`).
