//! Resource pool — generic pool integrated with the `Resource` trait.
//!
//! `Pool<R>` calls `R::create`, `R::is_valid`, `R::recycle` and `R::cleanup`
//! directly, removing the need for closure factories.
//!
//! ## Lifecycle Hooks
//!
//! When a [`HookRegistry`] is attached (via [`Pool::with_hooks`]), the pool
//! fires [`HookEvent::Create`] before/after [`Resource::create()`] and
//! [`HookEvent::Cleanup`] before/after [`Resource::cleanup()`]. Before-hooks
//! can cancel create operations; cleanup hooks are best-effort (errors are
//! logged but never propagated).
//!
//! [`Resource::create()`]: crate::Resource::create
//! [`Resource::cleanup()`]: crate::Resource::cleanup

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use hdrhistogram::Histogram;
use nebula_core::{ExecutionId, ResourceKey, WorkflowId};
use nebula_credential::traits::RotationStrategy;
use nebula_resilience::{
    CircuitBreaker, CircuitBreakerConfig, CircuitState, Gate, ResilienceError,
};
use parking_lot::{Mutex, RwLock};
use tokio::sync::{Semaphore, TryAcquireError};
use tokio_util::sync::CancellationToken;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::events::{CleanupReason, EventBus, ResourceEvent};
use crate::guard::Guard;
use crate::hooks::{HookEvent, HookRegistry};
use crate::lifecycle::Lifecycle;
use crate::poison::{Poison, PoisonError};
use crate::resource::Resource;
use crate::scope::Scope;

// ---------------------------------------------------------------------------
// CounterGuard — RAII waiter-count tracker
// ---------------------------------------------------------------------------

/// RAII guard that increments an [`AtomicUsize`] on construction and
/// decrements it on drop.
///
/// Used in [`Pool::acquire_inner`] to track the number of callers currently
/// waiting for a semaphore permit, without relying on paired manual
/// `fetch_add` / `fetch_sub` calls that can be skipped on early returns.
struct CounterGuard(Arc<AtomicUsize>);

impl CounterGuard {
    fn new(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::Relaxed);
        Self(Arc::clone(counter))
    }
}

impl Drop for CounterGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// PoolConfig
// ---------------------------------------------------------------------------

use serde::{Deserialize, Serialize};

/// Strategy for selecting idle instances from the pool.
///
/// Controls whether the most-recently-used or least-recently-used
/// idle instance is returned on acquire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PoolStrategy {
    /// First-in, first-out: return the **oldest** idle instance.
    ///
    /// Distributes usage evenly across instances. This is the default.
    #[default]
    Fifo,
    /// Last-in, first-out: return the **most recently used** idle instance.
    ///
    /// Keeps a hot working set small, letting less-used instances idle-expire
    /// naturally. Useful when `min_size` is low relative to `max_size`.
    Lifo,
}

/// Backpressure policy for acquire behavior when the pool is saturated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PoolBackpressurePolicy {
    /// Immediately return [`Error::PoolExhausted`] when no permit is available.
    FailFast,
    /// Wait up to `timeout` for a permit, then return [`Error::PoolExhausted`].
    BoundedWait {
        /// Max wait time for permit acquisition.
        timeout: Duration,
    },
    /// Dynamically choose wait timeout based on current pressure.
    Adaptive(AdaptiveBackpressurePolicy),
}

/// Configuration for adaptive acquire backpressure behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdaptiveBackpressurePolicy {
    /// Utilization threshold (`active / max_size`) considered high pressure.
    pub high_pressure_utilization: f64,
    /// Waiter threshold considered high pressure.
    pub high_pressure_waiters: usize,
    /// Timeout used under low pressure.
    pub low_pressure_timeout: Duration,
    /// Timeout used under high pressure.
    pub high_pressure_timeout: Duration,
}

impl Default for AdaptiveBackpressurePolicy {
    fn default() -> Self {
        Self {
            high_pressure_utilization: 0.8,
            high_pressure_waiters: 8,
            low_pressure_timeout: Duration::from_secs(30),
            high_pressure_timeout: Duration::from_millis(100),
        }
    }
}

/// Configuration for resource pooling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolConfig {
    /// Minimum number of resources in the pool
    pub min_size: usize,
    /// Maximum number of resources in the pool
    pub max_size: usize,
    /// Timeout for acquiring a resource from the pool
    pub acquire_timeout: Duration,
    /// Time after which idle resources are removed
    pub idle_timeout: Duration,
    /// Maximum lifetime of a resource
    pub max_lifetime: Duration,
    /// Interval for validation/health checks
    pub validation_interval: Duration,
    /// If set, a background task calls `maintain()` at this interval.
    /// `None` disables automatic maintenance (the default).
    #[serde(default)]
    pub maintenance_interval: Option<Duration>,
    /// Strategy for selecting idle instances on acquire.
    /// Default: [`PoolStrategy::Fifo`].
    #[serde(default)]
    pub strategy: PoolStrategy,
    /// Optional acquire backpressure policy profile.
    ///
    /// When omitted, bounded-wait behavior is preserved using `acquire_timeout`
    /// for backward compatibility.
    #[serde(default)]
    pub backpressure_policy: Option<PoolBackpressurePolicy>,
    /// Circuit breaker applied to `Resource::create()`.
    ///
    /// See `nebula_resilience::standard_config()` for sensible defaults.
    #[serde(default)]
    pub create_breaker: Option<CircuitBreakerConfig>,
    /// Circuit breaker applied to `Resource::recycle()`.
    ///
    /// See `nebula_resilience::standard_config()` for sensible defaults.
    #[serde(default)]
    pub recycle_breaker: Option<CircuitBreakerConfig>,
    /// Optional timeout for a single `Resource::create()` call.
    ///
    /// `None` (default) keeps create unbounded and relies on higher-level
    /// cancellation plus circuit breakers.
    #[serde(default)]
    pub create_timeout: Option<Duration>,
    /// Optional timeout for a single `Resource::recycle()` call.
    ///
    /// `None` (default) keeps recycle unbounded.
    #[serde(default)]
    pub recycle_timeout: Option<Duration>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_size: 1,
            max_size: 10,
            acquire_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(3600),
            validation_interval: Duration::from_secs(30),
            maintenance_interval: None,
            strategy: PoolStrategy::default(),
            backpressure_policy: None,
            create_breaker: None,
            recycle_breaker: None,
            create_timeout: None,
            recycle_timeout: None,
        }
    }
}

impl PoolConfig {
    /// Enable both create/recycle circuit breakers using standard defaults.
    #[must_use]
    pub fn with_standard_breakers(mut self) -> Self {
        self.create_breaker = Some(nebula_resilience::standard_config());
        self.recycle_breaker = Some(nebula_resilience::standard_config());
        self
    }
}

impl PoolConfig {
    /// Validate pool configuration, returning an error if invalid.
    pub fn validate(&self) -> Result<()> {
        if self.max_size == 0 {
            return Err(Error::configuration("max_size must be greater than 0"));
        }
        if self.min_size > self.max_size {
            return Err(Error::configuration(format!(
                "min_size ({}) must not exceed max_size ({})",
                self.min_size, self.max_size
            )));
        }
        if self.acquire_timeout.is_zero() {
            return Err(Error::configuration(
                "acquire_timeout must be greater than zero",
            ));
        }
        if let Some(policy) = &self.backpressure_policy {
            match policy {
                PoolBackpressurePolicy::FailFast => {}
                PoolBackpressurePolicy::BoundedWait { timeout } => {
                    if timeout.is_zero() {
                        return Err(Error::configuration(
                            "backpressure bounded wait timeout must be greater than zero",
                        ));
                    }
                }
                PoolBackpressurePolicy::Adaptive(adaptive) => {
                    if !(0.0..=1.0).contains(&adaptive.high_pressure_utilization)
                        || adaptive.high_pressure_utilization == 0.0
                    {
                        return Err(Error::configuration(
                            "adaptive high_pressure_utilization must be in (0, 1]",
                        ));
                    }
                    if adaptive.high_pressure_waiters == 0 {
                        return Err(Error::configuration(
                            "adaptive high_pressure_waiters must be greater than zero",
                        ));
                    }
                    if adaptive.low_pressure_timeout.is_zero()
                        || adaptive.high_pressure_timeout.is_zero()
                    {
                        return Err(Error::configuration(
                            "adaptive timeouts must be greater than zero",
                        ));
                    }
                }
            }
        }
        if let Some(timeout) = self.create_timeout
            && timeout.is_zero()
        {
            return Err(Error::configuration(
                "create_timeout must be greater than zero when set",
            ));
        }
        if let Some(timeout) = self.recycle_timeout
            && timeout.is_zero()
        {
            return Err(Error::configuration(
                "recycle_timeout must be greater than zero when set",
            ));
        }
        Ok(())
    }

    /// Returns the effective acquire backpressure policy for this config.
    #[must_use]
    pub fn effective_backpressure_policy(&self) -> PoolBackpressurePolicy {
        self.backpressure_policy
            .clone()
            .unwrap_or(PoolBackpressurePolicy::BoundedWait {
                timeout: self.acquire_timeout,
            })
    }
}

// ---------------------------------------------------------------------------
// Pool internals
// ---------------------------------------------------------------------------

/// A pool entry wrapping a resource instance with lifecycle tracking.
struct Entry<T> {
    instance: T,
    created_at: Instant,
    last_used: Instant,
    /// Current lifecycle state of this entry.
    /// Tracked for observability and future use in drain/shutdown logic.
    #[allow(dead_code)]
    lifecycle: Lifecycle,
}

impl<T> Entry<T> {
    fn new(instance: T) -> Self {
        let now = Instant::now();
        Self {
            instance,
            created_at: now,
            last_used: now,
            lifecycle: Lifecycle::Ready,
        }
    }

    /// Return an entry to the pool, preserving the original `created_at`.
    fn returned(instance: T, created_at: Instant) -> Self {
        Self {
            instance,
            created_at,
            last_used: Instant::now(),
            lifecycle: Lifecycle::Idle,
        }
    }

    fn is_expired(&self, config: &PoolConfig) -> bool {
        self.created_at.elapsed() > config.max_lifetime
            || self.last_used.elapsed() > config.idle_timeout
    }
}

/// Pool statistics.
#[derive(Debug, Clone, Default)]
pub struct PoolStats {
    /// Total successful acquisitions.
    pub total_acquisitions: u64,
    /// Total releases back to pool.
    pub total_releases: u64,
    /// Current number of instances checked out.
    pub active: usize,
    /// Current number of idle instances in pool.
    pub idle: usize,
    /// Total instances ever created.
    pub created: u64,
    /// Total instances ever destroyed.
    pub destroyed: u64,
    /// Cumulative wait time across all acquisitions (milliseconds).
    pub total_wait_time_ms: u64,
    /// Maximum observed wait time for a single acquisition (milliseconds).
    pub max_wait_time_ms: u64,
    /// Number of times the pool was exhausted (acquire timed out).
    pub exhausted_count: u64,
    /// Acquire latency distribution summary.
    /// `None` when no acquisitions have been recorded yet.
    pub acquire_latency: Option<LatencyPercentiles>,
}

