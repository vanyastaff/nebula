# nebula-resource — API Reference (v2)

Complete public API reference. All types are in `nebula_resource` unless noted.
Re-exported from `nebula_core`: `ExecutionId`, `ResourceKey`, `WorkflowId`, `resource_key!`.

---

## Table of Contents

- [Core Traits](#core-traits)
- [Topology Traits](#topology-traits)
- [Topology Configs](#topology-configs)
- [Handle](#handle)
- [Manager](#manager)
- [Error Model](#error-model)
- [Context](#context)
- [Options](#options)
- [Resilience](#resilience)
- [Recovery](#recovery)
- [Events](#events)
- [Metrics](#metrics)
- [State](#state)
- [Runtime Types](#runtime-types)
- [Utilities](#utilities)

---

## Core Traits

### `Resource`

The central abstraction. Describes how to create, health-check, and tear down one resource type.
Uses RPITIT (`impl Future`) — no `Box<dyn Future>` overhead.

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;   // live resource handle
    type Lease: Send + Sync + 'static;     // what callers hold
    type Error: Into<crate::Error> + ...;  // resource-specific error
    type Auth: AuthScheme;                 // auth material resolved before create()

    fn key() -> ResourceKey;
    fn metadata() -> ResourceMetadata { ... }  // default: derived from key

    fn create(&self, config: &Self::Config, auth: &Self::Auth,
              ctx: &ResourceContext) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    fn check(&self, runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { ... }  // default: Ok

    fn shutdown(&self, runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { ... }  // default: no-op

    fn destroy(&self, runtime: Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { ... }  // default: drop
}
```

**Lifecycle:** `create → check (periodic) → shutdown → destroy`

If `Runtime` and `Lease` are the same type, the blanket `From<T> for T` satisfies conversion bounds used by pool/resident/exclusive topologies.

---

### `ResourceConfig`

Operational configuration. Must contain no secrets — auth material goes in `Auth`.

```rust
pub trait ResourceConfig: Send + Sync + Clone + 'static {
    fn validate(&self) -> Result<(), Error> { Ok(()) }
    fn fingerprint(&self) -> u64 { 0 }  // used for hot-reload change detection
}
```

Two configs with the same non-zero fingerprint are treated as identical during `reload_config`.

---

### `AuthScheme`

Authentication scheme resolved by the credential system before `Resource::create`. Use `()` for unauthenticated resources. Defined in `nebula-credential` (`crates/credential/src/scheme/auth.rs`).

```rust
pub trait AuthScheme: Send + Sync + Clone + 'static {}
```

---

### `AnyResource`

Trait-object-safe marker for type-erased resource registration. Implementors of `Resource` typically also implement this.

```rust
pub trait AnyResource: Send + Sync + 'static {
    fn key(&self) -> ResourceKey;
    fn metadata(&self) -> ResourceMetadata;
}
```

---

## Topology Traits

Topology traits extend `Resource` with lifecycle hooks specific to how instances are managed. Register the matching runtime via `TopologyRuntime`.

### `Pooled` — N interchangeable instances

```rust
pub trait Pooled: Resource {
    fn is_broken(&self, runtime: &Self::Runtime) -> BrokenCheck { Healthy }
    fn recycle(&self, runtime: &Self::Runtime, metrics: &InstanceMetrics)
        -> impl Future<Output = Result<RecycleDecision, Self::Error>> + Send { Keep }
    fn prepare(&self, runtime: &Self::Runtime, ctx: &ResourceContext)
        -> impl Future<Output = Result<(), Self::Error>> + Send { Ok(()) }
}
```

| Type | Purpose |
|------|---------|
| `BrokenCheck` | Sync O(1) result from `is_broken`: `Healthy` or `Broken(String)` |
| `RecycleDecision` | Async recycle result: `Keep` (return to pool) or `Drop` (destroy) |
| `InstanceMetrics` | `error_count`, `checkout_count`, `created_at` — available in `recycle` |

`is_broken` runs in the `Drop` path — no async, no I/O. `prepare` runs after checkout, before the caller receives the lease (use for `SET search_path`, session setup, etc.).

Acquire bounds: `R: Clone`, `R::Runtime: Clone + Into<R::Lease>`, `R::Lease: Into<R::Runtime>`.

---

### `Resident` — one shared instance, clone on acquire

```rust
pub trait Resident: Resource where Self::Lease: Clone {
    fn is_alive_sync(&self, runtime: &Self::Runtime) -> bool { true }
    fn stale_after(&self) -> Option<Duration> { None }
}
```

The runtime is created once and cloned for every caller. Use for stateless or internally-pooled clients (e.g., `reqwest::Client`).

Acquire bounds: `R::Runtime: Clone + Into<R::Lease>`, `R::Lease: Clone`.

---

### `Service` — long-lived runtime, short-lived tokens

```rust
pub trait Service: Resource {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    fn acquire_token(&self, runtime: &Self::Runtime, ctx: &ResourceContext)
        -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;

    fn release_token(&self, runtime: &Self::Runtime, token: Self::Lease)
        -> impl Future<Output = Result<(), Self::Error>> + Send { Ok(()) }
}
```

`TokenMode::Cloned` → owned handle, release is a no-op. `TokenMode::Tracked` → guarded handle, `release_token` is called on drop.

---

### `Transport` — shared connection, multiplexed sessions

```rust
pub trait Transport: Resource {
    fn open_session(&self, transport: &Self::Runtime, ctx: &ResourceContext)
        -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;

    fn close_session(&self, transport: &Self::Runtime, session: Self::Lease, healthy: bool)
        -> impl Future<Output = Result<(), Self::Error>> + Send { Ok(()) }

    fn keepalive(&self, transport: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { Ok(()) }
}
```

One transport (HTTP/2, gRPC channel, AMQP connection) serves many sessions (streams, channels). Semaphore enforces `max_sessions`.

---

### `Exclusive` — one caller at a time

```rust
pub trait Exclusive: Resource {
    fn reset(&self, runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { Ok(()) }
}
```

The semaphore allows only one active lease. `reset` is called after each release, before the next caller receives the lock. Use for serial ports, single-writer databases.

---

### `EventSource` — pull-based event subscription (secondary)

```rust
pub trait EventSource: Resource {
    type Event: Send + Clone + 'static;
    type Subscription: Send + 'static;

    fn subscribe(&self, runtime: &Self::Runtime, ctx: &ResourceContext)
        -> impl Future<Output = Result<Self::Subscription, Self::Error>> + Send;

    fn recv(&self, subscription: &mut Self::Subscription)
        -> impl Future<Output = Result<Self::Event, Self::Error>> + Send;
}
```

Secondary topology — layered on top of a primary runtime. `recv` blocks until an event arrives.

---

### `Daemon` — background run loop (secondary)

```rust
pub trait Daemon: Resource {
    fn run(&self, runtime: &Self::Runtime, ctx: &ResourceContext, cancel: CancellationToken)
        -> impl Future<Output = Result<(), Self::Error>> + Send;
}
```

Runs until the token is cancelled or the future returns. The implementation must select on `cancel` for cooperative shutdown. Governed by `RestartPolicy`: `Never`, `OnFailure` (default), `Always`.

---

## Topology Configs

Each config is the `Config` type alias exported from `nebula_resource` (e.g., `PoolConfig`).

| Type | Key fields | Defaults |
|------|-----------|---------|
| `PoolConfig` | `min_size: u32`, `max_size: u32`, `idle_timeout: Option<Duration>`, `max_lifetime: Option<Duration>`, `create_timeout: Duration`, `strategy: PoolStrategy`, `warmup: WarmupStrategy`, `test_on_checkout: bool`, `maintenance_interval: Duration`, `max_concurrent_creates: u32` | `1 / 10 / 300s / 1800s / 30s / Lifo / None / false / 30s / 3` |
| `ResidentConfig` | `recreate_on_failure: bool`, `create_timeout: Duration` | `false / 30s` |
| `ServiceConfig` | `drain_timeout: Option<Duration>` | `None` |
| `TransportConfig` | `max_sessions: u32`, `keepalive_interval: Option<Duration>`, `acquire_timeout: Duration` | `10 / Some(30s) / 30s` |
| `ExclusiveConfig` | `acquire_timeout: Duration` | `30s` |
| `EventSourceConfig` | `buffer_size: usize` | `0` |
| `DaemonConfig` | `restart_policy: RestartPolicy`, `max_restarts: u32`, `restart_backoff: Duration` | `OnFailure / 5 / 1s` |

`PoolStrategy`: `Lifo` (default, hot working set) or `Fifo` (even spread).
`WarmupStrategy`: `None` (default), `Sequential`, `Parallel`, or `Staggered { interval }`.

---

## Handle

### `ResourceGuard<R>`

The value callers hold while using a resource. Annotated `#[must_use]` — dropping immediately releases the resource back to the topology.

**Three ownership modes:**

| Mode | Description |
|------|-------------|
| `Owned` | Caller owns the lease outright — no pool return, `hold_duration` returns zero |
| `Guarded` | Exclusive lease, returned to pool via callback on drop |
| `Shared` | `Arc`-wrapped lease for resident topology |

```rust
impl<R: Resource> ResourceGuard<R> {
    // Constructors (used by topology runtimes, rarely called directly):
    pub fn owned(lease: R::Lease, key: ResourceKey, tag: TopologyTag) -> Self;
    pub fn guarded(lease: R::Lease, key: ResourceKey, tag: TopologyTag,
                   generation: u64, on_release: impl FnOnce(R::Lease, bool) + Send + 'static) -> Self;
    pub fn shared(lease: Arc<R::Lease>, key: ResourceKey, tag: TopologyTag,
                  generation: u64, on_release: impl FnOnce(bool) + Send + 'static) -> Self;

    // Use:
    pub fn taint(&mut self);                 // mark for destroy instead of recycle
    pub fn detach(self) -> Option<R::Lease>; // extract lease, skip release callback
    pub fn hold_duration(&self) -> Duration; // zero for Owned, elapsed for Guarded/Shared
    pub fn resource_key(&self) -> &ResourceKey;
    pub fn topology_tag(&self) -> TopologyTag;
    pub fn generation(&self) -> Option<u64>;

    // Implements: Deref<Target = R::Lease>, Debug, Drop
}
```

**Panic safety:** The `Drop` impl wraps release callbacks in `catch_unwind`. If the callback panics, the semaphore permit is still returned — no permit leaks.

`detach` on a `Shared` guard returns `None` because the `Arc` may have other holders.

---

## Manager

### `Manager`

Central registry and lifecycle manager. Share via `Arc<Manager>`.

```rust
impl Manager {
    pub fn new() -> Self;
    pub fn with_config(config: ManagerConfig) -> Self;

    // Registration:
    pub fn register<R: Resource>(
        &self, resource: R, config: R::Config,
        scope: ScopeLevel, topology: TopologyRuntime<R>,
        resilience: Option<AcquireResilience>,
        recovery_gate: Option<Arc<RecoveryGate>>,
    ) -> Result<(), Error>;

    // Convenience shorthands (Auth = (), scope = Global, no resilience/gate):
    pub fn register_pooled<R: Resource<Auth = ()>>(
        &self, resource: R, config: R::Config, pool_config: PoolConfig) -> Result<(), Error>;
    pub fn register_resident<R: Resource<Auth = ()>>(
        &self, resource: R, config: R::Config, resident_config: ResidentConfig) -> Result<(), Error>;
    pub fn register_service<R: Resource<Auth = ()>>(
        &self, resource: R, config: R::Config, runtime: R::Runtime,
        service_config: ServiceConfig) -> Result<(), Error>;
    pub fn register_exclusive<R: Resource<Auth = ()>>(
        &self, resource: R, config: R::Config, runtime: R::Runtime,
        exclusive_config: ExclusiveConfig) -> Result<(), Error>;

    // Lookup:
    pub fn lookup<R: Resource>(&self, scope: &ScopeLevel) -> Result<Arc<ManagedResource<R>>, Error>;
    pub fn contains(&self, key: &ResourceKey) -> bool;
    pub fn keys(&self) -> Vec<ResourceKey>;
    pub fn get_any(&self, key: &ResourceKey, scope: &ScopeLevel)
        -> Option<Arc<dyn AnyManagedResource>>;

    // Acquire (topology-specific):
    pub async fn acquire_pooled<R: Pooled + Clone + ...>(
        &self, auth: &R::Auth, ctx: &ResourceContext, options: &AcquireOptions,
    ) -> Result<ResourceGuard<R>, Error>;
    pub async fn acquire_resident<R: Resident + ...>(...) -> Result<ResourceGuard<R>, Error>;
    pub async fn acquire_service<R: Service + Clone + ...>(...) -> Result<ResourceGuard<R>, Error>;
    pub async fn acquire_transport<R: Transport + Clone + ...>(...) -> Result<ResourceGuard<R>, Error>;
    pub async fn acquire_exclusive<R: Exclusive + Clone + ...>(...) -> Result<ResourceGuard<R>, Error>;

    // Config hot-reload:
    pub fn reload_config<R: Resource>(&self, new_config: R::Config,
        scope: &ScopeLevel) -> Result<(), Error>;

    // Removal:
    pub fn remove(&self, key: &ResourceKey) -> Result<(), Error>;

    // Observability:
    pub fn subscribe_events(&self) -> broadcast::Receiver<ResourceEvent>;
    pub fn metrics(&self) -> Option<&ResourceOpsMetrics>;
    pub fn recovery_groups(&self) -> &RecoveryGroupRegistry;
    pub fn cancel_token(&self) -> &CancellationToken;
    pub fn is_shutdown(&self) -> bool;

    // Shutdown:
    pub fn shutdown(&self);  // immediate — cancels token, no drain
    pub async fn graceful_shutdown(&self, config: ShutdownConfig);
}
```

**`reload_config` mechanics:** validates the new config, atomically swaps it, increments the generation counter. For pool topology, updates the fingerprint so idle instances created with the old config are evicted on next checkout or release.

**`graceful_shutdown` phases:** (1) cancel token → new acquires rejected; (2) drain in-flight handles up to `drain_timeout`; (3) clear registry; (4) await release queue workers (bounded 10 s).

---

### `ManagerConfig`

```rust
pub struct ManagerConfig {
    pub release_queue_workers: usize,           // default: 2
    pub metrics_registry: Option<Arc<MetricsRegistry>>,  // default: None
}
```

---

### `ShutdownConfig`

```rust
pub struct ShutdownConfig {
    pub drain_timeout: Duration,  // default: 30s
}
```

---

## Error Model

### `Error`

```rust
pub struct Error { /* kind, scope, message, resource_key, source */ }

impl Error {
    // Constructors:
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self;
    pub fn transient(message: impl Into<String>) -> Self;
    pub fn permanent(message: impl Into<String>) -> Self;
    pub fn exhausted(message: impl Into<String>, retry_after: Option<Duration>) -> Self;
    pub fn not_found(key: &ResourceKey) -> Self;
    pub fn cancelled() -> Self;
    pub fn backpressure(message: impl Into<String>) -> Self;

    // Builders:
    pub fn with_resource_key(self, key: ResourceKey) -> Self;
    pub fn with_source(self, source: impl Error + Send + Sync + 'static) -> Self;
    pub fn with_scope(self, scope: ErrorScope) -> Self;

    // Introspection:
    pub fn kind(&self) -> &ErrorKind;
    pub fn scope(&self) -> &ErrorScope;
    pub fn resource_key(&self) -> Option<&ResourceKey>;
    pub fn is_retryable(&self) -> bool;  // true for Transient and Exhausted
    pub fn retry_after(&self) -> Option<Duration>;
}
```

---

### `ErrorKind` and `ErrorScope`

```rust
#[non_exhaustive]
pub enum ErrorKind {
    Transient,                              // network blip, timeout — retry with backoff
    Permanent,                              // auth failure, bad config — never retry
    Exhausted { retry_after: Option<Duration> }, // rate limit — retry after cooldown
    Backpressure,                           // pool/semaphore full
    NotFound,                               // key not in registry
    Cancelled,                              // CancellationToken fired
}

#[non_exhaustive]
pub enum ErrorScope {
    Resource,                 // the resource itself may be broken (default)
    Target { id: String },    // only a specific target failed (e.g., one user blocked)
}
```

**Retryability:** `Transient` and `Exhausted` return `true` from `is_retryable()`. The `AcquireResilience` retry loop only retries errors where `is_retryable()` is `true`.

---

### `ClassifyError` derive macro

Generates `From<YourError> for nebula_resource::Error`. See `nebula_resource_macros::ClassifyError` for full documentation.

---

## Context

### `ResourceContext`

Concrete execution context passed to all resource lifecycle methods.

```rust
impl ResourceContext {
    pub fn new(execution_id: ExecutionId) -> Self;  // scope = Global
    pub fn with_scope(self, scope: ScopeLevel) -> Self;
    pub fn with_cancel_token(self, token: CancellationToken) -> Self;
}
```

`ResourceContext` implements capability traits (`HasResources`, `HasCredentials`) rather than
carrying an `Extensions` type-map. Access typed capabilities via the trait methods.

---

### `ScopeLevel`

Re-exported from `nebula_core::ScopeLevel`.

Lifecycle boundary for resource instances. Finer scopes are cleaned up more aggressively.

```rust
pub enum ScopeLevel {
    Global,
    Organization(String),
    Project(String),
    Workflow(WorkflowId),
    Execution(ExecutionId),
}
```

Registry lookup prefers an exact scope match; falls back to `Global`.

---

## Options

### `AcquireOptions`

Passed to every `acquire_*` call. Communicates intent and deadline to topologies.

```rust
pub struct AcquireOptions {
    pub intent: AcquireIntent,      // default: Standard
    pub deadline: Option<Instant>,  // absolute deadline for the acquire
    pub tags: SmallVec<[...; 2]>,   // routing/diagnostics key-value pairs
}

impl AcquireOptions {
    pub fn with_deadline(self, deadline: Instant) -> Self;
    pub fn with_intent(self, intent: AcquireIntent) -> Self;
    pub fn with_tag(self, key: impl Into<Cow<'static, str>>,
                    value: impl Into<Cow<'static, str>>) -> Self;
    pub fn remaining(&self) -> Option<Duration>;
}
```

---

### `AcquireIntent`

```rust
#[non_exhaustive]
pub enum AcquireIntent {
    Standard,                          // default path
    LongRunning,                       // caller will hold the lease for a long time
    Streaming { expected: Duration },  // streaming data, hint for duration
    Prefetch,                          // low priority, may be deferred
    Critical,                          // bypass queues, never throttle
}
```

---

## Resilience

### `AcquireResilience`

Wraps acquire calls with timeout, exponential-backoff retry, and circuit-breaker hints.
Pass as `resilience` parameter to `Manager::register`.

```rust
pub struct AcquireResilience {
    pub timeout: Option<Duration>,
    pub retry: Option<AcquireRetryConfig>,
    pub circuit_breaker: Option<AcquireCircuitBreakerPreset>,
}

impl AcquireResilience {
    pub fn standard() -> Self;  // 30s timeout, 3 attempts, standard breaker
    pub fn fast() -> Self;      // 10s timeout, 2 attempts, fast breaker
    pub fn slow() -> Self;      // 60s timeout, 5 attempts, slow breaker
    pub fn none() -> Self;      // no timeout, no retries, no breaker
}
```

---

### `AcquireRetryConfig` and `AcquireCircuitBreakerPreset`

```rust
pub struct AcquireRetryConfig {
    pub max_attempts: u32,      // total attempts including the initial try
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

pub enum AcquireCircuitBreakerPreset {
    Standard,  // 5 failures, 30s reset
    Fast,      // 3 failures, 10s reset
    Slow,      // 10 failures, 60s reset
}
```

Retry only fires on `is_retryable()` errors. `timeout` is a per-attempt wall-clock limit.

---

## Recovery

### `RecoveryGate`

CAS-based state machine preventing thundering herd on dead backends. Only one caller at a time performs the expensive probe.

```text
Idle → InProgress → Idle           (success path)
                  → Failed         (transient failure, exponential backoff)
                  → PermanentlyFailed (max_attempts exceeded or explicit fail_permanent)
```

```rust
impl RecoveryGate {
    pub fn new(config: RecoveryGateConfig) -> Self;
    pub fn try_begin(&self) -> Result<RecoveryTicket, TryBeginError>;
    pub fn state(&self) -> GateState;
    pub fn reset(&self);  // admin override, returns gate to Idle
}
```

---

### `RecoveryTicket`

RAII exclusive recovery rights. `#[must_use]` — dropping without resolving auto-fails with backoff.

```rust
impl RecoveryTicket {
    pub fn resolve(self);                             // success → Idle, wake waiters
    pub fn fail_transient(self, message: impl Into<String>);  // → Failed with backoff
    pub fn fail_permanent(self, message: impl Into<String>);  // → PermanentlyFailed
    pub fn attempt(&self) -> u32;
}
```

---

### `RecoveryGateConfig`, `GateState`, `RecoveryWaiter`

```rust
pub struct RecoveryGateConfig {
    pub max_attempts: u32,     // default: 5
    pub base_backoff: Duration, // default: 1s, doubled each attempt, capped at 5 min
}

#[non_exhaustive]
pub enum GateState {
    Idle,
    InProgress { attempt: u32 },
    Failed { message: String, retry_at: Instant, attempt: u32 },
    PermanentlyFailed { message: String },
}

impl RecoveryWaiter {
    pub async fn wait(&self) -> GateState;  // blocks until gate leaves InProgress
}
```

---

### `RecoveryGroupRegistry`

Per-key registry of `RecoveryGate` instances. `Manager` exposes this via `recovery_groups()`.

```rust
impl RecoveryGroupRegistry {
    pub fn new() -> Self;
    pub fn get_or_create(&self, key: RecoveryGroupKey, config: RecoveryGateConfig)
        -> Arc<RecoveryGate>;
    pub fn get(&self, key: &RecoveryGroupKey) -> Option<Arc<RecoveryGate>>;
    pub fn remove(&self, key: &RecoveryGroupKey) -> Option<Arc<RecoveryGate>>;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}
```

---

### `WatchdogHandle`

Opt-in periodic health probe. Calls `on_health_change(false)` after `failure_threshold` consecutive failures; calls `on_health_change(true)` after `recovery_threshold` consecutive successes. Dropping the handle cancels the probe.

```rust
impl WatchdogHandle {
    pub fn start<F, Fut>(config: WatchdogConfig, check_fn: F,
        on_health_change: impl Fn(bool) + Send + Sync + 'static,
        parent_cancel: CancellationToken) -> Self
    where F: Fn() -> Fut + Send + Sync + 'static, Fut: Future<Output = Result<(), Error>> + Send;

    pub async fn stop(self);
}

pub struct WatchdogConfig {
    pub interval: Duration,           // default: 30s
    pub probe_timeout: Duration,      // default: 5s — timeout counts as failure
    pub failure_threshold: u32,       // default: 3
    pub recovery_threshold: u32,      // default: 1
}
```

---

## Events

### `ResourceEvent`

Lifecycle events emitted by `Manager` into a 256-slot broadcast channel. Slow consumers receive `RecvError::Lagged`.

```rust
#[non_exhaustive]
pub enum ResourceEvent {
    Registered { key: ResourceKey },
    Removed { key: ResourceKey },
    AcquireSuccess { key: ResourceKey, duration: Duration },
    AcquireFailed { key: ResourceKey, error: String },
    Released { key: ResourceKey, held: Duration, tainted: bool },
    HealthChanged { key: ResourceKey, healthy: bool },
    ConfigReloaded { key: ResourceKey },
}

impl ResourceEvent {
    pub fn key(&self) -> &ResourceKey;
}
```

Subscribe via `Manager::subscribe_events() -> broadcast::Receiver<ResourceEvent>`.

---

## Metrics

### `ResourceOpsMetrics` and `ResourceOpsSnapshot`

Lock-free atomic counters backed by a `MetricsRegistry`. `Manager::metrics()` returns aggregate counters if a `MetricsRegistry` was provided via `ManagerConfig`.

```rust
impl ResourceOpsMetrics {
    pub fn new(registry: &MetricsRegistry) -> Self;
    pub fn record_acquire(&self);
    pub fn record_acquire_error(&self);
    pub fn record_release(&self);
    pub fn record_create(&self);
    pub fn record_destroy(&self);
    pub fn snapshot(&self) -> ResourceOpsSnapshot;
}

pub struct ResourceOpsSnapshot {
    pub acquire_total: u64,
    pub acquire_errors: u64,
    pub release_total: u64,
    pub create_total: u64,
    pub destroy_total: u64,
}
```

---

## State

### `ResourcePhase` and `ResourceStatus`

```rust
#[non_exhaustive]
pub enum ResourcePhase {
    Initializing, Ready, Reloading, Draining, ShuttingDown, Failed,
}

impl ResourcePhase {
    pub fn is_accepting(&self) -> bool;  // true for Ready and Reloading
    pub fn is_terminal(&self) -> bool;   // true for ShuttingDown and Failed
}

pub struct ResourceStatus {
    pub phase: ResourcePhase,
    pub generation: u64,             // incremented on each reload_config
    pub last_error: Option<String>,
}
```

---

## Runtime Types

These types are used when calling `Manager::register` directly. The convenience helpers (`register_pooled`, etc.) construct them internally.

### `TopologyRuntime<R>`

Dispatch enum selecting which runtime manages a resource's instances.

```rust
pub enum TopologyRuntime<R: Resource> {
    Pool(PoolRuntime<R>),
    Resident(ResidentRuntime<R>),
    Service(ServiceRuntime<R>),
    Transport(TransportRuntime<R>),
    Exclusive(ExclusiveRuntime<R>),
    EventSource(EventSourceRuntime<R>),
    Daemon(DaemonRuntime<R>),
}
```

Each variant (`PoolRuntime<R>`, `ResidentRuntime<R>`, etc.) is constructible via `::new(config)` or `::new(runtime, config)`. Topology runtimes are not intended to be called directly by application code — use the `Manager::acquire_*` methods.

### `ManagedResource<R>`

Per-registration bundle holding the resource, hot-swappable config, topology runtime, release queue, generation counter, status, metrics, resilience config, and optional recovery gate. Returned by `Manager::lookup` for low-level access.

```rust
impl<R: Resource> ManagedResource<R> {
    pub fn generation(&self) -> u64;
    pub fn status(&self) -> Arc<ResourceStatus>;
    pub fn config(&self) -> Arc<R::Config>;
    pub fn metrics(&self) -> &Arc<ResourceOpsMetrics>;
}
```

### `Registry` and `AnyManagedResource`

Type-erased storage for `ManagedResource<R>`. Provides two lookup paths: by `ResourceKey + ScopeLevel` (returns `Arc<dyn AnyManagedResource>`) and by type `R` (downcasts to `Arc<ManagedResource<R>>`). Exposed via `Manager::get_any` for diagnostic use.

---

## Utilities

### `Cell<T>`

Lock-free `ArcSwapOption`-backed cell for the Resident topology. Holds at most one `Arc<T>`.

```rust
impl<T> Cell<T> {
    pub fn new() -> Self;
    pub fn store(&self, value: Arc<T>);
    pub fn load(&self) -> Option<Arc<T>>;
    pub fn take(&self) -> Option<Arc<T>>;
    pub fn is_some(&self) -> bool;
}
```

---

### `ReleaseQueue`

Distributes async cleanup tasks across N primary workers and one unbounded fallback. Primary channels are bounded (256 tasks each); overflow goes to the fallback — tasks are never dropped. Workers drain buffered tasks before exiting on cancellation.

```rust
impl ReleaseQueue {
    pub fn new(worker_count: usize) -> (Self, ReleaseQueueHandle);  // panics if 0
    pub fn with_cancel(worker_count: usize, cancel: CancellationToken)
        -> (Self, ReleaseQueueHandle);
    pub fn submit(&self, factory: impl FnOnce() -> Pin<Box<dyn Future<Output = ()> + Send>>
                                               + Send + 'static);
    pub fn close(&self);
    pub async fn shutdown(handle: ReleaseQueueHandle);
}
```

Task execution is bounded at 30 s per task. When used via `Manager`, the manager's token is shared with workers so `graceful_shutdown` signals drain automatically.

`ReleaseQueueHandle` is `#[must_use]` — dropping without calling `shutdown` leaks worker tasks.

---

### `ResourceMetadata`

UI and diagnostic information returned by `Resource::metadata()`.

```rust
pub struct ResourceMetadata {
    pub key: ResourceKey,
    pub name: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
}

impl ResourceMetadata {
    pub fn from_key(key: &ResourceKey) -> Self;
}
```

---

### `TopologyTag`

Identifies which topology a `ResourceGuard` was acquired from. Used for observability and diagnostics.

```rust
#[non_exhaustive]
pub enum TopologyTag { Pool, Resident, Service, Transport, Exclusive, EventSource, Daemon }

impl TopologyTag {
    pub fn as_str(self) -> &'static str;
}
```
