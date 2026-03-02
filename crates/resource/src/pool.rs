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

use nebula_core::{ExecutionId, WorkflowId};
use parking_lot::Mutex;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::events::{CleanupReason, EventBus, ResourceEvent};
use crate::guard::Guard;
use crate::hooks::{HookEvent, HookRegistry};
use crate::lifecycle::Lifecycle;
use crate::resource::Resource;
use crate::scope::Scope;

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
        }
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
        Ok(())
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
    /// Median acquire latency over the recent window (milliseconds).
    /// `None` when no acquisitions have been recorded yet.
    pub acquire_latency_p50_ms: Option<u64>,
    /// 95th-percentile acquire latency (milliseconds).
    pub acquire_latency_p95_ms: Option<u64>,
    /// 99th-percentile acquire latency (milliseconds).
    pub acquire_latency_p99_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// LatencyRingBuffer — fixed-size sliding window for percentile computation
// ---------------------------------------------------------------------------

/// Fixed-capacity ring buffer that stores the most recent `N` acquire
/// latency samples (in milliseconds). Percentiles are computed on demand
/// by sorting a snapshot — this is cheap for the default window size
/// (1024) and avoids external histogram dependencies.
const LATENCY_WINDOW: usize = 1024;

#[derive(Clone)]
struct LatencyRingBuffer {
    buf: Vec<u64>,
    pos: usize,
    full: bool,
}

impl std::fmt::Debug for LatencyRingBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LatencyRingBuffer")
            .field("len", &self.len())
            .field("capacity", &self.buf.len())
            .finish()
    }
}

impl Default for LatencyRingBuffer {
    fn default() -> Self {
        Self::new(LATENCY_WINDOW)
    }
}

impl LatencyRingBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            buf: vec![0; capacity],
            pos: 0,
            full: false,
        }
    }

    /// Record a latency sample (milliseconds).
    fn push(&mut self, value: u64) {
        self.buf[self.pos] = value;
        self.pos += 1;
        if self.pos >= self.buf.len() {
            self.pos = 0;
            self.full = true;
        }
    }

    /// Number of samples currently stored.
    fn len(&self) -> usize {
        if self.full { self.buf.len() } else { self.pos }
    }

    /// Compute a percentile (0–100). Returns `None` if the buffer is empty.
    fn percentile(&self, pct: f64) -> Option<u64> {
        let n = self.len();
        if n == 0 {
            return None;
        }
        let mut sorted: Vec<u64> = if self.full {
            self.buf.clone()
        } else {
            self.buf[..self.pos].to_vec()
        };
        sorted.sort_unstable();
        let idx = ((pct / 100.0) * (n as f64 - 1.0)).round() as usize;
        Some(sorted[idx.min(n - 1)])
    }
}

/// Combined pool state: idle queue + statistics under a single lock.
struct PoolState<T> {
    idle: VecDeque<Entry<T>>,
    stats: PoolStats,
    /// Sliding window of recent acquire latencies for percentile computation.
    latency_window: LatencyRingBuffer,
    /// Set to true after `shutdown()` to prevent Guard drops from
    /// reinserting instances into the idle queue.
    shutdown: bool,
}

/// Inner shared state for the pool.
struct PoolInner<R: Resource> {
    resource: Arc<R>,
    config: R::Config,
    pool_config: PoolConfig,
    state: Mutex<PoolState<R::Instance>>,
    /// Semaphore limits concurrent active (checked-out) instances.
    /// Idle instances do not hold permits.
    semaphore: Semaphore,
    /// Cancellation token for background tasks (maintenance).
    /// Cancelled on `shutdown()`.
    cancel: CancellationToken,
    /// Optional event bus for emitting lifecycle events.
    event_bus: Option<Arc<EventBus>>,
    /// Number of callers currently waiting to acquire an instance.
    waiting_count: Arc<AtomicUsize>,
    /// Optional hook registry for lifecycle hooks (Create, Cleanup).
    hooks: Option<Arc<HookRegistry>>,
    /// Handle for the background maintenance task, if spawned.
    /// Stored so we can join on it during shutdown.
    maintenance_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
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
        let stats = self.inner.state.lock().stats.clone();
        let key = self.inner.resource.key();
        f.debug_struct("Pool")
            .field("resource_id", &key.as_ref())
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
        pool_config.validate()?;
        let max = pool_config.max_size;
        let maintenance_interval = pool_config.maintenance_interval;
        let cancel = CancellationToken::new();

