# nebula-resource — API Reference (post-cascade)

Public API reference for `nebula-resource`, copied verbatim from
`crates/resource/src/`. The crate re-exports a curated slice of `nebula-core`
and `nebula-credential` (per ADR-0036) so consumers do not need direct deps:

- From `nebula_core`: `ExecutionId`, `ResourceKey`, `ScopeLevel`, `WorkflowId`,
  `resource_key!`.
- From `nebula_credential`: `Credential`, `CredentialContext`, `CredentialId`,
  `NoCredential`, `NoCredentialState`, `SchemeGuard`.
- From `nebula_resource_macros`: `ClassifyError`, `Resource` derives.

For the event catalog see [`events.md`](events.md). For pool internals see
[`pooling.md`](pooling.md). For recovery internals see
[`recovery.md`](recovery.md).

---

## Table of Contents

- [Core Traits](#core-traits)
- [Topology Traits](#topology-traits)
- [Topology Configs](#topology-configs)
- [Handle](#handle)
- [Manager](#manager)
- [Manager Options](#manager-options)
- [Error Model](#error-model)
- [Context](#context)
- [Acquire Options](#acquire-options)
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

Five associated types and six lifecycle methods. Uses return-position
`impl Future` (RPITIT) — no `Box<dyn Future>` per call.

```rust,ignore
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;
    type Lease: Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static;
    type Credential: Credential;        // ADR-0036 — `NoCredential` opts out

    fn key() -> ResourceKey;

    fn create(
        &self,
        config: &Self::Config,
        scheme: &<Self::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        ctx: &'a CredentialContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
        let _ = (new_scheme, ctx);
        async { Ok(()) }
    }

    fn on_credential_revoke(
        &self,
        credential_id: &CredentialId,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = credential_id;
        async { Ok(()) }
    }

    fn check(&self, runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { /* default: Ok */ }
    fn shutdown(&self, runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { /* default: no-op */ }
    fn destroy(&self, runtime: Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { /* default: drop */ }

    fn schema()   -> nebula_schema::ValidSchema where Self: Sized;
    fn metadata() -> ResourceMetadata           where Self: Sized;
}
```

**Lifecycle:** `create → check (periodic) → on_credential_refresh /
on_credential_revoke (rotation) → shutdown → destroy`.

`on_credential_refresh` and `on_credential_revoke` are default-no-op hooks per
ADR-0036. Connection-bound resources (Pool, Service, Transport) override
refresh with the blue-green pool swap (Tech Spec §15.7); revoke must guarantee
no further authenticated traffic on the revoked credential. `SchemeGuard` is
`ZeroizeOnDrop` and tied to the call lifetime — never retain it past the call.

### `ResourceConfig`

Operational config (no secrets). `HasSchema` super-bound lets
`ResourceMetadata::for_resource` auto-derive the config schema.

```rust,ignore
pub trait ResourceConfig: nebula_schema::HasSchema + Send + Sync + Clone + 'static {
    fn validate(&self) -> Result<(), crate::Error> { Ok(()) }
    fn fingerprint(&self) -> u64 { 0 }
}
```

`fingerprint` powers hot-reload short-circuiting — `Manager::reload_config`
returns `ReloadOutcome::NoChange` when fingerprints match. Default `0`
deduplicates nothing; override with a stable hash of behaviour-affecting fields.

### `ResourceMetadata`

Catalog metadata composed over `nebula_metadata::BaseMetadata`. Resource has no
entity-specific metadata fields today — every catalog concern (key, name,
description, schema, version, tags, maturity, deprecation) lives on the base.

```rust,ignore
#[non_exhaustive]
pub struct ResourceMetadata {
    pub base: nebula_metadata::BaseMetadata<ResourceKey>,
}
```

| Constructor | Notes |
|-------------|-------|
| `ResourceMetadata::new(key, name, description, schema)` | Explicit schema |
| `ResourceMetadata::for_resource::<R>(key, name, description)` | Schema auto-derived from `R::Config` via `HasSchema` |
| `ResourceMetadata::from_key(key)` | Minimal — key as name, empty schema |
| `ResourceMetadata::builder(key, name, description)` | Returns `ResourceMetadataBuilder` (chains `with_schema`, `with_version(major, minor)`, `build()`) |

`validate_compatibility(&self, previous: &Self)` enforces catalog-citizen rules
(key immutable / version monotonic / schema-break-requires-major) via
`nebula_metadata::validate_base_compat`. `MetadataCompatibilityError` is
`#[non_exhaustive]` with one variant today: `Base(BaseCompatError<ResourceKey>)`.

### `AnyResource`

Trait-object-safe marker for type-erased registration: `fn key(&self) ->
ResourceKey` and `fn metadata(&self) -> ResourceMetadata`.

---

## Topology Traits

Each topology trait extends `Resource` with topology-specific lifecycle hooks.
Five topologies cover the full integration space.

### `Pooled`

```rust,ignore
pub trait Pooled: Resource {
    fn is_broken(&self, _runtime: &Self::Runtime) -> BrokenCheck { BrokenCheck::Healthy }
    fn recycle(&self, _runtime: &Self::Runtime, _metrics: &InstanceMetrics)
        -> impl Future<Output = Result<RecycleDecision, Self::Error>> + Send { async { Ok(RecycleDecision::Keep) } }
    fn prepare(&self, _runtime: &Self::Runtime, _ctx: &ResourceContext)
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }
}
```

`is_broken` runs in `Drop` — sync, O(1), no I/O. `recycle` is async, called on
return-to-pool. `prepare` runs after checkout, before the lease reaches the
caller (e.g., `SET search_path`). `BrokenCheck`: `Healthy | Broken(String)`.
`RecycleDecision`: `Keep | Drop`.

### `Resident`

```rust,ignore
pub trait Resident: Resource where Self::Lease: Clone {
    fn is_alive_sync(&self, _runtime: &Self::Runtime) -> bool { true }
    fn stale_after(&self) -> Option<Duration> { None }
}
```

Single shared instance, clone on acquire (e.g., `reqwest::Client`).

### `Service`

```rust,ignore
pub trait Service: Resource {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    fn acquire_token(&self, runtime: &Self::Runtime, ctx: &ResourceContext)
        -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;
    fn release_token(&self, _runtime: &Self::Runtime, _token: Self::Lease)
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }
}
```

Long-lived runtime, short-lived tokens. `TokenMode::Cloned` (default; release
no-op, owned handle) or `TokenMode::Tracked` (release required, guarded handle).

### `Transport`

```rust,ignore
pub trait Transport: Resource {
    fn open_session(&self, transport: &Self::Runtime, ctx: &ResourceContext)
        -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;
    fn close_session(&self, _transport: &Self::Runtime, _session: Self::Lease, _healthy: bool)
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }
    fn keepalive(&self, _transport: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }
}
```

Shared connection multiplexes short-lived sessions (HTTP/2, gRPC, AMQP).

### `Exclusive`

```rust,ignore
pub trait Exclusive: Resource {
    fn reset(&self, _runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }
}
```

One caller at a time via semaphore(1). `reset` runs before the next acquire
(#384). Suitable for non-concurrency-safe resources (serial ports, single-writer DBs).

> **Daemon / EventSource:** these traits no longer live in `nebula-resource`.
> Per ADR-0037 / cascade П3, autonomous background workloads moved into
> `nebula_engine::daemon` (EventSource folded into Daemon).

---

## Topology Configs

| Config | Topology | Key fields (defaults) |
|--------|----------|------------------------|
| `PoolConfig` | `Pooled` | `min_size: u32 = 1`, `max_size: u32 = 10`, `idle_timeout: Option<Duration> = Some(5 min)`, `max_lifetime: Option<Duration> = Some(30 min)`, `create_timeout: Duration = 30s`, `strategy: PoolStrategy = Lifo`, `warmup: WarmupStrategy = None`, `test_on_checkout: bool = false`, `maintenance_interval: Duration = 30s`, `max_concurrent_creates: u32 = 3` |
| `ResidentConfig` | `Resident` | `recreate_on_failure: bool = false`, `create_timeout: Duration = 30s` |
| `ServiceConfig` | `Service` | `drain_timeout: Option<Duration> = None` |
| `TransportConfig` | `Transport` | `max_sessions: u32 = 10`, `keepalive_interval: Option<Duration> = Some(30s)`, `acquire_timeout: Duration = 30s` |
| `ExclusiveConfig` | `Exclusive` | `acquire_timeout: Duration = 30s` |

`PoolStrategy`: `Lifo` (reuse most recent) or `Fifo` (spread load).
`WarmupStrategy`: `None`, `Sequential`, `Parallel`, `Staggered { interval }`.

---

## Handle

### `ResourceGuard<R>`

`#[must_use]` RAII guard. Dereferences to `R::Lease`. Three modes:

| Mode | Constructor | Drop |
|------|-------------|------|
| Owned | `ResourceGuard::owned(lease, key, tag)` | no-op |
| Guarded | `ResourceGuard::guarded(lease, key, tag, generation, on_release)` (variant `guarded_with_permit(.., Option<OwnedSemaphorePermit>)` holds the permit so it returns even if the release callback panics) | calls `on_release(lease, tainted)` |
| Shared | `ResourceGuard::shared(Arc<lease>, key, tag, generation, on_release)` | calls `on_release(tainted)` |

Methods: `taint(&mut self)` (mark for destroy; Owned no-op), `detach(self) ->
Option<R::Lease>` (None for Shared), `hold_duration() -> Duration`,
`resource_key() -> &ResourceKey`, `topology_tag() -> TopologyTag`,
`generation() -> Option<u64>` (None for Owned).

Drop is panic-safe — release callbacks run inside `catch_unwind`; semaphore
permits return even if the callback panics. Implements `nebula_core::Guard`
and `TypedGuard`.

---

## Manager

`Manager` owns the registry, recovery groups, release queue, and a shared
`CancellationToken`. Share via `Arc<Manager>` across tasks.

### Construction

```rust,ignore
impl Manager {
    pub fn new() -> Self;
    pub fn with_config(config: ManagerConfig) -> Self;
    pub fn with_lifecycle(self, lifecycle: LayerLifecycle) -> Self;
    pub fn lifecycle(&self) -> Option<&LayerLifecycle>;
    pub fn subscribe_events(&self) -> broadcast::Receiver<ResourceEvent>;
}
```

`subscribe_events` returns a 256-event-buffer receiver; slow consumers get
`RecvError::Lagged`. See [`events.md`](events.md) for the catalog.

### Register

11 methods total: 1 full `register` plus 5 + 5 shorthands. `register` takes
positional resilience / recovery_gate / credential_id / rotation timeout; the
`register_*_with` shorthands consolidate those into `RegisterOptions`.

```rust,ignore
// Full — fully positional, supports any Credential type.
pub fn register<R: Resource>(
    &self,
    resource: R,
    config: R::Config,
    scope: ScopeLevel,
    topology: TopologyRuntime<R>,
    resilience: Option<AcquireResilience>,
    recovery_gate: Option<Arc<RecoveryGate>>,
    credential_id: Option<CredentialId>,
    credential_rotation_timeout: Option<Duration>,
) -> Result<(), Error>;

// 5 no-credential convenience methods (all bound `R::Credential = NoCredential`):
pub fn register_pooled<R>(&self, resource: R, config: R::Config, pool_config: PoolConfig)         -> Result<(), Error>;
pub fn register_resident<R>(&self, resource: R, config: R::Config, resident_config: ResidentConfig) -> Result<(), Error>;
pub fn register_service<R>(&self, resource: R, config: R::Config, runtime: R::Runtime, service_config: ServiceConfig)         -> Result<(), Error>;
pub fn register_transport<R>(&self, resource: R, config: R::Config, runtime: R::Runtime, transport_config: TransportConfig) -> Result<(), Error>;
pub fn register_exclusive<R>(&self, resource: R, config: R::Config, runtime: R::Runtime, exclusive_config: ExclusiveConfig) -> Result<(), Error>;

// 5 _with shorthands taking RegisterOptions (also bound NoCredential today):
pub fn register_pooled_with<R>(&self, resource: R, config: R::Config, pool_config: PoolConfig, options: RegisterOptions)         -> Result<(), Error>;
pub fn register_resident_with<R>(&self, resource: R, config: R::Config, resident_config: ResidentConfig, options: RegisterOptions) -> Result<(), Error>;
pub fn register_service_with<R>(&self, resource: R, config: R::Config, runtime: R::Runtime, service_config: ServiceConfig, options: RegisterOptions)         -> Result<(), Error>;
pub fn register_transport_with<R>(&self, resource: R, config: R::Config, runtime: R::Runtime, transport_config: TransportConfig, options: RegisterOptions) -> Result<(), Error>;
pub fn register_exclusive_with<R>(&self, resource: R, config: R::Config, runtime: R::Runtime, exclusive_config: ExclusiveConfig, options: RegisterOptions) -> Result<(), Error>;
```

`register` validates the credential-binding contract before registry mutation:
a credential-bearing resource (`R::Credential != NoCredential`) registered
without a `credential_id` returns `Error::missing_credential_id` and the
registry is not touched.

### Acquire

10 methods total: 5 full (require scheme material) plus 5 `_default`
shorthands (only available for `R::Credential = NoCredential`).

```rust,ignore
pub async fn acquire_pooled<R>(
    &self,
    scheme: &<R::Credential as Credential>::Scheme,
    ctx: &ResourceContext,
    options: &AcquireOptions,
) -> Result<ResourceGuard<R>, Error>
where R: Pooled + Clone + Send + Sync + 'static, /* + topology bounds */;

pub async fn acquire_pooled_default<R>(&self, ctx: &ResourceContext, options: &AcquireOptions)
    -> Result<ResourceGuard<R>, Error>
where R: Pooled<Credential = NoCredential> + /* + topology bounds */;
```

The other four topologies have the same shape, each with a `_default`
overload: `acquire_resident` / `acquire_service` / `acquire_transport` /
`acquire_exclusive`.

Non-blocking pool variants — `try_acquire_pooled(scheme, ctx, options)` and
`try_acquire_pooled_default(ctx, options)` — return `ErrorKind::Backpressure`
immediately when all `max_size` slots are in use, never queueing.

### Pool warmup, stats, reload, remove, shutdown, lookup

```rust,ignore
pub async fn warmup_pool<R>(&self, scheme, ctx) -> Result<usize, Error>;
pub async fn warmup_pool_no_credential<R>(&self, ctx) -> Result<usize, Error>
    where R: Pooled<Credential = NoCredential> + ...;

pub async fn pool_stats<R>(&self, scope: &ScopeLevel) -> Option<PoolStats>
    where R: Pooled + ...;

pub fn reload_config<R: Resource>(&self, new_config: R::Config, scope: &ScopeLevel) -> Result<ReloadOutcome, Error>;
pub fn remove(&self, key: &ResourceKey) -> Result<(), Error>;

pub fn shutdown(&self);                                                              // immediate cancel
pub async fn graceful_shutdown(&self, config: ShutdownConfig) -> Result<ShutdownReport, ShutdownError>;

pub fn lookup<R: Resource>(&self, scope: &ScopeLevel) -> Result<Arc<ManagedResource<R>>, Error>;
pub fn contains(&self, key: &ResourceKey) -> bool;
pub fn keys(&self) -> Vec<ResourceKey>;
pub fn get_any(&self, key: &ResourceKey, scope: &ScopeLevel) -> Option<Arc<dyn AnyManagedResource>>;
pub fn health_check<R: Resource>(&self, scope: &ScopeLevel) -> Result<ResourceHealthSnapshot, Error>;

pub fn metrics(&self)         -> Option<&ResourceOpsMetrics>;
pub fn recovery_groups(&self) -> &RecoveryGroupRegistry;
pub fn cancel_token(&self)    -> &CancellationToken;
pub fn is_shutdown(&self)     -> bool;
```

Notes:
- Two warmup methods by design (Tech Spec §5.2 / security amendment B-3): the
  no-credential path passes `&()` directly — never calls `Scheme::default()`,
  killing the "warm with empty credential → 401 storm" footgun.
- `pool_stats` returns `None` when not registered or not Pool topology.
- `reload_config` short-circuits on fingerprint match (`NoChange`); otherwise
  atomically swaps the config and bumps the generation counter. Pool topology
  also updates the pool fingerprint so stale idle instances evict on next
  acquire/release. Service topology returns `PendingDrain { old_generation }`;
  others return `SwappedImmediately`.
- `remove` prunes the credential rotation reverse-index so future refresh /
  revoke does not fan out to a removed resource.
- `graceful_shutdown` is CAS-guarded — second concurrent caller gets
  `ShutdownError::AlreadyShuttingDown`.
- `lookup` rejects new acquires once shutdown begins
  (`ErrorKind::Cancelled`). Scope-aware: exact match, falls back to `Global`.
- `ResourceHealthSnapshot { key, phase, gate_state, metrics, generation }` —
  `metrics` is `Some` only when a metrics registry was configured.

---

## Manager Options

### `ManagerConfig`

| Field | Default | Purpose |
|-------|---------|---------|
| `release_queue_workers: usize` | `2` | Background workers for async cleanup |
| `metrics_registry: Option<Arc<MetricsRegistry>>` | `None` | Telemetry registry; `None` skips metrics with zero overhead |
| `credential_rotation_timeout: Duration` | `30s` | Default per-resource FULL rotation dispatch budget (covers `SchemeFactory::acquire` + the resource hook). Overridable per-resource via `RegisterOptions`. |

### `RegisterOptions` (`#[non_exhaustive]`)

| Field | Default | Purpose |
|-------|---------|---------|
| `scope: ScopeLevel` | `Global` | Scope key |
| `resilience: Option<AcquireResilience>` | `None` | Timeout + retry policy on acquire |
| `recovery_gate: Option<Arc<RecoveryGate>>` | `None` | Thundering-herd prevention |
| `credential_id: Option<CredentialId>` | `None` | Required for credential-bearing resources; populates rotation reverse-index |
| `credential_rotation_timeout: Option<Duration>` | `None` (falls back to `ManagerConfig`) | Per-resource budget override |

Builders: `with_credential_id(id) -> Self`, `with_rotation_timeout(timeout) -> Self`.

### `ShutdownConfig`, `DrainTimeoutPolicy`, `ShutdownReport`, `ShutdownError`

`ShutdownConfig` is `#[non_exhaustive]`. Defaults: `drain_timeout: 30s`,
`on_drain_timeout: DrainTimeoutPolicy::Abort`, `release_queue_timeout: 10s`.
Builders: `with_drain_timeout`, `with_drain_timeout_policy`,
`with_release_queue_timeout`.

`DrainTimeoutPolicy` (`#[non_exhaustive]`, `#[default] = Abort`):

- `Abort` — return `ShutdownError::DrainTimeout` without clearing the registry;
  live handles remain valid; every resource is transitioned to `Failed`
  (R-023).
- `Force` — log, clear anyway, report `outstanding_handles_after_drain` in the
  report. Opt-in escape hatch.

`ShutdownReport` (`#[non_exhaustive]`): `outstanding_handles_after_drain: u64`,
`registry_cleared: bool`, `release_queue_drained: bool`.

`ShutdownError` (`#[non_exhaustive]`): `AlreadyShuttingDown`,
`DrainTimeout { outstanding: u64 }`, `ReleaseQueueTimeout { timeout: Duration }`.

---

## Error Model

### `Error`

Unified error carrying kind + scope + message + optional resource key +
optional source. Constructors: `Error::new(kind, message)`, `transient`,
`permanent`, `exhausted(message, retry_after: Option<Duration>)`,
`backpressure`, `not_found(&ResourceKey)`, `cancelled`,
`missing_credential_id(ResourceKey)`, `scheme_type_mismatch::<R>()`. Builders:
`with_resource_key`, `with_source`, `with_scope`. Accessors: `kind()`,
`scope()`, `resource_key()`, `is_retryable()`, `retry_after()`.

`is_retryable()` returns `true` for `Transient`, `Exhausted`, `Backpressure`.
`retry_after()` returns the explicit hint for `Exhausted`, default `50ms` for
`Backpressure`, otherwise `None`. Implements `nebula_error::Classify` so
resource errors flow through the workspace error category / code / retry-hint
pipeline.

### `ErrorKind` and `ErrorScope`

```rust,ignore
#[non_exhaustive]
pub enum ErrorKind {
    Transient, Permanent,
    Exhausted { retry_after: Option<Duration> },
    Backpressure, NotFound, Cancelled,
}

#[non_exhaustive]
pub enum ErrorScope { Resource /* default */, Target { id: String } }
```

### Rotation outcomes

```rust,ignore
pub enum RefreshOutcome { Ok, Failed(Error), TimedOut { budget: Duration } }
pub enum RevokeOutcome  { Ok, Failed(Error), TimedOut { budget: Duration } }

pub struct RotationOutcome {
    pub ok: usize,
    pub failed: usize,
    pub timed_out: usize,
}
impl RotationOutcome {
    pub fn total(&self) -> usize;
    pub fn has_partial_failure(&self) -> bool;
}
```

Produced one-per-resource by the rotation dispatcher and folded into
`RotationOutcome` for `ResourceEvent::CredentialRefreshed` / `CredentialRevoked`.

### `ClassifyError` derive

Re-exported from `nebula-resource-macros`. Generates `From<T> for
nebula_resource::Error` from per-variant `#[classify(transient | permanent |
exhausted)]` attributes. See the macro's rustdoc for syntax.

---

## Context

### `ResourceContext`

Embeds `BaseContext` for identity / scope / cancellation; holds
`Arc<dyn ResourceAccessor>` and `Arc<dyn CredentialAccessor>` for capability
resolution.

```rust,ignore
impl ResourceContext {
    pub fn new(
        base: BaseContext,
        resources: Arc<dyn ResourceAccessor>,
        credentials: Arc<dyn CredentialAccessor>,
    ) -> Self;

    /// Minimal — scope + cancellation only (warmup, daemon loops). No-op accessors.
    pub fn minimal(scope: Scope, cancellation: CancellationToken) -> Self;

    pub fn scope_level(&self)  -> ScopeLevel;          // Execution > Workflow > Workspace > Organization > Global
    pub fn cancel_token(&self) -> &CancellationToken;
    pub fn execution_id(&self) -> Option<ExecutionId>;
}
```

Implements `nebula_core::context::Context`, `HasResources` (`fn resources(&self)
-> &dyn ResourceAccessor`), and `HasCredentials` (`fn credentials(&self) ->
&dyn CredentialAccessor`).

---

## Acquire Options

### `AcquireOptions`

```rust,ignore
pub struct AcquireOptions {
    pub intent:   AcquireIntent,
    pub deadline: Option<Instant>,
    pub tags:     SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]>,
}
```

Builders: `with_deadline`, `with_intent`, `with_tag`. Accessor: `remaining()
-> Option<Duration>`.

> **Status:** `intent` and `tags` are reserved for future engine integration.
> No topology in `nebula-resource` reads them today (#391). Setting
> `AcquireIntent::Critical` does NOT bypass queues or change throttling — only
> `deadline` affects acquire behaviour. Per Strategy §5.2, deprecation /
> removal will be considered in a future cascade if uptake remains zero.

### `AcquireIntent`

`#[non_exhaustive]` enum: `Standard`, `LongRunning`, `Streaming { expected:
Duration }`, `Prefetch`, `Critical`.

---

## Resilience

### `AcquireResilience` and `AcquireRetryConfig`

```rust,ignore
pub struct AcquireResilience {
    pub timeout: Option<Duration>,
    pub retry:   Option<AcquireRetryConfig>,
}

pub struct AcquireRetryConfig {
    pub max_attempts:    u32,       // total tries (initial + retries); clamped >= 1
    pub initial_backoff: Duration,
    pub max_backoff:     Duration,  // caps exponential growth (multiplier 2.0)
}
```

Two fields only — no circuit breaker. Three presets: `standard()` (30 s
timeout / `max_attempts = 3` = 1 initial + 2 retries), `fast()` (10 s /
`max_attempts = 2`), `slow()` (60 s / `max_attempts = 5`); `none()`
disables both. Internally converts to `nebula_resilience::RetryConfig`
with exponential backoff (2× multiplier). A user-supplied
`max_attempts: 0` clamps to `1` instead of panicking (#383).

---

## Recovery

CAS-based serializer prevents thundering herd on dead backends. Deeper
semantics live in [`recovery.md`](recovery.md).

### `RecoveryGate` and `RecoveryGateConfig`

```rust,ignore
pub fn RecoveryGate::new(config: RecoveryGateConfig) -> Self;
pub fn RecoveryGate::try_begin(&self) -> Result<RecoveryTicket, TryBeginError>;
pub fn RecoveryGate::state(&self)     -> GateState;
pub fn RecoveryGate::reset(&self);
```

`RecoveryGateConfig`: `max_attempts: u32 = 5`, `base_backoff: Duration = 1s`.
Backoff doubles each attempt, capped at 5 minutes internally.

### `GateState` and `TryBeginError`

```rust,ignore
#[non_exhaustive]
pub enum GateState {
    Idle,
    InProgress       { attempt: u32 },
    Failed           { message: String, retry_at: Instant, attempt: u32 },
    PermanentlyFailed { message: String },
}

pub enum TryBeginError {
    AlreadyInProgress(RecoveryWaiter),
    RetryLater       { retry_at: Instant },
    PermanentlyFailed { message: String },
}
```

### `RecoveryTicket` and `RecoveryWaiter`

`RecoveryTicket` is `#[must_use]`. Methods: `resolve(self)`,
`fail_transient(self, message)`, `fail_permanent(self, message)`,
`attempt(&self) -> u32`. Drop without resolve auto-fails into transient with
backoff. `RecoveryWaiter` (returned inside `TryBeginError::AlreadyInProgress`)
awaits the in-flight attempt's outcome.

### `RecoveryGroupRegistry` and `RecoveryGroupKey`

```rust,ignore
pub fn RecoveryGroupRegistry::new() -> Self;
pub fn get_or_create(&self, key: RecoveryGroupKey, config: RecoveryGateConfig) -> Arc<RecoveryGate>;
pub fn get(&self, key: &RecoveryGroupKey)    -> Option<Arc<RecoveryGate>>;
pub fn remove(&self, key: &RecoveryGroupKey) -> Option<Arc<RecoveryGate>>;
pub fn len(&self) -> usize;
pub fn is_empty(&self) -> bool;
```

`RecoveryGroupKey::new(impl Into<String>)`, `as_str()`.

### `WatchdogHandle` and `WatchdogConfig`

```rust,ignore
pub fn WatchdogHandle::start<F, Fut>(
    config: WatchdogConfig,
    check_fn: F,
    on_health_change: impl Fn(bool) + Send + Sync + 'static,
    parent_cancel: CancellationToken,
) -> Self
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), crate::Error>> + Send;
```

Drop cancels the background task. `WatchdogConfig` defaults: `interval: 30s`,
`probe_timeout: 5s`, `failure_threshold: 3`, `recovery_threshold: 1`.

---

## Events

`ResourceEvent` is `#[non_exhaustive]` with **12 variants** today. Cheap to
clone. See [`events.md`](events.md) for the canonical catalog (variant table,
emission contracts, key recovery semantics).

```rust,ignore
pub fn Manager::subscribe_events(&self) -> tokio::sync::broadcast::Receiver<ResourceEvent>;

impl ResourceEvent {
    pub fn key(&self) -> Option<&ResourceKey>;
    // Some for the 10 per-resource variants;
    // None for CredentialRefreshed / CredentialRevoked (use credential_id field).
}
```

---

## Metrics

`ResourceOpsMetrics` is a cheap-clone handle into the shared `MetricsRegistry`.
Construct via `ResourceOpsMetrics::new(&registry)` or implicitly through
`ManagerConfig::metrics_registry`. `snapshot()` returns `ResourceOpsSnapshot`
(Copy):

| `ResourceOpsSnapshot` field | Type |
|------------------------------|------|
| `acquire_total`, `acquire_errors`, `release_total`, `create_total`, `destroy_total` | `u64` |
| `rotation_attempts`, `revoke_attempts`, `rotation_dispatch_latency_count` | `OutcomeCountersSnapshot` |
| `credential_rotation_skipped` | `u64` |

`OutcomeCountersSnapshot` mirrors `nebula_metrics::naming::rotation_outcome`
closed labels: `success`, `failed`, `timed_out` (each `u64`).

---

## State

```rust,ignore
#[non_exhaustive]
pub enum ResourcePhase {
    Initializing, Ready, Reloading, Draining, ShuttingDown, Failed,
}

pub struct ResourceStatus {
    pub phase: ResourcePhase,
    pub generation: u64,
    pub last_error: Option<String>,
}
```

`ResourcePhase` methods: `is_accepting()` (true for `Ready`, `Reloading`),
`is_terminal()` (true for `ShuttingDown`, `Failed`). `Display` produces
snake_case (`"shutting_down"`, etc.). `ResourceStatus::new()` starts in
`Initializing` with `generation: 0`, `last_error: None`.

---

## Runtime Types

### `TopologyRuntime<R>`

Dispatch enum holding the runtime state for a registered resource. **5
variants** (post-П3):

```rust,ignore
pub enum TopologyRuntime<R: Resource> {
    Pool(PoolRuntime<R>),
    Resident(ResidentRuntime<R>),
    Service(ServiceRuntime<R>),
    Transport(TransportRuntime<R>),
    Exclusive(ExclusiveRuntime<R>),
}

impl<R: Resource> TopologyRuntime<R> {
    pub fn tag(&self) -> TopologyTag;
}
```

Daemon and EventSource are NOT variants of `TopologyRuntime` — they live in
`nebula_engine::daemon` per ADR-0037.

### `ManagedResource<R>`, `Registry`, `AnyManagedResource`, `PoolStats`

`ManagedResource<R>` is the per-registration handle the registry stores —
holds the resource, `ArcSwap<R::Config>` for hot-reload, the topology runtime,
the manager's release queue, generation counter, status cell, optional
resilience / recovery gate / credential ID. Constructed inside `register()`;
fetched typed via `Manager::lookup` or type-erased via `Manager::get_any`.

`Registry` is type-erased storage with two indexes (primary by
`ResourceKey + ScopeLevel`, secondary by `TypeId`). Methods: `register`,
`get`, `get_typed::<R>`, `remove`, `keys`, `contains`, `clear`.

`AnyManagedResource` is the trait every `ManagedResource<R>` implements:
`resource_key`, `as_any_arc`, `managed_type_id`, `set_phase_erased`,
`set_failed_erased`, `phase_erased`.

`PoolStats` is returned from `Manager::pool_stats` — see [`pooling.md`](pooling.md).

---

## Utilities

### `Cell<T>`

Lock-free `ArcSwap`-based cell for resident topologies. Methods: `new()`,
`store(Arc<T>)`, `load() -> Option<Arc<T>>`, `take() -> Option<Arc<T>>`,
`is_some()`.

### `ReleaseQueue`

Background worker pool for async resource cleanup. **Best-effort on crash** per
canon §11.4 — orphaned resources rely on the next process to drain. The manager
owns one internally; callers never construct one directly. Diagnostic
accessors: `fallback_count`, `dropped_count`, `rescued_count`.

### `ReloadOutcome`

`SwappedImmediately`, `PendingDrain { old_generation: u64 }`, `Restarting`,
`NoChange`. Returned from `Manager::reload_config`. `Restarting` is reserved
for engine-side daemon dispatch (per ADR-0037 daemons live in
`nebula_engine::daemon`).

### `TopologyTag`

```rust,ignore
#[non_exhaustive]
pub enum TopologyTag { Pool, Resident, Service, Transport, Exclusive }

impl TopologyTag {
    pub fn as_str(self) -> &'static str;  // "pool" | "resident" | "service" | "transport" | "exclusive"
}
// Display delegates to as_str.
```

5 variants (post-П3 / ADR-0037).
