# nebula-resource — API Reference

Complete public API reference. All types are in `nebula_resource` unless noted.

---

## Table of Contents

- [Core Traits](#core-traits)
- [Manager](#manager)
- [Pool](#pool)
- [Guard](#guard)
- [Context and Scope](#context-and-scope)
- [Metadata](#metadata)
- [Error Handling](#error-handling)
- [Lifecycle](#lifecycle)
- [Resource References and Providers](#resource-references-and-providers)
- [Components and Credentials](#components-and-credentials)
- [Observability](#observability)
- [Prelude](#prelude)

---

## Core Traits

### `Config`

```rust
pub trait Config: Send + Sync + 'static {
    fn validate(&self) -> Result<()> { Ok(()) }
}
```

Marker trait for resource configuration types. Override `validate` to return
`Error::Validation` on invalid fields before a pool is created.

---

### `Resource`

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: Config;
    type Instance: Send + Sync + 'static;

    fn metadata(&self) -> ResourceMetadata;

    async fn create(
        &self,
        config: &Self::Config,
        ctx: &Context,
    ) -> Result<Self::Instance>;

    async fn is_reusable(
        &self,
        instance: &Self::Instance,
    ) -> Result<bool> { Ok(true) }

    async fn recycle(
        &self,
        instance: &mut Self::Instance,
    ) -> Result<()> { Ok(()) }

    async fn cleanup(
        &self,
        instance: Self::Instance,
    ) -> Result<()> { drop(instance); Ok(()) }

    fn declare_key() -> ResourceKey
    where Self: Sized;
}
```

| Method | Description |
|--------|-------------|
| `metadata` | Returns static `ResourceMetadata` used for discovery and events. The `metadata.key` is the canonical `ResourceKey` for this type. |
| `create` | Builds a new `Instance` from `Config` and `Context`. Called when the pool needs a new connection. |
| `is_broken` | **Synchronous** fast-path broken check. Return `true` if the instance is obviously dead (e.g. closed socket, invalid descriptor) so the pool skips recycle immediately. Default: always `false`. |
| `prepare` | **Async** per-acquire hook called just before the instance is handed to the caller. Use for authentication refresh, lease renewal, or health pre-check. Default: no-op. |
| `is_reusable` | Returns `Ok(true)` if the instance can be reused after being returned to the pool. Return `Ok(false)` or `Err` to discard. Default: always reusable. |
| `is_reusable_with_meta` | Metadata-aware variant of `is_reusable`. Receives an `InstanceMetadata` snapshot (creation time, idle duration, acquire count). Defaults to calling `is_reusable`. |
| `recycle` | Resets state before returning an instance to the idle queue (e.g. rollback uncommitted transactions). Default: no-op. |
| `recycle_with_meta` | Metadata-aware variant of `recycle`. Receives `InstanceMetadata`. Defaults to calling `recycle`. |
| `cleanup` | Permanently destroys an instance (close socket, flush buffers). Called for tainted or expired instances. Default: drop. |
| `declare_key` | Returns the `ResourceKey` used by `ResourceRef::of::<R>()`. Default: snake_case of the type name. Override for a stable key independent of type name. |

---

## Manager

### `Manager`

Central registry. Holds one `Pool<R>` per registered resource type.

```rust
impl Manager {
    pub fn new() -> Self;
    pub fn builder() -> ManagerBuilder;

    /// Register a resource with its config and pool settings.
    /// Returns a typed pool handle for direct pool access.
    pub fn register<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        pool_config: PoolConfig,
    ) -> Result<TypedPool<R>>;

    /// Register a resource under a specific scope and strategy.
    pub fn register_scoped<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        pool_config: PoolConfig,
        scope: Scope,
        strategy: Strategy,
    ) -> Result<TypedPool<R>>;

    /// Acquire an instance by resource key.
    pub async fn acquire(
        &self,
        key: &ResourceKey,
        ctx: &Context,
    ) -> Result<AnyGuard>;

    /// Acquire a typed instance.
    pub async fn acquire_typed<R: Resource>(
        &self,
        ctx: &Context,
    ) -> Result<TypedResourceGuard<R>>;

    /// Get a typed pool handle for a registered resource.
    pub fn pool<R: Resource>(&self) -> Option<TypedPool<R>>;

    /// Hot-reload config for a resource without dropping the existing pool.
    pub async fn reload_config<R: Resource>(
        &self,
        config: R::Config,
    ) -> Result<()>;

    /// Aggregate status snapshot for all registered resources.
    pub fn list_status(&self) -> Vec<ResourceStatus>;

    /// Status snapshot for one resource.
    pub fn get_status(&self, key: &ResourceKey) -> Option<ResourceStatus>;

    /// Reference to the shared event bus.
    pub fn event_bus(&self) -> &Arc<EventBus>;

    /// Declare a dependency between two resource keys.
    pub fn add_dependency(
        &self,
        dependent: &ResourceKey,
        dependency: &ResourceKey,
    ) -> Result<()>;

    /// Enable auto-scaling for one registered resource.
    pub fn enable_autoscaling(
        &self,
        key: &ResourceKey,
        policy: AutoScalePolicy,
    ) -> Result<()>;

    /// Graceful shutdown with phased drain, cleanup, and terminate.
    pub async fn shutdown(&self) -> Result<()>;

    /// Graceful shutdown with explicit timeouts.
    pub async fn shutdown_with(&self, config: ShutdownConfig) -> Result<()>;
}
```

---

### `ManagerBuilder`

```rust
impl ManagerBuilder {
    pub fn new() -> Self;

    pub fn health_config(self, config: HealthCheckConfig) -> Self;
    pub fn event_bus(self, event_bus: Arc<EventBus>) -> Self;
    pub fn quarantine_config(self, config: QuarantineConfig) -> Self;

    /// Set a default AutoScalePolicy applied to every registered pool.
    pub fn default_autoscale_policy(self, policy: AutoScalePolicy) -> Self;

    pub fn build(self) -> Manager;
}
```

**Example:**

```rust
use std::sync::Arc;
use nebula_resource::{EventBus, HealthCheckConfig, ManagerBuilder, QuarantineConfig};

let manager = ManagerBuilder::new()
    .event_bus(Arc::new(EventBus::new(4096)))
    .health_config(HealthCheckConfig {
        failure_threshold: 5,
        ..Default::default()
    })
    .quarantine_config(QuarantineConfig::default())
    .build();
```

---

### `ShutdownConfig`

```rust
pub struct ShutdownConfig {
    /// Max wait for in-flight acquisitions to complete. Default: 30s.
    pub drain_timeout: Duration,
    /// Max time for cleanup callbacks per pool. Default: 10s.
    pub cleanup_timeout: Duration,
    /// Max time for forceful termination after cleanup. Default: 5s.
    pub terminate_timeout: Duration,
}
```

---

### `ResourceStatus`

Aggregate snapshot combining metadata, health, pool dimensions, and quarantine
state. Used by `nebula-api` for `GET /resources` and `GET /resources/:id`.

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ResourceStatus {
    pub metadata: ResourceMetadata,
    pub health: HealthState,
    pub pool: ResourcePoolStatus,
    pub quarantined: bool,
    pub quarantine_reason: Option<String>,
    pub scope: Scope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ResourcePoolStatus {
    pub active: usize,
    pub idle: usize,
    pub max_size: usize,
}
```

---

### `TypedPool<R>`

A typed handle to a registered pool. Acquired from `Manager::register` or
`Manager::pool::<R>()`.

```rust
impl<R: Resource> TypedPool<R> {
    pub async fn acquire(&self, ctx: &Context) -> Result<TypedResourceGuard<R>>;
    pub fn stats(&self) -> Result<PoolStats>;
}
```

---

### `TypedResourceGuard<R>` and `AnyGuard`

```rust
// TypedResourceGuard<R> — returned by acquire_typed / TypedPool::acquire
impl<R: Resource> TypedResourceGuard<R> {
    /// Extract the instance, skipping pool return (use carefully).
    pub fn into_inner(self) -> R::Instance;
    // Implements Deref<Target = R::Instance>, DerefMut, Drop
}

// AnyGuard — type-erased guard returned by Manager::acquire
impl AnyGuard {
    /// Downcast to a typed guard.
    pub fn downcast<R: Resource>(self) -> Result<TypedResourceGuard<R>>;
    // Implements AnyGuardTrait, Drop
}
```

---

## Pool

### `PoolConfig`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolConfig {
    /// Minimum number of idle connections to maintain. Default: 1.
    pub min_size: usize,
    /// Maximum total connections (idle + active). Default: 10.
    pub max_size: usize,
    /// Max wait for acquire permit. Default: 30s.
    pub acquire_timeout: Duration,
    /// Evict idle connections older than this. Default: 10 min.
    pub idle_timeout: Duration,
    /// Evict connections older than this regardless of idle time. Default: 30 min.
    pub max_lifetime: Duration,
    /// How often `is_reusable` is called on idle connections. Default: 60s.
    pub validation_interval: Duration,
    /// How often background maintenance runs. Default: Some(60s).
    pub maintenance_interval: Option<Duration>,
    /// Idle selection order. Default: Fifo.
    pub strategy: PoolStrategy,
    /// Acquire backpressure. Default: BoundedWait { timeout: 30s }.
    pub backpressure_policy: Option<PoolBackpressurePolicy>,
    /// Circuit breaker for Resource::create. Default: None.
    pub create_breaker: Option<CircuitBreakerConfig>,
    /// Circuit breaker for Resource::recycle. Default: None.
    pub recycle_breaker: Option<CircuitBreakerConfig>,
    /// Timeout for Resource::create. Default: None (uses acquire_timeout).
    pub create_timeout: Option<Duration>,
    /// Timeout for Resource::recycle. Default: None.
    pub recycle_timeout: Option<Duration>,
    /// Whether instances are exclusively owned by one caller (default) or
    /// cloned and shared across callers without consuming semaphore permits.
    /// `Shared` requires `R::Instance: Clone`; use `Pool::acquire_shared`.
    pub sharing_mode: PoolSharingMode,
    /// When `true`, the pool spawns a background warm-up `maintain` call
    /// immediately after construction to pre-fill idle instances up to
    /// `min_size`. Default: `false`.
    pub warm_up: bool,
}

impl PoolConfig {
    /// Attach sensible circuit breakers to create and recycle.
    pub fn with_standard_breakers(self) -> Self;

    /// Validate constraints (min_size <= max_size, etc.).
    pub fn validate(&self) -> Result<()>;

    /// Resolve the effective backpressure policy (falls back to BoundedWait).
    pub fn effective_backpressure_policy(&self) -> PoolBackpressurePolicy;
}
```

---

### `PoolStrategy`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PoolStrategy {
    /// Return the oldest idle instance. Distributes usage evenly. Default.
    #[default]
    Fifo,
    /// Return the most recently used idle instance.
    /// Keeps a hot working set small; idle-surplus instances expire naturally.
    Lifo,
}
```

---

### `PoolSharingMode`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PoolSharingMode {
    /// Each caller gets an exclusively owned instance (the pool's default).
    #[default]
    Exclusive,
    /// Callers receive a cloned copy of an idle instance without consuming a
    /// semaphore permit. Requires `R::Instance: Clone`. Use
    /// `Pool::acquire_shared` to opt into this mode per call.
    Shared,
}
```

`Shared` is intended for lightweight, immutable-by-convention instances
(e.g. a config snapshot or read-only HTTP client wrapper). The pool keeps
one resident instance alive; every call to `acquire_shared` clones it and
returns a no-op guard (drop does not recycle or return a permit).

---

### `PoolBackpressurePolicy`

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PoolBackpressurePolicy {
    /// Return Error::PoolExhausted immediately if no permit is available.
    FailFast,
    /// Wait up to `timeout`, then return Error::PoolExhausted.
    BoundedWait { timeout: Duration },
    /// Switch between low/high-pressure timeouts based on utilisation.
    Adaptive(AdaptiveBackpressurePolicy),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveBackpressurePolicy {
    /// Utilisation ratio (active / max_size) above which pressure is "high". Default: 0.8.
    pub high_pressure_utilization: f64,
    /// Waiter count above which pressure is "high". Default: 8.
    pub high_pressure_waiters: usize,
    /// Acquire timeout under low pressure. Default: 30s.
    pub low_pressure_timeout: Duration,
    /// Acquire timeout under high pressure. Default: 100ms.
    pub high_pressure_timeout: Duration,
}
```

---

### `InstanceMetadata`

Passed to `Resource::is_reusable_with_meta` and `Resource::recycle_with_meta`.
Allows implementations to make lifecycle decisions based on how long an instance
has been alive, how long it has been idle, and how many times it has been used.

```rust
#[derive(Debug, Clone, Copy)]
pub struct InstanceMetadata {
    /// When the instance was created by `Resource::create`.
    pub created_at: Instant,
    /// When the instance last entered the idle queue (last release time).
    pub idle_since: Instant,
    /// How many times this instance has been checked out (including current).
    pub acquire_count: usize,
}
```

**Example** — rotate a connection if it is more than 30 minutes old:

```rust
async fn is_reusable_with_meta(
    &self,
    instance: &Self::Instance,
    meta: &InstanceMetadata,
) -> Result<bool> {
    if meta.created_at.elapsed() > Duration::from_secs(1800) {
        return Ok(false); // force cleanup and re-create
    }
    self.is_reusable(instance).await
}
```

---

### Pool methods

```rust
impl<R: Resource> Pool<R> {
    /// Evict idle instances for which `predicate` returns `false`.
    /// Respects `min_size` — will not drop below the minimum.
    /// Returns the number of evicted instances.
    pub async fn retain<F>(
        &self,
        predicate: F,  // FnMut(&R::Instance, created_at: Instant, idle_since: Instant) -> bool
    ) -> usize;

    /// Resize the pool live without draining it.
    /// - Grow: adds `(new_max - current_max)` semaphore permits.
    /// - Shrink: best-effort — tries to quietly absorb excess permits;
    ///   active guards beyond `new_max` complete normally.
    pub fn set_max_size(&self, new_max: usize) -> Result<()>;
}

/// Available only when `R::Instance: Clone`.
impl<R: Resource> Pool<R>
where
    R::Instance: Clone,
{
    /// Acquire a cloned snapshot of the front idle instance without consuming
    /// a semaphore permit. If the idle queue is empty, creates one instance
    /// and keeps it resident.
    ///
    /// The returned guard's `on_drop` is a no-op — dropping the guard neither
    /// returns a permit nor recycles the instance.
    ///
    /// Use for lightweight, effectively-immutable resources (e.g. config
    /// snapshots, read-only clients).
    pub async fn acquire_shared(
        &self,
        ctx: &Context,
    ) -> Result<(Guard<R::Instance, fn(R::Instance, bool)>, Duration)>;
}
```

---

### `PoolStats` and `LatencyPercentiles`

```rust
#[derive(Debug, Clone, Serialize)]
pub struct PoolStats {
    pub total_acquisitions: u64,
    pub total_releases: u64,
    pub active: usize,
    pub idle: usize,
    pub created: u64,
    pub destroyed: u64,
    pub total_wait_time_ms: u64,
    pub max_wait_time_ms: u64,
    pub exhausted_count: u64,
    /// HDR histogram percentiles. None if no acquisitions recorded yet.
    pub acquire_latency: Option<LatencyPercentiles>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatencyPercentiles {
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub p999_ms: u64,
    pub mean_ms: f64,
}
```

---

## Guard

### `Guard<T>`

```rust
pub struct Guard<T, F = Box<dyn FnOnce(T, bool) + Send>>
where
    F: FnOnce(T, bool) + Send + 'static,
{
    // fields private
}

impl<T, F> Guard<T, F>
where
    F: FnOnce(T, bool) + Send + 'static,
{
    /// Construct a guard with a drop callback.
    /// `on_drop(instance, is_tainted)` is called exactly once on drop.
    pub fn new(resource: T, on_drop: F) -> Self;

    /// Mark the instance as unusable. The drop callback will receive
    /// `is_tainted = true`, causing the pool to call cleanup instead of recycle.
    pub fn taint(&mut self);

    pub fn is_tainted(&self) -> bool;

    /// Extract the inner value, consuming the guard without calling the callback.
    pub fn into_inner(self) -> T;

    /// Detach the instance from the pool: fires the `on_detach` callback (which
    /// returns the semaphore permit and decrements the active count) but skips
    /// the `on_drop` recycle/cleanup path entirely. Use when ownership of the
    /// instance is being transferred outside the pool (e.g. moved into a long-
    /// lived actor).
    pub fn detach(self) -> T;

    /// Leak the instance: extracts the value without calling any callback.
    /// The semaphore permit is **not** returned. The pool's active count and
    /// `max_size` enforcement are permanently affected until the pool is
    /// destroyed. Only use for diagnostics or intentional permanent extraction.
    pub fn leak(self) -> T;
}

// Implements: Deref<Target = T>, DerefMut, Debug (where T: Debug), Drop
```

---

## Context and Scope

### `Context`

```rust
pub struct Context {
    pub scope: Scope,
    pub execution_id: ExecutionId,
    pub workflow_id: WorkflowId,
    /// W3C TraceContext for distributed tracing propagation. Optional.
    pub trace_context: Option<TraceContext>,
    // cancellation, metadata, recorder — accessed via methods
    // tenant_id is derived from scope — see tenant_id() below
}

impl Context {
    /// Construct with an active cancellation token derived from a new root.
    pub fn new(scope: Scope, workflow_id: WorkflowId, execution_id: ExecutionId) -> Self;

    /// Construct without a cancellation token (for background/system operations).
    pub fn background(scope: Scope, workflow_id: WorkflowId, execution_id: ExecutionId) -> Self;

    /// Returns the tenant ID embedded in the current scope (if any).
    /// Reads from Tenant / Workflow / Execution / Action scope variants.
    pub fn tenant_id(&self) -> Option<&str>;

    /// Upgrade the scope to embed `tenant_id` instead of storing a separate field.
    /// Global → Tenant; Tenant → replaced; Workflow/Execution/Action → tenant_id sub-field updated.
    pub fn with_tenant(self, tenant_id: impl Into<String>) -> Self;
    pub fn with_metadata(self, key: impl Into<String>, value: impl Into<String>) -> Self;
    pub fn with_cancellation(self, token: CancellationToken) -> Self;
    pub fn with_recorder(self, recorder: Arc<dyn Recorder>) -> Self;
    /// Attach a W3C TraceContext for distributed tracing propagation.
    pub fn with_trace_context(self, tc: TraceContext) -> Self;

    pub fn is_cancellable(&self) -> bool;
    pub fn is_cancelled(&self) -> bool;
    pub fn recorder(&self) -> Arc<dyn Recorder>;

    /// Retrieve a typed sub-resource pool handle previously injected by the manager.
    pub fn resource<R: Resource>(&self, key: &str) -> Option<ResourcePoolHandle<R>>;
}

/// W3C TraceContext propagation headers for distributed tracing.
///
/// Inject into outbound requests via `inject_headers`; extract from inbound
/// requests via `from_headers`.
pub struct TraceContext {
    /// Mandatory `traceparent` header value.
    /// Format: `{version}-{trace-id}-{parent-id}-{trace-flags}`
    pub traceparent: String,
    /// Optional `tracestate` header value carrying vendor-specific trace data.
    pub tracestate: Option<String>,
}

impl TraceContext {
    /// Extract from an HTTP header map; returns `None` if `traceparent` is absent.
    pub fn from_headers(headers: &HashMap<String, String>) -> Option<Self>;
    /// Inject `traceparent` (and `tracestate` if present) into a header map.
    pub fn inject_headers(&self, headers: &mut HashMap<String, String>);
}
```

---

### `Scope`

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    Global,
    Tenant      { tenant_id: String },
    Workflow    { workflow_id: String, tenant_id: Option<String> },
    Execution   { execution_id: String, workflow_id: Option<String>, tenant_id: Option<String> },
    Action      { action_id: String, execution_id: Option<String>, workflow_id: Option<String>, tenant_id: Option<String> },
    Custom      { key: String, value: String, parent: Option<Box<Scope>> },
}

impl Scope {
    // Fallible constructors — return Err if any ID string is empty.
    pub fn try_tenant(tenant_id: impl Into<String>) -> Result<Self, String>;
    pub fn try_workflow(workflow_id: impl Into<String>) -> Result<Self, String>;
    pub fn try_workflow_in_tenant(workflow_id: impl Into<String>, tenant_id: impl Into<String>) -> Result<Self, String>;
    pub fn try_execution(execution_id: impl Into<String>) -> Result<Self, String>;
    pub fn try_execution_in_workflow(
        execution_id: impl Into<String>,
        workflow_id: impl Into<String>,
        tenant_id: Option<String>,
    ) -> Result<Self, String>;
    pub fn try_action(action_id: impl Into<String>) -> Result<Self, String>;
    pub fn try_action_in_execution(
        action_id: impl Into<String>,
        execution_id: impl Into<String>,
        workflow_id: Option<String>,
        tenant_id: Option<String>,
    ) -> Result<Self, String>;
    /// Create a Custom scope without a parent.
    pub fn try_custom(key: impl Into<String>, value: impl Into<String>) -> Result<Self, String>;
    /// Create a Custom scope nested under `parent` for hierarchical containment checks.
    pub fn try_custom_with_parent(key: impl Into<String>, value: impl Into<String>, parent: Scope) -> Result<Self, String>;

    /// Numeric depth: Global=0, Tenant=1, Workflow=2, Execution=3, Action=4, Custom≥1.
    pub fn hierarchy_level(&self) -> u8;
    pub fn is_broader_than(&self, other: &Scope) -> bool;
    pub fn is_narrower_than(&self, other: &Scope) -> bool;

    /// Returns true if `self` transitively contains `other`
    /// (i.e. `other` is a descendant in the hierarchy).
    pub fn contains(&self, other: &Scope) -> bool;

    pub fn scope_key(&self) -> String;
    pub fn description(&self) -> String;
}
```

---

### `Strategy`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Strategy {
    /// Exact scope match only.
    Strict,
    /// Pool scope must contain (or equal) the requested scope. Default.
    #[default]
    Hierarchical,
    /// Exact match first; falls back to hierarchical if no exact match.
    Fallback,
}

impl Strategy {
    pub fn is_compatible(&self, resource_scope: &Scope, requested_scope: &Scope) -> bool;
}
```

---

## Metadata

### `ResourceMetadata`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceMetadata {
    /// Canonical identifier used everywhere in the system.
    pub key: ResourceKey,
    pub name: String,
    pub description: String,
    pub icon: Option<String>,
    pub icon_url: Option<String>,
    pub tags: Vec<String>,
}

impl ResourceMetadata {
    pub fn new(key: ResourceKey, name: impl Into<String>, description: impl Into<String>) -> Self;
    pub fn from_key(key: ResourceKey) -> Self;

    /// Start a builder (preferred over direct construction).
    pub fn builder(key: ResourceKey, name: impl Into<String>, description: impl Into<String>) -> ResourceMetadataBuilder;
}

pub struct ResourceMetadataBuilder { /* ... */ }

impl ResourceMetadataBuilder {
    pub fn icon(self, icon: impl Into<String>) -> Self;
    pub fn icon_url(self, icon_url: impl Into<String>) -> Self;
    pub fn tag(self, tag: impl Into<String>) -> Self;
    pub fn tags<T, I>(self, tags: I) -> Self
    where T: Into<String>, I: IntoIterator<Item = T>;
    pub fn build(self) -> ResourceMetadata;
}
```

---

## Error Handling

### `Error`

```rust
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("configuration error: {message}")]
    Configuration {
        message: String,
        #[source] source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("resource '{resource_key}' failed to initialise: {reason}")]
    Initialization {
        resource_key: ResourceKey,
        reason: String,
        #[source] source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("resource '{resource_key}' unavailable: {reason}")]
    Unavailable {
        resource_key: ResourceKey,
        reason: String,
        retryable: bool,
    },

    #[error("health check failed for '{resource_key}' (attempt {attempt}): {reason}")]
    HealthCheck {
        resource_key: ResourceKey,
        reason: String,
        attempt: u32,
    },

    #[error("no credential configured for resource '{resource_key}'")]
    CredentialNotConfigured { resource_key: ResourceKey },

    #[error("credential '{credential_id}' not found for resource '{resource_key}'")]
    MissingCredential {
        credential_id: String,
        resource_key: ResourceKey,
    },

    #[error("resource '{resource_key}' timed out after {timeout_ms}ms during {operation}")]
    Timeout {
        resource_key: ResourceKey,
        timeout_ms: u64,
        operation: String,
    },

    #[error("circuit breaker open for '{resource_key}' operation '{operation}'")]
    CircuitBreakerOpen {
        resource_key: ResourceKey,
        operation: &'static str,
        retry_after: Option<Duration>,
    },

    #[error("pool exhausted for '{resource_key}': {current_size}/{max_size} active, {waiters} waiting")]
    PoolExhausted {
        resource_key: ResourceKey,
        current_size: usize,
        max_size: usize,
        waiters: usize,
    },

    #[error("dependency '{dependency_id}' failed for '{resource_key}': {reason}")]
    DependencyFailure {
        resource_key: ResourceKey,
        dependency_id: String,
        reason: String,
    },

    #[error("circular dependency detected: {cycle}")]
    CircularDependency { cycle: String },

    #[error("invalid state transition for '{resource_key}': {from} → {to}")]
    InvalidStateTransition {
        resource_key: ResourceKey,
        from: String,
        to: String,
    },

    #[error("validation failed")]
    Validation { violations: Vec<FieldViolation> },

    #[error("internal error for '{resource_key}': {message}")]
    Internal {
        resource_key: ResourceKey,
        message: String,
        #[source] source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

impl Error {
    pub fn configuration(message: impl Into<String>) -> Self;
    pub fn validation(violations: Vec<FieldViolation>) -> Self;

    pub fn category(&self) -> ErrorCategory;
    pub fn is_retryable(&self) -> bool;
    pub fn is_fatal(&self) -> bool;
    pub fn is_validation(&self) -> bool;
    pub fn retry_after(&self) -> Option<Duration>;
    pub fn operation(&self) -> Option<&'static str>;
    pub fn resource_key(&self) -> Option<&ResourceKey>;
}
```

---

### `ErrorCategory` and `FieldViolation`

```rust
pub enum ErrorCategory {
    /// Operation may succeed on retry. Apply resilience layer backoff.
    Retryable,
    /// Permanent failure. Do not retry; fail the node.
    Fatal,
    /// Invalid input, config, or contract. Fix the caller; do not retry.
    Validation,
}

pub struct FieldViolation {
    pub field: String,
    pub constraint: String,
    pub actual: String,
}

impl FieldViolation {
    pub fn new(
        field: impl Into<String>,
        constraint: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self;
}
```

---

## Lifecycle

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Lifecycle {
    #[default]
    Created,
    Initializing,
    Ready,
    InUse,
    Idle,
    Maintenance,
    Draining,
    Cleanup,
    Terminated,
    Failed,
}

impl Lifecycle {
    /// True if the instance can be handed to a caller (Ready | Idle).
    pub fn is_available(&self) -> bool;
    /// True if no further transitions are possible (Terminated | Failed).
    pub fn is_terminal(&self) -> bool;
    /// True for transient states (Initializing | Draining | Cleanup).
    pub fn is_transitional(&self) -> bool;
    /// True if acquire is valid (Ready | Idle).
    pub fn can_acquire(&self) -> bool;
    /// Validate a state transition against the state machine.
    pub fn can_transition_to(&self, target: Lifecycle) -> bool;
    /// All valid next states from this state.
    pub fn next_states(&self) -> &'static [Lifecycle];
    pub fn description(&self) -> &'static str;
}
```

---

## Resource References and Providers

### `ResourceRef<R>` and `ErasedResourceRef`

```rust
pub struct ResourceRef<R: Resource> {
    pub key: ResourceKey,
    // PhantomData<fn() -> R>
}

impl<R: Resource> ResourceRef<R> {
    /// Construct from a string key.
    pub fn new(key: &str) -> Result<Self>;
    /// Derive from Resource::declare_key — no string needed.
    pub fn of() -> Self;
    /// Erase the type parameter for storage in heterogeneous collections.
    pub fn erase(self) -> ErasedResourceRef;
}

pub struct ErasedResourceRef {
    pub key: ResourceKey,
}
```

---

### `ResourceProvider`

Trait implemented by `Manager` and test doubles. Enables type-safe resource
acquisition without a direct dependency on `Manager`.

```rust
pub trait ResourceProvider: Send + Sync {
    async fn resource<R: Resource>(&self, ctx: &Context) -> Result<Guard<R::Instance>>;
    async fn acquire(&self, id: &str, ctx: &Context) -> Result<Box<dyn Any + Send>>;

    // Default implementations (call acquire / resource):
    async fn has_resource<R: Resource>(&self, ctx: &Context) -> bool;
    async fn has(&self, id: &str, ctx: &Context) -> bool;
    async fn exists(&self, id: &str, ctx: &Context) -> bool;
    async fn exists_resource<R: Resource>(&self, ctx: &Context) -> bool;
}
```

---

## Components and Credentials

### `HasResourceComponents`

Declare static credential and sub-resource dependencies for a `Resource` type.
Used by the engine to pre-validate dependencies before execution.

```rust
pub trait HasResourceComponents: Resource {
    fn components() -> ResourceComponents;
}

pub struct ResourceComponents {
    // private fields
}

impl ResourceComponents {
    pub fn new() -> Self;

    /// Declare a required credential of type `C`.
    pub fn credential<C: nebula_credential::CredentialType>(self, id: &str) -> Self;

    /// Declare a sub-resource dependency.
    pub fn resource<R: Resource>(self, key: &str) -> Self;

    pub fn credential_ref(&self) -> Option<&ErasedCredentialRef>;
    pub fn resource_refs(&self) -> &[ErasedResourceRef];
}
```

---

## Observability

See [`events-and-hooks.md`](events-and-hooks.md) for the full `EventBus`,
`ResourceEvent`, and `HookRegistry` reference.

See [`health-and-quarantine.md`](health-and-quarantine.md) for `HealthChecker`,
`HealthState`, `QuarantineManager`, and `RecoveryStrategy`.

### `MetricsCollector`

Bridges pool stats to `nebula-metrics`. Registered automatically by `Manager`.

```rust
pub struct MetricsCollector;

impl MetricsCollector {
    pub fn record_acquire(key: &ResourceKey, wait_ms: u64);
    pub fn record_release(key: &ResourceKey, usage_ms: u64);
    pub fn record_create(key: &ResourceKey, success: bool);
    pub fn record_exhausted(key: &ResourceKey);
}
```

---

## Prelude

The prelude re-exports the most commonly used types:

```rust
use nebula_resource::prelude::*;
```

Includes: `Context`, `Error`, `ErrorCategory`, `Result`, `Guard`, `Lifecycle`,
`ResourceMetadata`, `ErasedResourceRef`, `ResourceProvider`, `ResourceRef`,
`Config`, `Resource`, `Scope`, `Strategy`, `AutoScalePolicy`,
`BackPressurePolicy`, `EventBus`, `EventBusStats`, `EventFilter`,
`EventSubscriber`, `QuarantineTrigger`, `ResourceEvent`, `ScopedEvent`,
`ScopedSubscriber`, `SubscriptionScope`, `HealthCheckable`, `HealthState`,
`HealthStatus`, `ResourceHealthAdapter`, `HookEvent`, `HookFilter`,
`HookRegistry`, `HookResult`, `ResourceHook`, `Manager`, `ManagerBuilder`,
`ResourceHandle`, `ResourcePoolStatus`, `ResourceStatus`,
`TypedResourceGuard`, `AdaptiveBackpressurePolicy`, `LatencyPercentiles`,
`Pool`, `PoolBackpressurePolicy`, `PoolConfig`, `PoolStats`, `PoolStrategy`,
`ExecutionId`, `ResourceId`, `ResourceKey`, `WorkflowId`.