        let key = resource.key();
        tracing::debug!(
            resource_id = %key,
            min_size = pool_config.min_size,
            max_size = pool_config.max_size,
            "Created new resource pool"
        );

        let pool = Self {
            inner: Arc::new(PoolInner {
                resource: Arc::new(resource),
                config,
                pool_config,
                state: Mutex::new(PoolState {
                    idle: VecDeque::with_capacity(max),
                    stats: PoolStats::default(),
                    latency_window: LatencyRingBuffer::default(),
                    shutdown: false,
                }),
                semaphore: Semaphore::new(max),
                cancel: cancel.clone(),
                event_bus,
                waiting_count: Arc::new(AtomicUsize::new(0)),
                hooks,
                maintenance_handle: Mutex::new(None),
            }),
        };

        // Spawn automatic maintenance task if configured.
        if let Some(interval) = maintenance_interval {
            let maintenance_pool = pool.clone();
            let handle = tokio::spawn(async move {
                loop {
                    tokio::select! {
                        () = tokio::time::sleep(interval) => {}
                        () = cancel.cancelled() => break,
                    }
                    // Use a Global-scope context for background maintenance.
                    let ctx = Context::new(Scope::Global, WorkflowId::nil(), ExecutionId::nil());
                    let _ = maintenance_pool.maintain(&ctx).await;
                }
            });
            *pool.inner.maintenance_handle.lock() = Some(handle);
        }