// ---------------------------------------------------------------------------
// Latency histogram
// ---------------------------------------------------------------------------

/// Acquire latency percentiles and mean (milliseconds).
#[derive(Debug, Clone)]
pub struct LatencyPercentiles {
    /// 50th percentile (median).
    pub p50_ms: u64,
    /// 95th percentile.
    pub p95_ms: u64,
    /// 99th percentile.
    pub p99_ms: u64,
    /// 99.9th percentile.
    pub p999_ms: u64,
    /// Arithmetic mean latency.
    pub mean_ms: f64,
}

type LatencyHistogram = Histogram<u64>;

fn new_latency_histogram() -> LatencyHistogram {
    Histogram::<u64>::new_with_bounds(1, 60_000, 3).expect("latency histogram bounds must be valid")
}

// ---------------------------------------------------------------------------
// CredentialHandler (type-erased for pool)
// ---------------------------------------------------------------------------

/// Type-erased credential handler stored in the pool.
/// Created at registration time by the manager; captures the concrete State type.
pub(crate) trait CredentialHandler<I>: Send + Sync {
    /// Apply serialized credential state to an instance.
    fn authorize(&self, instance: &mut I, state: &serde_json::Value) -> Result<()>;
    /// Strategy to apply when the credential rotates.
    fn rotation_strategy(&self) -> RotationStrategy;
}

// ---------------------------------------------------------------------------
// PoolState
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// IdleResult — outcome of a single pop from the idle queue
// ---------------------------------------------------------------------------

/// Result of one idle-queue pop, produced inside `with_inner_state`.
///
/// Carrying this out of the lock lets the hot path commit statistics
/// *in the same critical section as the pop*, so the acquire path only
/// needs one lock acquisition instead of two.
enum IdleResult<T> {
    /// Non-expired entry popped; statistics already committed under lock.
    /// Carries `wait_ms` so we can record it into the latency histogram
    /// *after* releasing the state lock.
    Valid(Entry<T>, u64),
    /// Expired entry popped; needs async cleanup, no stats committed.
    Expired(T),
    /// Queue empty; caller must create a new instance.
    Miss,
}

/// Combined pool state: idle queue + statistics under a single lock.
///
/// Latency histogram fields have been moved to [`LatencyState`] on `PoolInner`
/// to eliminate contention between idle-queue operations and histogram recording
/// under high-concurrency workloads.
struct PoolState<T> {
    idle: VecDeque<Entry<T>>,
    stats: PoolStats,
    /// Set to true after `shutdown()` to prevent Guard drops from
    /// reinserting instances into the idle queue.
    shutdown: bool,
}

/// Latency histogram state, kept in a **separate** lock from `PoolState`.
///
/// Separating histogram recording from the idle-queue mutex means that
/// 32–64 concurrent workers can record latency samples in parallel with other
/// threads checking out or returning instances, avoiding the serialisation
/// bottleneck seen under high-contention benchmarks.
struct LatencyState {
    histogram: LatencyHistogram,
    /// Monotonic counter incremented on every `histogram.record()`.
    seq: u64,
    /// Cached percentiles, invalidated when `seq` advances.
    percentiles_cache: Option<LatencyPercentiles>,
}

/// Inner shared state for the pool.
struct PoolInner<R: Resource> {
    resource: Arc<R>,
    config: R::Config,
    pool_config: PoolConfig,
    /// Cached resource key — avoids calling `resource.metadata()` on every hot-path
    /// operation (acquire, release, events). Populated once at pool construction.
    resource_key: ResourceKey,
    state: Mutex<Poison<PoolState<R::Instance>>>,
    /// Separate lock for latency histogram — decoupled from the idle-queue
    /// mutex so that recording samples does not block queue operations and
    /// vice versa under high concurrency.
    latency_state: Mutex<LatencyState>,
    /// Semaphore limits concurrent active (checked-out) instances.
    /// Idle instances do not hold permits.
    semaphore: Semaphore,
    /// Cooperative shutdown barrier.
    ///
    /// Background tasks and request handlers enter the gate before doing work.
    /// `shutdown()` calls `gate.close().await` to drain all active work before
    /// closing the semaphore and cleaning up idle instances.
    gate: Gate,
    /// Cancellation token for background tasks (maintenance).
    /// Cancelled on `shutdown()`.
    cancel: CancellationToken,
    /// Optional event bus for emitting lifecycle events.
    event_bus: Option<Arc<EventBus>>,
    /// Number of callers currently waiting to acquire an instance.
    waiting_count: Arc<AtomicUsize>,
    /// Lock-free active instance counter for adaptive backpressure.
    ///
    /// Mirrors `stats.active` but accessible without acquiring the state lock.
    active_count: AtomicUsize,
    /// Optional hook registry for lifecycle hooks (Create, Cleanup).
    hooks: Option<Arc<HookRegistry>>,
    /// Handle for the background maintenance task, if spawned.
    /// Stored so we can join on it during shutdown.
    ///
    /// Only written once at construction (never on the hot path) and taken
    /// once during shutdown — `Mutex<Option<_>>` is the right tool here;
    /// `AtomicTake` would require `&mut Arc<PoolInner>` at construction which
    /// races with `tokio::spawn` in multi-threaded runtimes.
    maintenance_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Current credential state (set by `handle_rotation` or initial auth).
    /// New instances get `authorize()` called with this when created.
    credential_state: Arc<RwLock<Option<serde_json::Value>>>,
    /// Optional credential handler for rotation (None for resources without credentials).
    credential_handler: Option<Arc<dyn CredentialHandler<R::Instance>>>,
    /// Circuit breaker for `Resource::create()` failures.
    create_breaker: Option<CircuitBreaker>,
    /// Circuit breaker for `Resource::recycle()` failures.
    recycle_breaker: Option<CircuitBreaker>,
    /// Pre-built synthetic context for background maintenance operations
    /// (cleanup, drain, scale). Avoids a heap allocation on every invocation.
    maintenance_ctx: Context,
}

// ---------------------------------------------------------------------------
// Pool<R>
// ---------------------------------------------------------------------------

/// Generic resource pool.
///
/// Manages a bounded set of `R::Instance` objects, creating, validating,
/// recycling and cleaning them up via the `Resource` trait.
pub struct Pool<R: Resource> {
    inner: Arc<PoolInner<R>>,
}

impl<R: Resource> Clone for Pool<R> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<R: Resource> std::fmt::Debug for Pool<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stats = self
            .with_state_read(|state| state.stats.clone())
            .unwrap_or_default();
        f.debug_struct("Pool")
            .field("resource_id", &self.inner.resource_key.as_ref())
            .field("stats", &stats)
            .finish()
    }
}

impl<R: Resource> Pool<R> {
    /// Create a new pool for the given resource, config, and pool settings.
    ///
    /// If `pool_config.maintenance_interval` is set, a background task is
    /// spawned that calls `maintain()` at that interval. The task is
    /// cancelled automatically on `shutdown()`.
    ///
    /// # Errors
    /// Returns error if `pool_config` is invalid (e.g. max_size == 0).
    pub fn new(resource: R, config: R::Config, pool_config: PoolConfig) -> Result<Self> {
        Self::with_hooks(resource, config, pool_config, None, None)
    }

    /// Create a new pool with an optional event bus for lifecycle events.
    ///
    /// Same as [`new`](Self::new) but allows wiring in an [`EventBus`].
    ///
    /// # Errors
    /// Returns error if `pool_config` is invalid (e.g. max_size == 0).
    pub fn with_event_bus(
        resource: R,
        config: R::Config,
        pool_config: PoolConfig,
        event_bus: Option<Arc<EventBus>>,
    ) -> Result<Self> {
        Self::with_hooks(resource, config, pool_config, event_bus, None)
    }

    /// Create a new pool with an optional event bus **and** hook registry.
    ///
    /// When a [`HookRegistry`] is provided, the pool fires
    /// [`HookEvent::Create`] around `Resource::create()` and
    /// [`HookEvent::Cleanup`] around `Resource::cleanup()`.
    ///
    /// # Errors
    /// Returns error if `pool_config` is invalid (e.g. max_size == 0).
    pub fn with_hooks(
        resource: R,
        config: R::Config,
        pool_config: PoolConfig,
        event_bus: Option<Arc<EventBus>>,
        hooks: Option<Arc<HookRegistry>>,
    ) -> Result<Self> {
        Self::with_hooks_and_credential(resource, config, pool_config, event_bus, hooks, None)
    }

    /// Internal: same as [`with_hooks`] but with optional credential handler.
    /// Used by the manager when registering credential-backed resources.
    pub(crate) fn with_hooks_and_credential(
        resource: R,
        config: R::Config,
        pool_config: PoolConfig,
        event_bus: Option<Arc<EventBus>>,
        hooks: Option<Arc<HookRegistry>>,
        credential_handler: Option<Arc<dyn CredentialHandler<R::Instance>>>,
    ) -> Result<Self> {
        pool_config.validate()?;
        let create_breaker = if let Some(cfg) = &pool_config.create_breaker {
            Some(
                CircuitBreaker::new(cfg.clone()).map_err(|e| Error::Configuration {
                    message: format!("create_breaker config invalid: {e}"),
                    source: None,
                })?,
            )
        } else {
            None
        };
        let recycle_breaker = if let Some(cfg) = &pool_config.recycle_breaker {
            Some(
                CircuitBreaker::new(cfg.clone()).map_err(|e| Error::Configuration {
                    message: format!("recycle_breaker config invalid: {e}"),
                    source: None,
                })?,
            )
        } else {
            None
        };
        let max = pool_config.max_size;
        let maintenance_interval = pool_config.maintenance_interval;
        let cancel = CancellationToken::new();

        let resource_key = resource.metadata().key.clone();
        tracing::debug!(
            resource_id = %resource_key,
            min_size = pool_config.min_size,
            max_size = pool_config.max_size,
            "Created new resource pool"
        );

        let pool = Self {
            inner: Arc::new(PoolInner {
                resource: Arc::new(resource),
                config,
                pool_config,
                resource_key,
                state: Mutex::new(Poison::new(
                    "pool_state",
                    PoolState {
                        idle: VecDeque::with_capacity(max),
                        stats: PoolStats::default(),
                        shutdown: false,
                    },
                )),
                latency_state: Mutex::new(LatencyState {
                    histogram: new_latency_histogram(),
                    seq: 0,
                    percentiles_cache: None,
                }),
                semaphore: Semaphore::new(max),
                gate: Gate::new(),
                cancel: cancel.clone(),
                event_bus,
                waiting_count: Arc::new(AtomicUsize::new(0)),
                active_count: AtomicUsize::new(0),
                hooks,
                maintenance_handle: Mutex::new(None),
                credential_state: Arc::new(RwLock::new(None)),
                credential_handler,
                create_breaker,
                recycle_breaker,
                maintenance_ctx: Context::background(
                    Scope::Global,
                    WorkflowId::nil(),
                    ExecutionId::nil(),
                ),
            }),
        };

        // Spawn automatic maintenance task if configured.
        if let Some(interval) = maintenance_interval {
            let maintenance_pool = pool.clone();
            let handle = tokio::spawn(async move {
                // Hold a gate guard for the lifetime of this background task.
                // When `shutdown()` closes the gate, it will wait here until
                // the task completes its current iteration and drops this guard.
                let _gate_guard = match maintenance_pool.inner.gate.enter() {
                    Ok(guard) => guard,
                    Err(_) => return, // gate already closed before task started
                };
                let maintenance_ctx = maintenance_pool.inner.maintenance_ctx.clone();
                loop {
                    tokio::select! {
                        () = tokio::time::sleep(interval) => {}
                        () = cancel.cancelled() => break,
                    }
                    let _ = maintenance_pool.maintain(&maintenance_ctx).await;
                }
            });
            *pool.inner.maintenance_handle.lock() = Some(handle);
        }

        Ok(pool)
    }