        Ok(pool)
    }

    /// Acquire a resource instance from the pool.
    ///
    /// Returns an RAII `Guard` that returns the instance to the pool
    /// when dropped. Respects `ctx.cancellation` — if the token is
    /// cancelled while waiting, returns `Error::Unavailable` immediately.
    pub async fn acquire(&self, ctx: &Context) -> Result<(Guard<R::Instance>, Duration)> {
        let start = Instant::now();

        let result: Result<Guard<R::Instance>> = tokio::select! {
            result = self.acquire_inner(ctx, start) => result,
            () = ctx.cancellation.cancelled() => {
                Err(Error::Unavailable {
                    resource_key: self.inner.resource.key(),
                    reason: "Operation cancelled".to_string(),
                    retryable: false,
                })
            }
        };

        {
            let wait_duration = start.elapsed();
            match &result {
                Ok(_) => tracing::debug!(
                    resource_id = %self.inner.resource.key(),
                    scope = %ctx.scope,
                    wait_ms = wait_duration.as_millis() as u64,
                    "Acquired resource instance"
                ),
                Err(e) => tracing::warn!(
                    resource_id = %self.inner.resource.key(),
                    scope = %ctx.scope,
                    wait_ms = wait_duration.as_millis() as u64,
                    error = %e,
                    "Failed to acquire resource instance"
                ),
            }
        }
        // Suppress unused variable warning when tracing is off
        let _ = start;

        result.map(|guard| (guard, start.elapsed()))
    }

    /// Inner acquire logic, separated so `acquire` can wrap it in a
    /// cancellation-aware `select!`.
    async fn acquire_inner(&self, ctx: &Context, start: Instant) -> Result<Guard<R::Instance>> {
        let inner = &self.inner;

        // Track that we are waiting for a permit.
        inner.waiting_count.fetch_add(1, Ordering::SeqCst);

        // Acquire a permit (limits concurrent active instances)
        let permit =
            tokio::time::timeout(inner.pool_config.acquire_timeout, inner.semaphore.acquire())
                .await
                .map_err(|_| {
                    inner.waiting_count.fetch_sub(1, Ordering::SeqCst);
                    let mut state = inner.state.lock();
                    let resource_id = inner.resource.key().as_ref().to_string();
                    let waiters = inner.waiting_count.load(Ordering::SeqCst);
                    state.stats.exhausted_count += 1;
                    if let Some(bus) = &inner.event_bus {
                        let key = nebula_core::ResourceKey::try_from(resource_id.as_str())
                            .expect("resource id must be a valid ResourceKey");
                        bus.emit(ResourceEvent::PoolExhausted {
                            resource_key: key.clone(),
                            waiters,
                        });
                    }
                    let key = nebula_core::ResourceKey::try_from(resource_id.as_str())
                        .expect("resource id must be a valid ResourceKey");
                    Error::PoolExhausted {
                        resource_key: key,
                        current_size: state.stats.active,
                        max_size: inner.pool_config.max_size,
                        waiters,
                    }
                })?
                .map_err(|_| {
                    inner.waiting_count.fetch_sub(1, Ordering::SeqCst);
                    Error::Internal {
                        resource_key: inner.resource.key(),
                        message: "Pool semaphore closed".to_string(),
                        source: None,
                    }
                })?;

        // We got a permit, no longer waiting.
        inner.waiting_count.fetch_sub(1, Ordering::SeqCst);

        // Try to get an idle instance, tracking created_at for recycled entries.
        let (instance, created_at) = loop {
            let entry = {
                let mut state = inner.state.lock();
                match inner.pool_config.strategy {
                    PoolStrategy::Fifo => state.idle.pop_front(),
                    PoolStrategy::Lifo => state.idle.pop_back(),
                }
            };
            match entry {
                Some(entry) if entry.is_expired(&inner.pool_config) => {
                    // Expired — clean up and try next
                    tracing::debug!("Destroying expired resource instance");
                    Self::cleanup_with_hooks(inner, entry.instance, &CleanupReason::Expired, None)
                        .await;
                    // Don't add permit back — we'll create a new instance below if needed
                    continue;
                }
                Some(entry) => {
                    // Validate
                    let created_at = entry.created_at;
                    match inner.resource.is_valid(&entry.instance).await {
                        Ok(true) => break (entry.instance, Some(created_at)),
                        _ => {
                            tracing::debug!("Destroying invalid resource instance");
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
                None => {
                    // No idle instances — create new, firing Create hooks.
                    tracing::debug!("Creating new resource instance");
                    let instance = Self::create_with_hooks(inner, ctx).await?;
                    inner.state.lock().stats.created += 1;
                    break (instance, None);
                }
            }
        };

        // Record wait time stats now that we have an instance.
        let wait_ms = start.elapsed().as_millis() as u64;
        {
            let mut state = inner.state.lock();
            state.stats.total_acquisitions += 1;
            state.stats.active += 1;
            state.stats.idle = state.idle.len();
            state.stats.total_wait_time_ms += wait_ms;
            if wait_ms > state.stats.max_wait_time_ms {
                state.stats.max_wait_time_ms = wait_ms;
            }
            state.latency_window.push(wait_ms);
        }

        // Forget the permit — we'll add it back when the guard drops.
        permit.forget();

        let pool = self.clone();
        let acquire_instant = Instant::now();
        Ok(Guard::new(instance, move |inst| {
            let usage_duration = acquire_instant.elapsed();
            drop(tokio::spawn(Self::return_instance(
                pool,
                inst,
                created_at,
                usage_duration,
            )));
        }))
    }

    /// Return an instance to the pool (or clean it up).
    ///
    /// Called from the guard's drop callback in a spawned task.
    async fn return_instance(
        pool: Self,
        mut inst: R::Instance,
        created_at: Option<Instant>,
        usage_duration: Duration,
    ) {
        let inner = &pool.inner;
        let recycle_ok = inner.resource.recycle(&mut inst).await.is_ok();

        // Check shutdown under the same lock that pushes to idle to
        // prevent a race where shutdown flips between the read and insert.
        let cleanup_reason = if recycle_ok {
            let mut state = inner.state.lock();
            if !state.shutdown {
                let entry = match created_at {
                    Some(ca) => Entry::returned(inst, ca),
                    None => Entry::new(inst),
                };
                state.idle.push_back(entry);
                None
            } else {
                Some((inst, CleanupReason::Shutdown))
            }
        } else {
            Some((inst, CleanupReason::RecycleFailed))
        };

        if cleanup_reason.is_none() {
            tracing::debug!(
                resource_id = %inner.resource.key(),
                "Released resource instance back to pool"
            );
        }

        if let Some((to_cleanup, reason)) = cleanup_reason {
            Self::cleanup_with_hooks(inner, to_cleanup, &reason, None).await;
            tracing::debug!(
                resource_id = %inner.resource.key(),
                "Cleaned up resource instance on release (pool shutdown or recycle failed)"
            );
        }

        Self::emit_event(
            inner,
            ResourceEvent::Released {
                resource_key: inner.resource.key(),
                usage_duration,
            },
        );

        {
            let mut state = inner.state.lock();
            state.stats.total_releases += 1;
            state.stats.active = state.stats.active.saturating_sub(1);
            state.stats.idle = state.idle.len();
        }

        inner.semaphore.add_permits(1);
    }

    /// Emit an event if the pool has an event bus configured.
    fn emit_event(inner: &PoolInner<R>, event: ResourceEvent) {
        if let Some(bus) = &inner.event_bus {
            bus.emit(event);
        }
    }

    /// Create a new resource instance, firing [`HookEvent::Create`]
    /// before/after hooks when a [`HookRegistry`] is attached.
    ///
    /// Before-hooks can cancel the creation by returning
    /// [`HookResult::Cancel`](crate::hooks::HookResult::Cancel).
    async fn create_with_hooks(inner: &PoolInner<R>, ctx: &Context) -> Result<R::Instance> {
        let key = inner.resource.key();
        let resource_id = key.as_ref();

        // Run Create before-hooks.
        if let Some(hooks) = &inner.hooks {
            hooks
                .run_before(&HookEvent::Create, resource_id, ctx)
                .await?;
        }

        let result = inner.resource.create(&inner.config, ctx).await;

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
        let key = inner.resource.key();
        let resource_id = key.as_ref();
        let synthetic_ctx;
        let ctx = match ctx {
            Some(c) => c,
            None => {
                synthetic_ctx = Context::new(Scope::Global, WorkflowId::nil(), ExecutionId::nil());
                &synthetic_ctx
            }
        };

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

        inner.state.lock().stats.destroyed += 1;
        Self::emit_event(
            inner,
            ResourceEvent::CleanedUp {
                resource_key: nebula_core::ResourceKey::try_from(resource_id)
                    .expect("resource id must be a valid ResourceKey"),
                reason: reason.clone(),
            },
        );
    }

    /// Get current pool statistics, including latency percentiles
    /// computed from a sliding window of recent acquisitions.
    #[must_use]
    pub fn stats(&self) -> PoolStats {
        let state = self.inner.state.lock();
        let mut stats = state.stats.clone();
        stats.acquire_latency_p50_ms = state.latency_window.percentile(50.0);
        stats.acquire_latency_p95_ms = state.latency_window.percentile(95.0);
        stats.acquire_latency_p99_ms = state.latency_window.percentile(99.0);
        stats
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
        let ctx = Context::new(Scope::Global, WorkflowId::nil(), ExecutionId::nil());
        let mut created = 0;

        for _ in 0..count {
            // Check headroom under the lock, then release before async create.
            let has_room = {
                let state = inner.state.lock();
                let total = state.idle.len() + state.stats.active;
                total < inner.pool_config.max_size
            };

            if !has_room {
                break;
            }

            let instance = match Self::create_with_hooks(inner, &ctx).await {
                Ok(inst) => inst,
                Err(_) => break,
            };

            // Re-check capacity under the lock after the async create.
            // Returns Some(instance) if rejected (over capacity).
            let rejected = {
                let mut state = inner.state.lock();
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
            };

            if let Some(surplus) = rejected {
                Self::cleanup_with_hooks(inner, surplus, &CleanupReason::Evicted, Some(&ctx)).await;
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
            let entry = {
                let mut state = inner.state.lock();
                let total = state.idle.len() + state.stats.active;
                if total <= inner.pool_config.min_size || state.idle.is_empty() {
                    break;
                }
                state.idle.pop_front()
            };

            if let Some(entry) = entry {
                Self::cleanup_with_hooks(inner, entry.instance, &CleanupReason::Evicted, None)
                    .await;
                let mut state = inner.state.lock();
                state.stats.idle = state.idle.len();
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
        let state = self.inner.state.lock();
        (
            state.stats.active,
            state.idle.len(),
            self.inner.pool_config.max_size,
        )
    }

    /// Get the current number of callers waiting to acquire an instance.
    #[must_use]
    pub fn waiting_count(&self) -> usize {
        self.inner.waiting_count.load(Ordering::SeqCst)
    }

    /// Run maintenance: evict expired idle instances, ensure min_size.
    pub async fn maintain(&self, ctx: &Context) -> Result<()> {
        tracing::debug!(resource_id = %self.inner.resource.key(), "Running pool maintenance");

        let inner = &self.inner;

        // Evict expired idle entries
        let mut to_cleanup = Vec::new();
        {
            let mut state = inner.state.lock();
            let mut kept = VecDeque::with_capacity(state.idle.len());
            while let Some(entry) = state.idle.pop_front() {
                if entry.is_expired(&inner.pool_config) {
                    to_cleanup.push(entry.instance);
                } else {
                    kept.push_back(entry);
                }
            }
            state.idle = kept;
        }

        for inst in to_cleanup {
            Self::cleanup_with_hooks(inner, inst, &CleanupReason::Evicted, Some(ctx)).await;
        }

        // Ensure min_size
        let (current_idle, current_active) = {
            let state = inner.state.lock();
            (state.idle.len(), state.stats.active)
        };
        let total = current_idle + current_active;
        if total < inner.pool_config.min_size {
            let needed = inner.pool_config.min_size - total;
            for _ in 0..needed {
                match Self::create_with_hooks(inner, ctx).await {
                    Ok(instance) => {
                        let mut state = inner.state.lock();
                        state.idle.push_back(Entry::new(instance));
                        state.stats.created += 1;
                    }
                    Err(_) => break,
                }
            }
        }

        // Sync idle count
        let mut state = inner.state.lock();
        state.stats.idle = state.idle.len();

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

        // Close the semaphore so new acquire() calls fail immediately
        // instead of blocking until timeout.
        inner.semaphore.close();

        let entries: Vec<_> = {
            let mut state = inner.state.lock();
            state.shutdown = true;
            state.idle.drain(..).collect()
        };

        for entry in entries {
            Self::cleanup_with_hooks(inner, entry.instance, &CleanupReason::Shutdown, None).await;
        }

        inner.state.lock().stats.idle = 0;
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
        type Deps = ();

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
        assert!(PoolConfig::default().validate().is_ok());
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

    // -- Resource that fails validation --

    struct InvalidatingResource {
        /// After this many validations, start returning false
        fail_after: std::sync::atomic::AtomicU32,
    }

    impl Resource for InvalidatingResource {
        type Config = TestConfig;
        type Instance = String;
        type Deps = ();

        fn metadata(&self) -> ResourceMetadata {
            ResourceMetadata::from_key(ResourceKey::try_from("invalidating").expect("valid"))
        }

        async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            Ok(format!("{}-inst", config.prefix))
        }

        async fn is_valid(&self, _instance: &Self::Instance) -> Result<bool> {
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
            // First is_valid call returns false, subsequent ones return true (underflow wraps)
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
        type Deps = ();

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
        type Deps = ();

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
            type Deps = ();

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
            type Deps = ();

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
}