    fn map_poison_error(resource_key: nebula_core::ResourceKey, err: PoisonError) -> Error {
        Error::Internal {
            resource_key,
            message: format!("pool state poisoned: {err}"),
            source: None,
        }
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut PoolState<R::Instance>) -> T) -> Result<T> {
        let mut state = self.inner.state.lock();
        let mut guard = state
            .check_and_arm()
            .map_err(|err| Self::map_poison_error(self.inner.resource_key.clone(), err))?;
        let out = f(guard.data_mut());
        guard.disarm();
        Ok(out)
    }

    fn with_state_read<T>(&self, f: impl FnOnce(&PoolState<R::Instance>) -> T) -> Result<T> {
        let state = self.inner.state.lock();
        let data = state
            .try_read()
            .map_err(|err| Self::map_poison_error(self.inner.resource_key.clone(), err))?;
        Ok(f(data))
    }

    fn with_inner_state<T>(
        inner: &PoolInner<R>,
        f: impl FnOnce(&mut PoolState<R::Instance>) -> T,
    ) -> Result<T> {
        let mut state = inner.state.lock();
        let mut guard = state
            .check_and_arm()
            .map_err(|err| Self::map_poison_error(inner.resource_key.clone(), err))?;
        let out = f(guard.data_mut());
        guard.disarm();
        Ok(out)
    }

    /// Acquire a resource instance from the pool.
    ///
    /// Returns an RAII `Guard` that returns the instance to the pool
    /// when dropped. Respects `ctx.cancellation` when `Some` — if the
    /// token is cancelled while waiting, returns `Error::Unavailable`
    /// immediately. Background contexts (`None`) bypass `select!` entirely,
    /// saving ~100–130 ns on the maintenance/warm-up hot path.
    pub async fn acquire(
        &self,
        ctx: &Context,
    ) -> Result<(
        Guard<R::Instance, impl FnOnce(R::Instance, bool) + Send + 'static + use<R>>,
        Duration,
    )> {
        let start = Instant::now();

        let result = match &ctx.cancellation {
            // Hot path: no cancellation token → skip select! machinery entirely.
            None => self.acquire_inner(ctx, start).await,
            // Already cancelled before we even start.
            Some(token) if token.is_cancelled() => Err(Error::Unavailable {
                resource_key: self.inner.resource_key.clone(),
                reason: "Operation cancelled".to_string(),
                retryable: false,
            }),
            // Normal cancellable path.
            Some(token) => tokio::select! {
                biased;
                result = self.acquire_inner(ctx, start) => result,
                () = token.cancelled() => {
                    Err(Error::Unavailable {
                        resource_key: self.inner.resource_key.clone(),
                        reason: "Operation cancelled".to_string(),
                        retryable: false,
                    })
                }
            },
        };

        let wait_duration = start.elapsed();
        match &result {
            Ok(_) => tracing::debug!(
                resource_id = %self.inner.resource_key,
                scope = %ctx.scope,
                wait_ms = wait_duration.as_millis() as u64,
                "Acquired resource instance"
            ),
            Err(e) => tracing::warn!(
                resource_id = %self.inner.resource_key,
                scope = %ctx.scope,
                wait_ms = wait_duration.as_millis() as u64,
                error = %e,
                "Failed to acquire resource instance"
            ),
        }

        result.map(|guard| (guard, wait_duration))
    }

    /// Inner acquire logic, separated so `acquire` can wrap it in a
    /// cancellation-aware `select!`.
    #[expect(
        clippy::type_complexity,
        reason = "Concrete closure type; callers use inference"
    )]
    async fn acquire_inner(
        &self,
        ctx: &Context,
        start: Instant,
    ) -> Result<Guard<R::Instance, impl FnOnce(R::Instance, bool) + Send + 'static + use<R>>> {
        let inner = &self.inner;

        // RAII guard that increments `waiting_count` on entry and decrements on
        // any exit (success, early return, or panic), replacing paired
        // `fetch_add` / `fetch_sub` calls that can be missed on error paths.
        let _waiting = CounterGuard::new(&inner.waiting_count);

        // Acquire a permit according to configured backpressure policy.
        let permit = self.acquire_permit_with_policy().await?;

        if let Some(cb) = &inner.create_breaker
            && let Err(err) = cb.can_execute().await
        {
            return match err {
                ResilienceError::CircuitBreakerOpen { retry_after, .. } => {
                    let retry_after = retry_after.unwrap_or_default();
                    Self::emit_breaker_open(inner, "create", retry_after);
                    Err(Error::CircuitBreakerOpen {
                        resource_key: inner.resource_key.clone(),
                        operation: "create",
                        retry_after: Some(retry_after),
                    })
                }
                other => Err(Error::Internal {
                    resource_key: inner.resource_key.clone(),
                    message: format!("create breaker check failed: {other}"),
                    source: None,
                }),
            };
        }

        // Pop from the idle queue.  For valid (non-expired) entries the full
        // acquisition stats are committed **inside the same lock** as the pop,
        // eliminating the second lock acquisition on the hot idle-reuse path.
        let (instance, created_at) = loop {
            let idle_result = Self::with_inner_state(inner, |state| {
                let entry = match inner.pool_config.strategy {
                    PoolStrategy::Fifo => state.idle.pop_front(),
                    PoolStrategy::Lifo => state.idle.pop_back(),
                };
                match entry {
                    Some(entry) if entry.is_expired(&inner.pool_config) => {
                        state.stats.idle = state.idle.len();
                        IdleResult::Expired(entry.instance)
                    }
                    Some(entry) => {
                        // Commit all acquisition stats while still holding
                        // the lock — saves a second lock acquisition on
                        // the hot idle-reuse path.
                        let wait_ms = start.elapsed().as_millis() as u64;
                        state.stats.total_acquisitions += 1;
                        state.stats.active += 1;
                        state.stats.idle = state.idle.len();
                        state.stats.total_wait_time_ms += wait_ms;
                        if wait_ms > state.stats.max_wait_time_ms {
                            state.stats.max_wait_time_ms = wait_ms;
                        }
                        IdleResult::Valid(entry, wait_ms)
                    }
                    None => IdleResult::Miss,
                }
            })?;

            match idle_result {
                IdleResult::Expired(inst) => {
                    tracing::debug!("Destroying expired resource instance");
                    Self::cleanup_with_hooks(inner, inst, &CleanupReason::Expired, None).await;
                    continue;
                }
                IdleResult::Valid(entry, wait_ms) => {
                    // Record latency *outside* the state lock — the separate
                    // `latency_state` mutex means this does not serialise with
                    // concurrent idle-queue operations.
                    {
                        let mut lat = inner.latency_state.lock();
                        let _ = lat.histogram.record(wait_ms.max(1));
                        lat.seq = lat.seq.wrapping_add(1);
                        lat.percentiles_cache = None;
                    }
                    let created_at = entry.created_at;
                    // Fast path: poll is_reusable synchronously with a noop waker.
                    // Most health checks complete immediately (e.g. checking an
                    // atomic flag); only fall back to .await when truly Pending.
                    // The TaskCtx is scoped to the inner block so it is dropped
                    // before the .await, keeping the future Send.
                    let sync_result = {
                        use std::pin::pin;
                        use std::task::{Context as TaskCtx, Poll, Waker};
                        let waker = Waker::noop();
                        let mut task_cx = TaskCtx::from_waker(&waker);
                        let mut fut = pin!(inner.resource.is_reusable(&entry.instance));
                        match fut.as_mut().poll(&mut task_cx) {
                            Poll::Ready(result) => Some(result),
                            Poll::Pending => None,
                        }
                        // task_cx and waker are dropped here
                    };
                    let reusable = match sync_result {
                        Some(result) => result,
                        None => inner.resource.is_reusable(&entry.instance).await,
                    };
                    match reusable {
                        Ok(true) => break (entry.instance, Some(created_at)),
                        _ => {
                            // Undo the optimistically committed stats.
                            // total_wait_time_ms and latency_histogram are not
                            // reverted — an accepted approximation since
                            // is_reusable failures are rare.
                            tracing::debug!("Destroying invalid resource instance");
                            let _ = Self::with_inner_state(inner, |state| {
                                state.stats.total_acquisitions =
                                    state.stats.total_acquisitions.saturating_sub(1);
                                state.stats.active = state.stats.active.saturating_sub(1);
                            });
                            Self::cleanup_with_hooks(
                                inner,
                                entry.instance,
                                &CleanupReason::HealthCheckFailed,
                                None,
                            )
                            .await;
                            continue;
                        }
                    }
                }
                IdleResult::Miss => {
                    // No idle instances — create new, firing Create hooks.
                    tracing::debug!("Creating new resource instance");
                    let create_result = Self::create_with_hooks_timed(inner, ctx).await;
                    Self::maybe_record_breaker_result(
                        inner,
                        inner.create_breaker.as_ref(),
                        "create",
                        create_result.is_ok(),
                    )
                    .await;
                    let instance = create_result?;
                    break (instance, None);
                }
            }
        };

        // Stats lock for the create path only (idle-reuse committed them above).
        if created_at.is_none() {
            let wait_ms = start.elapsed().as_millis() as u64;
            Self::with_inner_state(inner, |state| {
                state.stats.total_acquisitions += 1;
                state.stats.active += 1;
                state.stats.created += 1;
                state.stats.idle = state.idle.len();
                state.stats.total_wait_time_ms += wait_ms;
                if wait_ms > state.stats.max_wait_time_ms {
                    state.stats.max_wait_time_ms = wait_ms;
                }
            })?;
            // Record latency outside the state lock.
            {
                let mut lat = inner.latency_state.lock();
                let _ = lat.histogram.record(wait_ms.max(1));
                lat.seq = lat.seq.wrapping_add(1);
                lat.percentiles_cache = None;
            }
        }
        inner.active_count.fetch_add(1, Ordering::Relaxed);

        // Forget the permit — we'll add it back when the guard drops.
        permit.forget();

        let pool = self.clone();
        let acquire_instant = Instant::now();
        Ok(Guard::new(instance, move |inst, tainted| {
            let usage_duration = acquire_instant.elapsed();
            // Fast path: attempt sync return without spawning a task.
            // Falls back to async spawn when the fast path isn't applicable.
            if let Some(inst) =
                Self::try_return_sync(&pool.inner, inst, created_at, usage_duration, tainted)
            {
                drop(tokio::spawn(Self::return_instance(
                    pool,
                    inst,
                    created_at,
                    usage_duration,
                    tainted,
                )));
            }
        }))
    }

    /// Attempt a zero-allocation synchronous return of an instance to the pool.
    ///
    /// Returns `None` when the instance was successfully returned to the idle queue.
    /// Returns `Some(inst)` when the async path is required, handing `inst` back
    /// to the caller unchanged.
    ///
    /// The sync fast-path applies when:
    /// - no recycle circuit-breaker is configured,
    /// - no lifecycle hooks are configured, AND
    /// - `Resource::recycle()` resolves in a single poll (the common case for
    ///   no-op / default-recycle implementations).
    ///
    /// Eliminates `tokio::spawn` overhead on the hot path, cutting acquire–release
    /// latency by ~60% for zero-cost resources.
    fn try_return_sync(
        inner: &PoolInner<R>,
        inst: R::Instance,
        created_at: Option<Instant>,
        usage_duration: Duration,
        tainted: bool,
    ) -> Option<R::Instance> {
        if tainted {
            return Some(inst);
        }
        if inner.recycle_breaker.is_some() || inner.hooks.is_some() {
            return Some(inst);
        }

        let mut inst = inst;
        // Poll recycle once with a noop waker.  The Pin and future drop at
        // block end, releasing the `&mut inst` borrow before we move `inst`.
        let recycle_ok = {
            let waker = std::task::Waker::noop();
            let cx = &mut std::task::Context::from_waker(waker);
            let mut fut = std::pin::pin!(inner.resource.recycle(&mut inst));
            matches!(fut.as_mut().poll(cx), std::task::Poll::Ready(Ok(())))
        };
        if !recycle_ok {
            return Some(inst);
        }

        // Push inst back to the idle queue under lock.
        // Use Option to avoid a conditional/partial move.
        let mut inst_opt = Some(inst);
        let pushed = {
            let mut state_lock = inner.state.lock();
            match state_lock.check_and_arm() {
                Ok(mut guard) => {
                    let s = guard.data_mut();
                    let ok = !s.shutdown;
                    if ok {
                        s.stats.total_releases += 1;
                        s.stats.active = s.stats.active.saturating_sub(1);
                        let i = inst_opt.take().expect("always Some before first take");
                        let entry = match created_at {
                            Some(ca) => Entry::returned(i, ca),
                            None => Entry::new(i),
                        };
                        s.idle.push_back(entry);
                        s.stats.idle = s.idle.len();
                    }
                    guard.disarm();
                    ok
                }
                Err(_) => false,
            }
        };
        if pushed {
            inner.active_count.fetch_sub(1, Ordering::Relaxed);
            inner.semaphore.add_permits(1);
            if let Some(bus) = &inner.event_bus {
                bus.emit(ResourceEvent::Released {
                    resource_key: inner.resource_key.clone(),
                    usage_duration,
                });
            }
            return None; // sync return complete
        }
        // Pool is shutting down or state was poisoned — hand inst to async path.
        inst_opt.take()
    }

    async fn acquire_permit_with_policy(&self) -> Result<tokio::sync::SemaphorePermit<'_>> {
        let inner = &self.inner;
        match inner.pool_config.effective_backpressure_policy() {
            PoolBackpressurePolicy::FailFast => match inner.semaphore.try_acquire() {
                Ok(permit) => Ok(permit),
                Err(TryAcquireError::NoPermits) => Err(self.pool_exhausted_error()),
                Err(TryAcquireError::Closed) => Err(self.pool_semaphore_closed_error()),
            },
            PoolBackpressurePolicy::BoundedWait { timeout } => {
                // Fast path: synchronous try-acquire avoids creating a timeout future
                // when a permit is immediately available (the common uncontended case).
                match inner.semaphore.try_acquire() {
                    Ok(permit) => return Ok(permit),
                    Err(TryAcquireError::Closed) => return Err(self.pool_semaphore_closed_error()),
                    Err(TryAcquireError::NoPermits) => {}
                }
                self.acquire_permit_with_timeout(timeout).await
            }
            PoolBackpressurePolicy::Adaptive(adaptive) => {
                // Fast path for adaptive policy too.
                match inner.semaphore.try_acquire() {
                    Ok(permit) => return Ok(permit),
                    Err(TryAcquireError::Closed) => return Err(self.pool_semaphore_closed_error()),
                    Err(TryAcquireError::NoPermits) => {}
                }
                let timeout = self.adaptive_timeout(&adaptive);
                self.acquire_permit_with_timeout(timeout).await
            }
        }
    }

    async fn acquire_permit_with_timeout(
        &self,
        timeout: Duration,
    ) -> Result<tokio::sync::SemaphorePermit<'_>> {
        let inner = &self.inner;
        match tokio::time::timeout(timeout, inner.semaphore.acquire()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(self.pool_semaphore_closed_error()),
            Err(_) => Err(self.pool_exhausted_error()),
        }
    }

    fn adaptive_timeout(&self, adaptive: &AdaptiveBackpressurePolicy) -> Duration {
        let inner = &self.inner;
        let active = inner.active_count.load(Ordering::Relaxed);
        let waiters = inner.waiting_count.load(Ordering::Relaxed);
        let utilization = active as f64 / inner.pool_config.max_size as f64;
        if utilization >= adaptive.high_pressure_utilization
            || waiters >= adaptive.high_pressure_waiters
        {
            adaptive.high_pressure_timeout
        } else {
            adaptive.low_pressure_timeout
        }
    }

    #[cold]
    fn pool_exhausted_error(&self) -> Error {
        let inner = &self.inner;
        let waiters = inner.waiting_count.load(Ordering::Relaxed);
        let current_size = match self.with_state(|state| {
            state.stats.exhausted_count += 1;
            state.stats.active
        }) {
            Ok(size) => size,
            Err(err) => return err,
        };
        let key = inner.resource_key.clone();
        Self::emit_event(inner, || ResourceEvent::PoolExhausted {
            resource_key: key.clone(),
            waiters,
        });
        Error::PoolExhausted {
            resource_key: key,
            current_size,
            max_size: inner.pool_config.max_size,
            waiters,
        }
    }

    #[cold]
    fn pool_semaphore_closed_error(&self) -> Error {
        Error::Internal {
            resource_key: self.inner.resource_key.clone(),
            message: "Pool semaphore closed".to_string(),
            source: None,
        }
    }

    /// Return an instance to the pool (or clean it up).
    ///
    /// Called from the guard's drop callback in a spawned task.
    async fn return_instance(
        pool: Self,
        inst: R::Instance,
        created_at: Option<Instant>,
        usage_duration: Duration,
        tainted: bool,
    ) {
        let inner = &pool.inner;
        let mut inst_slot = Some(inst);
        let mut recycle_ok = false;
        let mut skip_recycle = tainted;

        if let Some(cb) = &inner.recycle_breaker
            && let Err(ResilienceError::CircuitBreakerOpen { retry_after, .. }) =
                cb.can_execute().await
        {
            let retry_after = retry_after.unwrap_or_default();
            Self::emit_breaker_open(inner, "recycle", retry_after);
            skip_recycle = true;
        }

        if !skip_recycle {
            let recycle_result =
                Self::recycle_timed(inner, inst_slot.as_mut().expect("instance must exist")).await;
            recycle_ok = recycle_result.is_ok();
            Self::maybe_record_breaker_result(
                inner,
                inner.recycle_breaker.as_ref(),
                "recycle",
                recycle_result.is_ok(),
            )
            .await;
        }

        // Check shutdown under the same lock that pushes to idle to prevent a
        // race where shutdown flips between the read and insert. Stats are also
        // updated here to avoid a second lock acquisition after the fact.
        let cleanup_reason = Self::with_inner_state(inner, |state| {
            state.stats.total_releases += 1;
            state.stats.active = state.stats.active.saturating_sub(1);
            if recycle_ok && !state.shutdown {
                let inst = inst_slot.take().expect("instance must exist");
                let entry = match created_at {
                    Some(ca) => Entry::returned(inst, ca),
                    None => Entry::new(inst),
                };
                state.idle.push_back(entry);
                state.stats.idle = state.idle.len();
                None
            } else {
                let inst = inst_slot.take().expect("instance must exist");
                let reason = if state.shutdown {
                    CleanupReason::Shutdown
                } else if tainted {
                    CleanupReason::Tainted
                } else {
                    CleanupReason::RecycleFailed
                };
                state.stats.idle = state.idle.len();
                Some((inst, reason))
            }
        })
        .unwrap_or_else(|_| {
            Some((
                inst_slot.take().expect("instance must exist"),
                CleanupReason::RecycleFailed,
            ))
        });

        if cleanup_reason.is_none() {
            tracing::debug!(
                resource_id = %inner.resource_key,
                "Released resource instance back to pool"
            );
        }

        if let Some((to_cleanup, reason)) = cleanup_reason {
            Self::cleanup_with_hooks(inner, to_cleanup, &reason, None).await;
            tracing::debug!(
                resource_id = %inner.resource_key,
                "Cleaned up resource instance on release (pool shutdown or recycle failed)"
            );
        }

        Self::emit_event(inner, || ResourceEvent::Released {
            resource_key: inner.resource_key.clone(),
            usage_duration,
        });

        inner.active_count.fetch_sub(1, Ordering::Relaxed);
        inner.semaphore.add_permits(1);
    }

    /// Emit an event if the pool has an event bus configured.
    ///
    /// The event is constructed lazily via `make_event` and is only evaluated when
    /// an event bus is actually wired in, avoiding needless allocations in the
    /// common case where no bus is configured.
    fn emit_event(inner: &PoolInner<R>, make_event: impl FnOnce() -> ResourceEvent) {
        if let Some(bus) = &inner.event_bus {
            bus.emit(make_event());
        }
    }

    fn emit_breaker_open(inner: &PoolInner<R>, operation: &'static str, retry_after: Duration) {
        Self::emit_event(inner, || ResourceEvent::CircuitBreakerOpen {
            resource_key: inner.resource_key.clone(),
            operation,
            retry_after,
        });
    }

    fn emit_breaker_closed(inner: &PoolInner<R>, operation: &'static str) {
        Self::emit_event(inner, || ResourceEvent::CircuitBreakerClosed {
            resource_key: inner.resource_key.clone(),
            operation,
        });
    }

    async fn breaker_retry_after(cb: &CircuitBreaker) -> Duration {
        // `can_execute()` is called a second time solely to extract the
        // `retry_after` hint from the CircuitBreakerOpen error. The breaker is
        // already Open at this point (we just called `record_failure()`), so
        // the call is cheap (no contention). If CircuitBreaker ever exposes a
        // direct `retry_after()` accessor, prefer that instead.
        cb.can_execute()
            .await
            .err()
            .and_then(|e| match e {
                ResilienceError::CircuitBreakerOpen { retry_after, .. } => retry_after,
                _ => None,
            })
            .unwrap_or_default()
    }

    async fn breaker_record_success(
        inner: &PoolInner<R>,
        cb: &CircuitBreaker,
        operation: &'static str,
    ) {
        let was_half_open = matches!(cb.state().await, CircuitState::HalfOpen);
        cb.record_success().await;
        // A successful probe from HalfOpen always transitions to Closed per
        // circuit-breaker contract — no need for a third state() call.
        if was_half_open {
            Self::emit_breaker_closed(inner, operation);
        }
    }

    async fn breaker_record_failure(
        inner: &PoolInner<R>,
        cb: &CircuitBreaker,
        operation: &'static str,
    ) {
        cb.record_failure().await;
        if matches!(cb.state().await, CircuitState::Open) {
            let retry_after = Self::breaker_retry_after(cb).await;
            Self::emit_breaker_open(inner, operation, retry_after);
        }
    }

    async fn maybe_record_breaker_result(
        inner: &PoolInner<R>,
        breaker: Option<&CircuitBreaker>,
        operation: &'static str,
        success: bool,
    ) {
        let Some(cb) = breaker else {
            return;
        };

        if success {
            Self::breaker_record_success(inner, cb, operation).await;
        } else {
            Self::breaker_record_failure(inner, cb, operation).await;
        }
    }

    async fn create_with_hooks_timed(inner: &PoolInner<R>, ctx: &Context) -> Result<R::Instance> {
        if let Some(timeout) = inner.pool_config.create_timeout {
            return match tokio::time::timeout(timeout, Self::create_with_hooks(inner, ctx)).await {
                Ok(result) => result,
                Err(_) => Err(Error::Timeout {
                    resource_key: inner.resource_key.clone(),
                    timeout_ms: timeout.as_millis() as u64,
                    operation: "create".to_string(),
                }),
            };
        }
        Self::create_with_hooks(inner, ctx).await
    }

    async fn recycle_timed(inner: &PoolInner<R>, instance: &mut R::Instance) -> Result<()> {
        if let Some(timeout) = inner.pool_config.recycle_timeout {
            return match tokio::time::timeout(timeout, inner.resource.recycle(instance)).await {
                Ok(result) => result,
                Err(_) => Err(Error::Timeout {
                    resource_key: inner.resource_key.clone(),
                    timeout_ms: timeout.as_millis() as u64,
                    operation: "recycle".to_string(),
                }),
            };
        }
        inner.resource.recycle(instance).await
    }

    /// Create a new resource instance, firing [`HookEvent::Create`]
    /// before/after hooks when a [`HookRegistry`] is attached.
    ///
    /// Before-hooks can cancel the creation by returning
    /// [`HookResult::Cancel`](crate::hooks::HookResult::Cancel).
    async fn create_with_hooks(inner: &PoolInner<R>, ctx: &Context) -> Result<R::Instance> {
        let resource_id = inner.resource_key.as_ref();

        // Run Create before-hooks.
        if let Some(hooks) = &inner.hooks {
            hooks
                .run_before(&HookEvent::Create, resource_id, ctx)
                .await?;
        }

        let mut result = inner.resource.create(&inner.config, ctx).await;

        // Apply credential authorization if handler and state are set.
        if result.is_ok()
            && let (Some(handler), Some(state)) =
                (&inner.credential_handler, &*inner.credential_state.read())
            && let Ok(ref mut instance) = result
            && let Err(e) = handler.authorize(instance, state)
        {
            result = Err(e);
        }

        // Run Create after-hooks (errors are logged, never propagated).
        if let Some(hooks) = &inner.hooks {
            hooks
                .run_after(&HookEvent::Create, resource_id, ctx, result.is_ok())
                .await;
        }

        result
    }

    /// Clean up an instance, firing [`HookEvent::Cleanup`] before/after
    /// hooks when a [`HookRegistry`] is attached.
    ///
    /// This also increments the `destroyed` stat and emits a
    /// [`ResourceEvent::CleanedUp`] event.
    async fn cleanup_with_hooks(
        inner: &PoolInner<R>,
        instance: R::Instance,
        reason: &CleanupReason,
        ctx: Option<&Context>,
    ) {
        let resource_id = inner.resource_key.as_ref();
        let ctx = ctx.unwrap_or(&inner.maintenance_ctx);

        // Run Cleanup before-hooks (result is best-effort — cannot cancel a cleanup).
        if let Some(hooks) = &inner.hooks {
            let _ = hooks
                .run_before(&HookEvent::Cleanup, resource_id, ctx)
                .await;
        }

        let cleanup_ok = inner.resource.cleanup(instance).await.is_ok();

        // Run Cleanup after-hooks.
        if let Some(hooks) = &inner.hooks {
            hooks
                .run_after(&HookEvent::Cleanup, resource_id, ctx, cleanup_ok)
                .await;
        }

        let _ = Self::with_inner_state(inner, |state| {
            state.stats.destroyed += 1;
        });
        Self::emit_event(inner, || ResourceEvent::CleanedUp {
            resource_key: inner.resource_key.clone(),
            reason: reason.clone(),
        });
    }

    /// Called by `ResourceManager` when the bound credential rotates.
    ///
    /// - **HotSwap**: calls `authorize()` on all idle instances with the new state.
    /// - **DrainAndRecreate**: evicts all idle instances; new instances get `authorize()` on create.
    /// - **Reconnect**: same as DrainAndRecreate (evicts idle; in-flight complete and are cleaned on release).
    ///
    /// # Errors
    /// Returns `CredentialNotConfigured` if no credential handler was set for this pool.
    pub async fn handle_rotation(
        &self,
        new_state: &serde_json::Value,
        strategy: RotationStrategy,
        credential_key: nebula_core::CredentialKey,
    ) -> Result<()> {
        let inner = &self.inner;
        let key = inner.resource_key.clone();

        // Update stored state (new instances will use this).
        *inner.credential_state.write() = Some(new_state.clone());

        let handler = inner
            .credential_handler
            .as_ref()
            .ok_or(Error::CredentialNotConfigured {
                resource_key: key.clone(),
            })?;

        match strategy {
            RotationStrategy::HotSwap => {
                Self::with_inner_state(inner, |state| {
                    for entry in state.idle.iter_mut() {
                        handler.authorize(&mut entry.instance, new_state)?;
                    }
                    Ok(())
                })??;
            }
            RotationStrategy::DrainAndRecreate | RotationStrategy::Reconnect => {
                Self::drain_idle(inner).await;
            }
        }

        if let Some(bus) = &inner.event_bus {
            bus.emit(ResourceEvent::CredentialRotated {
                resource_key: key,
                credential_key,
                strategy: format!("{strategy:?}"),
            });
        }

        Ok(())
    }

    /// Evict all idle instances (used by DrainAndRecreate and Reconnect).
    async fn drain_idle(inner: &PoolInner<R>) {
        let to_cleanup: Vec<_> = Self::with_inner_state(inner, |state| {
            let entries: Vec<_> = state.idle.drain(..).collect();
            state.stats.idle = state.idle.len();
            entries
        })
        .unwrap_or_default();
        for entry in to_cleanup {
            Self::cleanup_with_hooks(
                inner,
                entry.instance,
                &CleanupReason::CredentialRotated,
                None,
            )
            .await;
        }
    }

    /// Get current pool statistics, including latency percentiles
    /// computed from a sliding window of recent acquisitions.
    #[must_use]
    pub fn stats(&self) -> PoolStats {
        // Snapshot stats from the state lock.
        let mut stats = match self.with_state_read(|state| state.stats.clone()) {
            Ok(s) => s,
            Err(_) => return PoolStats::default(),
        };

        // Read latency percentiles from the separate latency lock.
        // This lock is independent of the idle-queue mutex, so callers
        // of `stats()` never contend with the acquire hot path.
        let mut lat = self.inner.latency_state.lock();
        if let Some(ref cached) = lat.percentiles_cache {
            stats.acquire_latency = Some(cached.clone());
            return stats;
        }
        let computed = if lat.histogram.len() == 0 {
            None
        } else {
            Some(LatencyPercentiles {
                p50_ms: lat.histogram.value_at_quantile(0.50),
                p95_ms: lat.histogram.value_at_quantile(0.95),
                p99_ms: lat.histogram.value_at_quantile(0.99),
                p999_ms: lat.histogram.value_at_quantile(0.999),
                mean_ms: lat.histogram.mean(),
            })
        };
        lat.percentiles_cache = computed.clone();
        drop(lat);

        stats.acquire_latency = computed;
        stats
    }

    /// Intentionally poison internal pool state.
    ///
    /// This is primarily intended for integration tests that validate
    /// poisoned-state behavior.
    #[doc(hidden)]
    pub fn poison_for_test(&self) {
        let mut state = self.inner.state.lock();
        if let Ok(_guard) = state.check_and_arm() {
            // Dropping the guard without disarm poisons the state.
        }
    }

    /// Get a reference to the pool configuration.
    #[must_use]
    pub fn pool_config(&self) -> &PoolConfig {
        &self.inner.pool_config
    }

    /// Pre-create up to `count` idle instances, respecting `max_size`.
    ///
    /// Returns the number of instances actually created. The pool will not
    /// exceed `max_size` total (idle + active). Creation errors are silently
    /// ignored and the method returns what it managed to create so far.
    pub async fn scale_up(&self, count: usize) -> usize {
        let inner = &self.inner;
        let ctx = &inner.maintenance_ctx;
        let mut created = 0;

        for _ in 0..count {
            // Check headroom under the lock, then release before async create.
            let has_room = Self::with_inner_state(inner, |state| {
                let total = state.idle.len() + state.stats.active;
                total < inner.pool_config.max_size
            })
            .unwrap_or(false);

            if !has_room {
                break;
            }

            let instance = match Self::create_with_hooks(inner, ctx).await {
                Ok(inst) => inst,
                Err(_) => break,
            };

            // Re-check capacity under the lock after the async create.
            // Returns Some(instance) if rejected (over capacity).
            let rejected = Self::with_inner_state(inner, |state| {
                let total = state.idle.len() + state.stats.active;
                if total < inner.pool_config.max_size {
                    state.idle.push_back(Entry::new(instance));
                    state.stats.created += 1;
                    state.stats.idle = state.idle.len();
                    created += 1;
                    None
                } else {
                    Some(instance)
                }
            })
            .ok()
            .flatten();

            if let Some(surplus) = rejected {
                Self::cleanup_with_hooks(inner, surplus, &CleanupReason::Evicted, Some(ctx)).await;
                break;
            }
        }

        created
    }

    /// Remove up to `count` idle instances, respecting `min_size`.
    ///
    /// Returns the number of instances actually removed. The pool will keep
    /// at least `min_size` total (idle + active).
    pub async fn scale_down(&self, count: usize) -> usize {
        let inner = &self.inner;
        let mut removed = 0;

        for _ in 0..count {
            let entry = Self::with_inner_state(inner, |state| {
                let total = state.idle.len() + state.stats.active;
                if total <= inner.pool_config.min_size || state.idle.is_empty() {
                    None
                } else {
                    state.idle.pop_front()
                }
            })
            .ok()
            .flatten();

            if let Some(entry) = entry {
                Self::cleanup_with_hooks(inner, entry.instance, &CleanupReason::Evicted, None)
                    .await;
                let _ = Self::with_inner_state(inner, |state| {
                    state.stats.idle = state.idle.len();
                });
                removed += 1;
            } else {
                break;
            }
        }

        removed
    }

    /// Get a snapshot of current pool dimensions: `(active, idle, max_size)`.
    ///
    /// Useful for feeding into [`AutoScaler`](crate::autoscale::AutoScaler)
    /// without exposing the full `PoolStats`.
    #[must_use]
    pub fn utilization_snapshot(&self) -> (usize, usize, usize) {
        self.with_state_read(|state| {
            (
                state.stats.active,
                state.idle.len(),
                self.inner.pool_config.max_size,
            )
        })
        .unwrap_or((0, 0, self.inner.pool_config.max_size))
    }

    /// Get the current number of callers waiting to acquire an instance.
    #[must_use]
    pub fn waiting_count(&self) -> usize {
        self.inner.waiting_count.load(Ordering::Relaxed)
    }

    /// Run maintenance: evict expired idle instances, ensure min_size.
    pub async fn maintain(&self, ctx: &Context) -> Result<()> {
        tracing::debug!(resource_id = %self.inner.resource_key, "Running pool maintenance");

        let inner = &self.inner;

        // Evict expired idle entries
        let mut to_cleanup = Vec::new();
        Self::with_inner_state(inner, |state| {
            let mut kept = VecDeque::with_capacity(state.idle.len());
            while let Some(entry) = state.idle.pop_front() {
                if entry.is_expired(&inner.pool_config) {
                    to_cleanup.push(entry.instance);
                } else {
                    kept.push_back(entry);
                }
            }
            state.idle = kept;
        })?;

        for inst in to_cleanup {
            Self::cleanup_with_hooks(inner, inst, &CleanupReason::Evicted, Some(ctx)).await;
        }

        // Ensure min_size
        let (current_idle, current_active) =
            Self::with_inner_state(inner, |state| (state.idle.len(), state.stats.active))?;
        let total = current_idle + current_active;
        if total < inner.pool_config.min_size {
            let needed = inner.pool_config.min_size - total;
            for _ in 0..needed {
                let Ok(instance) = Self::create_with_hooks(inner, ctx).await else {
                    break;
                };

                Self::with_inner_state(inner, |state| {
                    state.idle.push_back(Entry::new(instance));
                    state.stats.created += 1;
                })?;
            }
        }

        // Sync idle count
        Self::with_inner_state(inner, |state| {
            state.stats.idle = state.idle.len();
        })?;

        Ok(())
    }

    /// Shut down the pool, cleaning up all idle instances.
    ///
    /// After shutdown:
    /// - Background maintenance task (if any) is cancelled and awaited.
    /// - New `acquire()` calls fail immediately (semaphore is closed).
    /// - Any `Guard` dropped will clean up its instance instead of
    ///   returning it to the idle queue.
    pub async fn shutdown(&self) -> Result<()> {
        let inner = &self.inner;

        // Cancel background maintenance task.
        inner.cancel.cancel();

        // Wait for the maintenance task to finish if it was spawned.
        let handle = inner.maintenance_handle.lock().take();
        if let Some(h) = handle {
            // The task should exit promptly because we cancelled above.
            let _ = h.await;
        }

        // Close the gate: mark as closing and wait for all background tasks
        // (and any in-flight gate guards) to drop their guards.
        inner.gate.close().await;

        // Close the semaphore so new acquire() calls fail immediately
        // instead of blocking until timeout.
        inner.semaphore.close();

        let entries: Vec<_> = {
            Self::with_inner_state(inner, |state| {
                state.shutdown = true;
                state.idle.drain(..).collect()
            })?
        };

        for entry in entries {
            Self::cleanup_with_hooks(inner, entry.instance, &CleanupReason::Shutdown, None).await;
        }

        Self::with_inner_state(inner, |state| {
            state.stats.idle = 0;
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::ResourceMetadata;
    use crate::resource::{Config, Resource};
    use crate::scope::Scope;
    use nebula_core::ResourceKey;

    // -- Test resource --

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TestConfig {
        prefix: String,
    }

    impl Config for TestConfig {
        fn validate(&self) -> Result<()> {
            if self.prefix.is_empty() {
                return Err(Error::configuration("prefix cannot be empty"));
            }
            Ok(())
        }
    }

    struct TestResource;

    impl Resource for TestResource {
        type Config = TestConfig;
        type Instance = String;
        fn metadata(&self) -> ResourceMetadata {
            ResourceMetadata::from_key(ResourceKey::try_from("test-resource").expect("valid"))
        }

        async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            Ok(format!("{}-instance", config.prefix))
        }
    }

    fn test_ctx() -> Context {
        Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
    }

    fn test_config() -> TestConfig {
        TestConfig {
            prefix: "test".to_string(),
        }
    }

    #[test]
    fn test_pool_config_default() {
        let config = PoolConfig::default();
        assert_eq!(config.min_size, 1);
        assert_eq!(config.max_size, 10);
        assert_eq!(config.acquire_timeout, Duration::from_secs(30));
        assert!(config.backpressure_policy.is_none());
        assert_eq!(
            config.effective_backpressure_policy(),
            PoolBackpressurePolicy::BoundedWait {
                timeout: Duration::from_secs(30)
            }
        );
    }

    #[test]
    fn test_pool_config_validation() {
        assert!(
            PoolConfig {
                max_size: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            PoolConfig {
                min_size: 11,
                max_size: 10,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            PoolConfig {
                acquire_timeout: Duration::ZERO,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            PoolConfig {
                backpressure_policy: Some(PoolBackpressurePolicy::BoundedWait {
                    timeout: Duration::ZERO
                }),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            PoolConfig {
                backpressure_policy: Some(PoolBackpressurePolicy::Adaptive(
                    AdaptiveBackpressurePolicy {
                        high_pressure_utilization: 1.2,
                        ..Default::default()
                    }
                )),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            PoolConfig {
                create_timeout: Some(Duration::ZERO),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            PoolConfig {
                recycle_timeout: Some(Duration::ZERO),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(PoolConfig::default().validate().is_ok());
    }

    #[tokio::test]
    async fn fail_fast_backpressure_returns_immediately() {
        let pool = Pool::new(
            TestResource,
            test_config(),
            PoolConfig {
                min_size: 0,
                max_size: 1,
                acquire_timeout: Duration::from_secs(10),
                backpressure_policy: Some(PoolBackpressurePolicy::FailFast),
                ..Default::default()
            },
        )
        .unwrap();

        let (_held, _) = pool.acquire(&test_ctx()).await.unwrap();
        let start = Instant::now();
        let err = pool.acquire(&test_ctx()).await.unwrap_err();
        let elapsed = start.elapsed();

        assert!(matches!(err, Error::PoolExhausted { .. }));
        assert!(
            elapsed < Duration::from_millis(100),
            "fail-fast should not wait for timeout, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn bounded_wait_backpressure_uses_policy_timeout() {
        let pool = Pool::new(
            TestResource,
            test_config(),
            PoolConfig {
                min_size: 0,
                max_size: 1,
                acquire_timeout: Duration::from_secs(10),
                backpressure_policy: Some(PoolBackpressurePolicy::BoundedWait {
                    timeout: Duration::from_millis(40),
                }),
                ..Default::default()
            },
        )
        .unwrap();

        let (_held, _) = pool.acquire(&test_ctx()).await.unwrap();
        let start = Instant::now();
        let err = pool.acquire(&test_ctx()).await.unwrap_err();
        let elapsed = start.elapsed();

        assert!(matches!(err, Error::PoolExhausted { .. }));
        assert!(
            elapsed < Duration::from_millis(300),
            "bounded-wait policy timeout should be respected, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn adaptive_backpressure_uses_high_pressure_timeout_when_saturated() {
        let pool = Pool::new(
            TestResource,
            test_config(),
            PoolConfig {
                min_size: 0,
                max_size: 1,
                acquire_timeout: Duration::from_secs(10),
                backpressure_policy: Some(PoolBackpressurePolicy::Adaptive(
                    AdaptiveBackpressurePolicy {
                        high_pressure_utilization: 0.5,
                        high_pressure_waiters: 1,
                        low_pressure_timeout: Duration::from_secs(1),
                        high_pressure_timeout: Duration::from_millis(30),
                    },
                )),
                ..Default::default()
            },
        )
        .unwrap();

        let (_held, _) = pool.acquire(&test_ctx()).await.unwrap();
        let start = Instant::now();
        let err = pool.acquire(&test_ctx()).await.unwrap_err();
        let elapsed = start.elapsed();

        assert!(matches!(err, Error::PoolExhausted { .. }));
        assert!(
            elapsed < Duration::from_millis(250),
            "adaptive policy under pressure should switch to short timeout, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn acquire_returns_instance() {
        let pool = Pool::new(TestResource, test_config(), PoolConfig::default()).unwrap();
        let ctx = test_ctx();
        let (guard, _wait) = pool.acquire(&ctx).await.unwrap();
        assert_eq!(*guard, "test-instance");
    }

    #[tokio::test]
    async fn pool_reuses_instances() {
        let pool = Pool::new(TestResource, test_config(), PoolConfig::default()).unwrap();

        // Acquire and drop to return to pool
        {
            let (_guard, _) = pool.acquire(&test_ctx()).await.unwrap();
        }
        // Give the spawn a moment to return the instance
        tokio::time::sleep(Duration::from_millis(50)).await;

        let stats = pool.stats();
        assert_eq!(stats.created, 1);

        // Acquire again — should reuse
        let (_guard, _) = pool.acquire(&test_ctx()).await.unwrap();
        let stats = pool.stats();
        assert_eq!(stats.total_acquisitions, 2);
        // May be 1 or 2 created depending on timing, but should be <= 2
        assert!(stats.created <= 2);
    }

    #[tokio::test]
    async fn pool_respects_max_size() {
        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_millis(100),
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();

        let (_g1, _) = pool.acquire(&test_ctx()).await.unwrap();
        let (_g2, _) = pool.acquire(&test_ctx()).await.unwrap();

        // Third acquire should timeout
        let result = pool.acquire(&test_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_cleans_idle() {
        let pool = Pool::new(TestResource, test_config(), PoolConfig::default()).unwrap();

        {
            let (_g, _) = pool.acquire(&test_ctx()).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

        pool.shutdown().await.unwrap();
        let stats = pool.stats();
        assert_eq!(stats.idle, 0);
    }

    struct SlowResource;

    impl Resource for SlowResource {
        type Config = TestConfig;
        type Instance = String;

        fn metadata(&self) -> ResourceMetadata {
            ResourceMetadata::from_key(ResourceKey::try_from("slow-resource").expect("valid"))
        }

        async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            tokio::time::sleep(Duration::from_millis(80)).await;
            Ok(format!("{}-instance", config.prefix))
        }

        async fn recycle(&self, _instance: &mut Self::Instance) -> Result<()> {
            tokio::time::sleep(Duration::from_millis(80)).await;
            Ok(())
        }
    }

    #[tokio::test]
    async fn create_timeout_returns_timeout_error() {
        let pool = Pool::new(
            SlowResource,
            test_config(),
            PoolConfig {
                min_size: 0,
                max_size: 1,
                create_timeout: Some(Duration::from_millis(10)),
                ..Default::default()
            },
        )
        .unwrap();

        let err = pool
            .acquire(&test_ctx())
            .await
            .expect_err("create must timeout");
        assert!(matches!(
            err,
            Error::Timeout { operation, .. } if operation == "create"
        ));
    }

    #[tokio::test]
    async fn recycle_timeout_cleans_up_instance() {
        let pool = Pool::new(
            SlowResource,
            test_config(),
            PoolConfig {
                min_size: 0,
                max_size: 1,
                recycle_timeout: Some(Duration::from_millis(10)),
                ..Default::default()
            },
        )
        .unwrap();

        {
            let (_guard, _) = pool.acquire(&test_ctx()).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(120)).await;

        let stats = pool.stats();
        assert_eq!(stats.idle, 0, "timed-out recycle must not return to idle");
        assert_eq!(
            stats.destroyed, 1,
            "timed-out recycle should cleanup instance"
        );
    }

    // -- Resource that fails validation --

    struct InvalidatingResource {
        /// After this many validations, start returning false
        fail_after: std::sync::atomic::AtomicU32,
    }

    impl Resource for InvalidatingResource {
        type Config = TestConfig;
        type Instance = String;
        fn metadata(&self) -> ResourceMetadata {
            ResourceMetadata::from_key(ResourceKey::try_from("invalidating").expect("valid"))
        }

        async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            Ok(format!("{}-inst", config.prefix))
        }

        async fn is_reusable(&self, _instance: &Self::Instance) -> Result<bool> {
            let remaining = self
                .fail_after
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Ok(remaining > 0)
        }
    }

    #[tokio::test]
    async fn exhausted_count_tracked() {
        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 1,
            acquire_timeout: Duration::from_millis(50),
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();
        let ctx = test_ctx();

        // Hold a guard so pool is full
        let (_g, _) = pool.acquire(&ctx).await.unwrap();

        // This should fail and increment exhausted_count
        let _ = pool.acquire(&ctx).await;

        let stats = pool.stats();
        assert_eq!(stats.exhausted_count, 1);
    }

    #[tokio::test]
    async fn acquire_skips_invalid_idle_and_creates_new() {
        let resource = InvalidatingResource {
            // First is_reusable call returns false, subsequent ones return true (underflow wraps)
            fail_after: std::sync::atomic::AtomicU32::new(0),
        };
        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let pool = Pool::new(resource, test_config(), pool_config).unwrap();

        // Acquire, drop, wait for return to idle
        {
            let (_g, _) = pool.acquire(&test_ctx()).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

        let stats = pool.stats();
        assert_eq!(stats.idle, 1, "instance should be in idle pool");

        // Next acquire: idle instance fails is_valid, gets destroyed, new one created
        let (_g, _) = pool.acquire(&test_ctx()).await.unwrap();

        let stats = pool.stats();
        assert_eq!(stats.destroyed, 1, "invalid instance should be destroyed");
        assert!(stats.created >= 2, "should have created a replacement");
    }

    // -- Resource that fails recycle --

    struct RecycleFailResource;

    impl Resource for RecycleFailResource {
        type Config = TestConfig;
        type Instance = String;
        fn metadata(&self) -> ResourceMetadata {
            ResourceMetadata::from_key(ResourceKey::try_from("recycle-fail").expect("valid"))
        }

        async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            Ok(format!("{}-inst", config.prefix))
        }

        async fn recycle(&self, _instance: &mut Self::Instance) -> Result<()> {
            let key = nebula_core::ResourceKey::try_from("recycle-fail").expect("valid key");
            Err(Error::Internal {
                resource_key: key,
                message: "recycle failed".to_string(),
                source: None,
            })
        }
    }

    #[tokio::test]
    async fn recycle_failure_destroys_instance() {
        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let pool = Pool::new(RecycleFailResource, test_config(), pool_config).unwrap();

        // Acquire and drop — recycle will fail, so instance should be destroyed
        {
            let (_g, _) = pool.acquire(&test_ctx()).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

        let stats = pool.stats();
        assert_eq!(stats.destroyed, 1, "failed recycle should destroy instance");
        assert_eq!(
            stats.idle, 0,
            "destroyed instance should not be in idle pool"
        );
    }

    #[tokio::test]
    async fn pool_recovers_after_recycle_failure() {
        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 1,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let pool = Pool::new(RecycleFailResource, test_config(), pool_config).unwrap();

        // Acquire and drop — recycle fails, instance destroyed, permit returned
        {
            let (_g2, _) = pool.acquire(&test_ctx()).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Should still be able to acquire (permit was returned even though recycle failed)
        let (_g, _) = pool
            .acquire(&test_ctx())
            .await
            .expect("should acquire after recycle failure");
        let stats = pool.stats();
        assert_eq!(stats.created, 2, "should have created a second instance");
    }

    #[tokio::test]
    async fn maintain_evicts_expired_and_replenishes() {
        let pool_config = PoolConfig {
            min_size: 2,
            max_size: 5,
            acquire_timeout: Duration::from_secs(1),
            idle_timeout: Duration::from_millis(50), // very short
            max_lifetime: Duration::from_secs(3600),
            validation_interval: Duration::from_secs(30),
            maintenance_interval: None,
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();

        // Acquire and return 3 instances
        for _ in 0..3 {
            let (_g, _) = pool.acquire(&test_ctx()).await.unwrap();
            // drop returns to pool
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Wait for idle timeout
        tokio::time::sleep(Duration::from_millis(100)).await;

        // maintain should evict expired and replenish to min_size
        pool.maintain(&test_ctx()).await.unwrap();

        let stats = pool.stats();
        assert!(
            stats.idle >= 2,
            "maintain should replenish to min_size, got idle={}",
            stats.idle
        );
    }

    #[tokio::test]
    async fn maintain_does_not_exceed_min_size() {
        let pool_config = PoolConfig {
            min_size: 1,
            max_size: 5,
            acquire_timeout: Duration::from_secs(1),
            idle_timeout: Duration::from_secs(3600),
            max_lifetime: Duration::from_secs(3600),
            validation_interval: Duration::from_secs(30),
            maintenance_interval: None,
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();

        // Already have 0 idle, 0 active -> total < min_size
        pool.maintain(&test_ctx()).await.unwrap();

        let stats = pool.stats();
        assert_eq!(
            stats.idle, 1,
            "maintain should create exactly min_size instances"
        );
        assert_eq!(stats.created, 1);

        // Run maintain again — should not create more
        pool.maintain(&test_ctx()).await.unwrap();
        let stats = pool.stats();
        assert_eq!(stats.idle, 1);
        assert_eq!(
            stats.created, 1,
            "maintain should not create beyond min_size"
        );
    }

    #[tokio::test]
    async fn concurrent_acquires_respect_max_size() {
        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 5,
            acquire_timeout: Duration::from_millis(200),
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();

        // Acquire max_size instances concurrently
        let mut guards = Vec::new();
        let ctx = test_ctx();
        for _ in 0..5 {
            let (g, _) = pool.acquire(&ctx).await.unwrap();
            guards.push(g);
        }

        // Next acquire should fail
        let result = pool.acquire(&test_ctx()).await;
        assert!(result.is_err(), "should not exceed max_size");

        // Drop one guard, wait for return
        guards.pop();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Should succeed now
        let (_g, _) = pool
            .acquire(&test_ctx())
            .await
            .expect("should acquire after release");
    }

    #[tokio::test]
    async fn acquire_after_shutdown_fails_immediately() {
        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_secs(5), // long timeout
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();

        pool.shutdown().await.unwrap();

        // acquire() should fail immediately (semaphore closed), not block for 5s
        let start = Instant::now();
        let result = pool.acquire(&test_ctx()).await;
        let elapsed = start.elapsed();

        assert!(result.is_err(), "acquire after shutdown should fail");
        assert!(
            elapsed < Duration::from_secs(1),
            "should fail immediately, not wait for timeout (took {:?})",
            elapsed
        );
    }

    #[tokio::test]
    async fn guard_dropped_after_shutdown_cleans_up() {
        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();
        let ctx = test_ctx();

        // Acquire a guard, then shutdown the pool while still holding it
        let (guard, _wait) = pool.acquire(&ctx).await.unwrap();
        pool.shutdown().await.unwrap();

        // Drop the guard — should NOT reinsert into idle, should clean up
        drop(guard);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let stats = pool.stats();
        assert_eq!(
            stats.idle, 0,
            "instance should not be reinserted after shutdown"
        );
        assert_eq!(stats.destroyed, 1, "instance should be cleaned up");
    }

    // -- Resource that fails create --

    struct FailingCreateResource {
        /// Countdown: create() fails while > 0, then succeeds.
        remaining_failures: std::sync::atomic::AtomicU32,
    }

    impl Resource for FailingCreateResource {
        type Config = TestConfig;
        type Instance = String;
        fn metadata(&self) -> ResourceMetadata {
            ResourceMetadata::from_key(ResourceKey::try_from("failing-create").expect("valid"))
        }

        async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            let remaining = self
                .remaining_failures
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if remaining > 0 {
                return Err(Error::Initialization {
                    resource_key: ResourceKey::try_from("failing-create").expect("valid"),
                    reason: "intentional failure".to_string(),
                    source: None,
                });
            }
            Ok(format!("{}-inst", config.prefix))
        }
    }

    #[tokio::test]
    async fn wait_time_stats_tracked() {
        let pool = Pool::new(
            TestResource,
            test_config(),
            PoolConfig {
                min_size: 0,
                max_size: 2,
                ..Default::default()
            },
        )
        .unwrap();
        let ctx = test_ctx();

        let (guard, wait) = pool.acquire(&ctx).await.unwrap();
        drop(guard);

        // wait_duration should be reasonable (sub-second on fast machines)
        assert!(
            wait < Duration::from_secs(5),
            "wait should be reasonable, got {wait:?}"
        );

        let stats = pool.stats();
        assert_eq!(stats.total_acquisitions, 1);
        // total_wait_time_ms might be 0 if acquire was instant
        assert!(stats.total_wait_time_ms <= 1000);
    }

    #[tokio::test]
    async fn create_failure_does_not_leak_semaphore_permit() {
        let resource = FailingCreateResource {
            remaining_failures: std::sync::atomic::AtomicU32::new(1),
        };
        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 1,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let pool = Pool::new(resource, test_config(), pool_config).unwrap();

        // First acquire should fail (create returns Err)
        let result = pool.acquire(&test_ctx()).await;
        assert!(result.is_err(), "first acquire should fail (create error)");

        // Second acquire should succeed (permit was returned, create now works)
        let (guard, _) = pool
            .acquire(&test_ctx())
            .await
            .expect("second acquire should succeed — permit must not be leaked");
        assert_eq!(*guard, "test-inst");
    }

    #[tokio::test]
    async fn acquire_respects_cancellation() {
        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 1,
            acquire_timeout: Duration::from_secs(10),
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();

        // Hold one guard so the pool is exhausted
        let (_g, _) = pool.acquire(&test_ctx()).await.unwrap();

        // Create a context with a cancellation token
        let token = tokio_util::sync::CancellationToken::new();
        let ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
            .with_cancellation(token.clone());

        // Cancel after 50ms
        let cancel_token = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancel_token.cancel();
        });

        // Acquire should fail due to cancellation, not wait 10s for timeout
        let start = Instant::now();
        let result = pool.acquire(&ctx).await;
        let elapsed = start.elapsed();

        assert!(result.is_err(), "acquire should fail when cancelled");
        assert!(
            elapsed < Duration::from_secs(1),
            "should fail quickly via cancellation, not wait for timeout (took {:?})",
            elapsed
        );
    }

    // ---------------------------------------------------------------
    // T012: Automatic maintenance scheduling
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn maintenance_task_replenishes_pool() {
        let pool_config = PoolConfig {
            min_size: 2,
            max_size: 5,
            acquire_timeout: Duration::from_secs(1),
            maintenance_interval: Some(Duration::from_millis(50)),
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();

        // Initially no idle instances (min_size is not pre-filled by new())
        let stats = pool.stats();
        assert_eq!(stats.idle, 0, "pool starts with 0 idle");

        // Wait for at least one maintenance cycle to run
        tokio::time::sleep(Duration::from_millis(150)).await;

        let stats = pool.stats();
        assert!(
            stats.idle >= 2,
            "maintenance task should replenish to min_size, got idle={}",
            stats.idle
        );
    }

    #[tokio::test]
    async fn maintenance_task_cancelled_on_shutdown() {
        let pool_config = PoolConfig {
            min_size: 2,
            max_size: 5,
            acquire_timeout: Duration::from_secs(1),
            maintenance_interval: Some(Duration::from_millis(50)),
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();

        // Let maintenance run once
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(pool.stats().idle >= 2);

        // Shutdown should cancel the maintenance task
        pool.shutdown().await.unwrap();

        // After shutdown, pool should be cleaned up
        assert_eq!(pool.stats().idle, 0, "shutdown should clean idle instances");
    }

    #[tokio::test]
    async fn no_maintenance_task_when_interval_is_none() {
        let pool_config = PoolConfig {
            min_size: 2,
            max_size: 5,
            acquire_timeout: Duration::from_secs(1),
            maintenance_interval: None, // explicitly None
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();

        // Wait a bit — no maintenance task should be running
        tokio::time::sleep(Duration::from_millis(100)).await;

        let stats = pool.stats();
        assert_eq!(
            stats.idle, 0,
            "without maintenance_interval, pool should not auto-replenish"
        );
    }

    // ---------------------------------------------------------------
    // T019: Pool selection strategy (FIFO / LIFO)
    // ---------------------------------------------------------------

    #[test]
    fn pool_strategy_default_is_fifo() {
        assert_eq!(PoolStrategy::default(), PoolStrategy::Fifo);
    }

    #[test]
    fn pool_config_default_strategy_is_fifo() {
        let config = PoolConfig::default();
        assert_eq!(config.strategy, PoolStrategy::Fifo);
    }

    /// FIFO: returning A then B, next acquire should yield A (oldest).
    #[tokio::test]
    async fn fifo_strategy_returns_oldest_first() {
        // We use a resource whose instances encode creation order.
        use std::sync::atomic::{AtomicU32, Ordering};

        struct CountingResource(AtomicU32);
        impl Resource for CountingResource {
            type Config = TestConfig;
            type Instance = String;
            fn metadata(&self) -> ResourceMetadata {
                ResourceMetadata::from_key(ResourceKey::try_from("counting").expect("valid"))
            }

            async fn create(&self, _cfg: &TestConfig, _ctx: &Context) -> Result<String> {
                let n = self.0.fetch_add(1, Ordering::SeqCst);
                Ok(format!("inst-{n}"))
            }
        }

        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 3,
            acquire_timeout: Duration::from_secs(1),
            strategy: PoolStrategy::Fifo,
            ..Default::default()
        };
        let pool = Pool::new(
            CountingResource(AtomicU32::new(0)),
            test_config(),
            pool_config,
        )
        .unwrap();

        // Acquire two instances: inst-0, inst-1
        let (g0, _) = pool.acquire(&test_ctx()).await.unwrap();
        let (g1, _) = pool.acquire(&test_ctx()).await.unwrap();
        assert_eq!(*g0, "inst-0");
        assert_eq!(*g1, "inst-1");

        // Return both (g0 first, then g1 — so queue is [inst-0, inst-1])
        drop(g0);
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(g1);
        tokio::time::sleep(Duration::from_millis(30)).await;

        // FIFO: next acquire should return inst-0 (oldest)
        let (g_next, _) = pool.acquire(&test_ctx()).await.unwrap();
        assert_eq!(*g_next, "inst-0", "FIFO should return oldest idle first");
    }

    /// LIFO: returning A then B, next acquire should yield B (most recent).
    #[tokio::test]
    async fn lifo_strategy_returns_newest_first() {
        use std::sync::atomic::{AtomicU32, Ordering};

        struct CountingResource(AtomicU32);
        impl Resource for CountingResource {
            type Config = TestConfig;
            type Instance = String;
            fn metadata(&self) -> ResourceMetadata {
                ResourceMetadata::from_key(ResourceKey::try_from("counting").expect("valid"))
            }

            async fn create(&self, _cfg: &TestConfig, _ctx: &Context) -> Result<String> {
                let n = self.0.fetch_add(1, Ordering::SeqCst);
                Ok(format!("inst-{n}"))
            }
        }

        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 3,
            acquire_timeout: Duration::from_secs(1),
            strategy: PoolStrategy::Lifo,
            ..Default::default()
        };
        let pool = Pool::new(
            CountingResource(AtomicU32::new(0)),
            test_config(),
            pool_config,
        )
        .unwrap();

        // Acquire two instances: inst-0, inst-1
        let (g0, _) = pool.acquire(&test_ctx()).await.unwrap();
        let (g1, _) = pool.acquire(&test_ctx()).await.unwrap();
        assert_eq!(*g0, "inst-0");
        assert_eq!(*g1, "inst-1");

        // Return both (g0 first, then g1 — so queue is [inst-0, inst-1])
        drop(g0);
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(g1);
        tokio::time::sleep(Duration::from_millis(30)).await;

        // LIFO: next acquire should return inst-1 (most recently used)
        let (g_next, _) = pool.acquire(&test_ctx()).await.unwrap();
        assert_eq!(*g_next, "inst-1", "LIFO should return newest idle first");
    }

    // ---------------------------------------------------------------
    // Credential rotation (handle_rotation)
    // ---------------------------------------------------------------

    /// CredentialHandler that counts authorize calls for testing.
    struct CountingCredentialHandler {
        count: std::sync::atomic::AtomicUsize,
        strategy: RotationStrategy,
    }

    impl CredentialHandler<String> for CountingCredentialHandler {
        fn authorize(&self, instance: &mut String, state: &serde_json::Value) -> Result<()> {
            self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Some(token) = state.get("token").and_then(|v| v.as_str()) {
                instance.clear();
                instance.push_str(token);
            }
            Ok(())
        }

        fn rotation_strategy(&self) -> RotationStrategy {
            self.strategy
        }
    }

    #[tokio::test]
    async fn handle_rotation_hot_swap_calls_authorize_on_idle() {
        let handler = Arc::new(CountingCredentialHandler {
            count: std::sync::atomic::AtomicUsize::new(0),
            strategy: RotationStrategy::HotSwap,
        });
        let pool = Pool::with_hooks_and_credential(
            TestResource,
            test_config(),
            PoolConfig::default(),
            None,
            None,
            Some(Arc::clone(&handler) as Arc<dyn CredentialHandler<String>>),
        )
        .unwrap();

        // Acquire and release to put instance in idle (return is async, wait for it)
        let (guard, _) = pool.acquire(&test_ctx()).await.unwrap();
        drop(guard);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let initial_count = handler.count.load(std::sync::atomic::Ordering::SeqCst);
        let cred_key = nebula_core::CredentialKey::new("test").expect("valid key");
        pool.handle_rotation(
            &serde_json::json!({"token": "rotated"}),
            RotationStrategy::HotSwap,
            cred_key,
        )
        .await
        .unwrap();

        assert!(
            handler.count.load(std::sync::atomic::Ordering::SeqCst) > initial_count,
            "authorize should be called on idle instance during HotSwap"
        );
    }

    #[tokio::test]
    async fn handle_rotation_drain_and_recreate_evicts_idle() {
        let handler = Arc::new(CountingCredentialHandler {
            count: std::sync::atomic::AtomicUsize::new(0),
            strategy: RotationStrategy::DrainAndRecreate,
        });
        let pool = Pool::with_hooks_and_credential(
            TestResource,
            test_config(),
            PoolConfig::default(),
            None,
            None,
            Some(Arc::clone(&handler) as Arc<dyn CredentialHandler<String>>),
        )
        .unwrap();

        // Acquire and release to put instance in idle (return is async, wait for it)
        let (guard, _) = pool.acquire(&test_ctx()).await.unwrap();
        drop(guard);
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(pool.stats().idle, 1, "one idle before rotation");

        let cred_key = nebula_core::CredentialKey::new("test").expect("valid key");
        pool.handle_rotation(
            &serde_json::json!({"token": "new"}),
            RotationStrategy::DrainAndRecreate,
            cred_key,
        )
        .await
        .unwrap();

        assert_eq!(pool.stats().idle, 0, "DrainAndRecreate should evict idle");
    }

    #[tokio::test]
    async fn handle_rotation_without_handler_returns_error() {
        let pool = Pool::new(TestResource, test_config(), PoolConfig::default()).unwrap();
        let cred_key = nebula_core::CredentialKey::new("test").expect("valid key");
        let err = pool
            .handle_rotation(
                &serde_json::json!({"token": "x"}),
                RotationStrategy::HotSwap,
                cred_key,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, Error::CredentialNotConfigured { .. }));
    }
}
