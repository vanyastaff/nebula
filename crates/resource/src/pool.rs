//! Resource pool — generic pool integrated with the `Resource` trait.
//!
//! `Pool<R>` calls `R::create`, `R::is_valid`, `R::recycle` and `R::cleanup`
//! directly, removing the need for closure factories.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::events::{CleanupReason, EventBus, ResourceEvent};
use crate::guard::Guard;
use crate::resource::Resource;
use crate::scope::Scope;

// ---------------------------------------------------------------------------
// PoolConfig
// ---------------------------------------------------------------------------

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Strategy for selecting idle instances from the pool.
///
/// Controls whether the most-recently-used or least-recently-used
/// idle instance is returned on acquire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
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
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
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
    #[cfg_attr(feature = "serde", serde(default))]
    pub maintenance_interval: Option<Duration>,
    /// Strategy for selecting idle instances on acquire.
    /// Default: [`PoolStrategy::Fifo`].
    #[cfg_attr(feature = "serde", serde(default))]
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

/// A pool entry wrapping a resource instance.
struct Entry<T> {
    instance: T,
    created_at: Instant,
    last_used: Instant,
}

impl<T> Entry<T> {
    fn new(instance: T) -> Self {
        let now = Instant::now();
        Self {
            instance,
            created_at: now,
            last_used: now,
        }
    }

    /// Return an entry to the pool, preserving the original `created_at`.
    fn returned(instance: T, created_at: Instant) -> Self {
        Self {
            instance,
            created_at,
            last_used: Instant::now(),
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
}

/// Combined pool state: idle queue + statistics under a single lock.
struct PoolState<T> {
    idle: VecDeque<Entry<T>>,
    stats: PoolStats,
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
        f.debug_struct("Pool")
            .field("resource_id", &self.inner.resource.id())
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
        Self::with_event_bus(resource, config, pool_config, None)
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
        pool_config.validate()?;
        let max = pool_config.max_size;
        let maintenance_interval = pool_config.maintenance_interval;
        let cancel = CancellationToken::new();

        #[cfg(feature = "tracing")]
        tracing::debug!(
             resource_id = %resource.id(),
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
                    shutdown: false,
                }),
                semaphore: Semaphore::new(max),
                cancel: cancel.clone(),
                event_bus,
            }),
        };

        // Spawn automatic maintenance task if configured.
        if let Some(interval) = maintenance_interval {
            let maintenance_pool = pool.clone();
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        () = tokio::time::sleep(interval) => {}
                        () = cancel.cancelled() => break,
                    }
                    // Use a Global-scope context for background maintenance.
                    let ctx = Context::new(Scope::Global, "maintenance", "maintenance");
                    let _ = maintenance_pool.maintain(&ctx).await;
                }
            });
        }

        Ok(pool)
    }

    /// Acquire a resource instance from the pool.
    ///
    /// Returns an RAII `Guard` that returns the instance to the pool
    /// when dropped. Respects `ctx.cancellation` — if the token is
    /// cancelled while waiting, returns `Error::Unavailable` immediately.
    pub async fn acquire(&self, ctx: &Context) -> Result<Guard<R::Instance>> {
        let start = Instant::now();

        let result = tokio::select! {
            result = self.acquire_inner(ctx) => result,
            () = ctx.cancellation.cancelled() => {
                Err(Error::Unavailable {
                    resource_id: self.inner.resource.id().to_string(),
                    reason: "Operation cancelled".to_string(),
                    retryable: false,
                })
            }
        };

        #[cfg(feature = "tracing")]
        {
            let wait_duration = start.elapsed();
            match &result {
                Ok(_) => tracing::debug!(
                    resource_id = %self.inner.resource.id(),
                    scope = %ctx.scope,
                    wait_ms = wait_duration.as_millis() as u64,
                    "Acquired resource instance"
                ),
                Err(e) => tracing::warn!(
                    resource_id = %self.inner.resource.id(),
                    scope = %ctx.scope,
                    wait_ms = wait_duration.as_millis() as u64,
                    error = %e,
                    "Failed to acquire resource instance"
                ),
            }
        }
        // Suppress unused variable warning when tracing is off
        let _ = start;

        result
    }

    /// Inner acquire logic, separated so `acquire` can wrap it in a
    /// cancellation-aware `select!`.
    async fn acquire_inner(&self, ctx: &Context) -> Result<Guard<R::Instance>> {
        let inner = &self.inner;

        // Acquire a permit (limits concurrent active instances)
        let permit =
            tokio::time::timeout(inner.pool_config.acquire_timeout, inner.semaphore.acquire())
                .await
                .map_err(|_| {
                    let state = inner.state.lock();
                    let resource_id = inner.resource.id().to_string();
                    if let Some(bus) = &inner.event_bus {
                        bus.emit(ResourceEvent::PoolExhausted {
                            resource_id: resource_id.clone(),
                            waiters: 0,
                        });
                    }
                    Error::PoolExhausted {
                        resource_id,
                        current_size: state.stats.active,
                        max_size: inner.pool_config.max_size,
                        waiters: 0,
                    }
                })?
                .map_err(|_| Error::Internal {
                    resource_id: inner.resource.id().to_string(),
                    message: "Pool semaphore closed".to_string(),
                    source: None,
                })?;

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
                    #[cfg(feature = "tracing")]
                    tracing::debug!("Destroying expired resource instance");
                    let _ = inner.resource.cleanup(entry.instance).await;
                    inner.state.lock().stats.destroyed += 1;
                    // Don't add permit back — we'll create a new instance below if needed
                    continue;
                }
                Some(entry) => {
                    // Validate
                    let created_at = entry.created_at;
                    match inner.resource.is_valid(&entry.instance).await {
                        Ok(true) => break (entry.instance, Some(created_at)),
                        _ => {
                            #[cfg(feature = "tracing")]
                            tracing::debug!("Destroying invalid resource instance");
                            let _ = inner.resource.cleanup(entry.instance).await;
                            inner.state.lock().stats.destroyed += 1;
                            continue;
                        }
                    }
                }
                None => {
                    // No idle instances — create new
                    #[cfg(feature = "tracing")]
                    tracing::debug!("Creating new resource instance");
                    let instance = inner.resource.create(&inner.config, ctx).await?;
                    inner.state.lock().stats.created += 1;
                    break (instance, None);
                }
            }
        };

        {
            let mut state = inner.state.lock();
            state.stats.total_acquisitions += 1;
            state.stats.active += 1;
            state.stats.idle = state.idle.len();
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
            #[cfg(feature = "tracing")]
            tracing::debug!(
                resource_id = %inner.resource.id(),
                "Released resource instance back to pool"
            );
        }

        if let Some((to_cleanup, reason)) = cleanup_reason {
            let _ = inner.resource.cleanup(to_cleanup).await;
            inner.state.lock().stats.destroyed += 1;
            Self::emit_event(
                inner,
                ResourceEvent::CleanedUp {
                    resource_id: inner.resource.id().to_string(),
                    reason,
                },
            );
            #[cfg(feature = "tracing")]
            tracing::debug!(
                resource_id = %inner.resource.id(),
                "Cleaned up resource instance on release (pool shutdown or recycle failed)"
            );
        }

        Self::emit_event(
            inner,
            ResourceEvent::Released {
                resource_id: inner.resource.id().to_string(),
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

    /// Get current pool statistics.
    #[must_use]
    pub fn stats(&self) -> PoolStats {
        self.inner.state.lock().stats.clone()
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
        let ctx = Context::new(Scope::Global, "autoscale", "autoscale");
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

            let instance = match inner.resource.create(&inner.config, &ctx).await {
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
                let _ = inner.resource.cleanup(surplus).await;
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
                let _ = inner.resource.cleanup(entry.instance).await;
                let mut state = inner.state.lock();
                state.stats.destroyed += 1;
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

    /// Run maintenance: evict expired idle instances, ensure min_size.
    pub async fn maintain(&self, ctx: &Context) -> Result<()> {
        #[cfg(feature = "tracing")]
        tracing::debug!(resource_id = %self.inner.resource.id(), "Running pool maintenance");

        let inner = &self.inner;

        // Evict expired idle entries
        let mut to_cleanup = Vec::new();
        {
            let mut state = inner.state.lock();
            let before = state.idle.len();
            let mut kept = VecDeque::with_capacity(state.idle.len());
            while let Some(entry) = state.idle.pop_front() {
                if entry.is_expired(&inner.pool_config) {
                    to_cleanup.push(entry.instance);
                } else {
                    kept.push_back(entry);
                }
            }
            state.idle = kept;
            let removed = before - state.idle.len();
            if removed > 0 {
                state.stats.destroyed += removed as u64;
            }
        }

        for inst in to_cleanup {
            let _ = inner.resource.cleanup(inst).await;
            if let Some(bus) = &inner.event_bus {
                bus.emit(ResourceEvent::CleanedUp {
                    resource_id: inner.resource.id().to_string(),
                    reason: CleanupReason::Evicted,
                });
            }
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
                match inner.resource.create(&inner.config, ctx).await {
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
    /// - Background maintenance task (if any) is cancelled.
    /// - New `acquire()` calls fail immediately (semaphore is closed).
    /// - Any `Guard` dropped will clean up its instance instead of
    ///   returning it to the idle queue.
    pub async fn shutdown(&self) -> Result<()> {
        let inner = &self.inner;

        // Cancel background maintenance task.
        inner.cancel.cancel();

        // Close the semaphore so new acquire() calls fail immediately
        // instead of blocking until timeout.
        inner.semaphore.close();

        let entries: Vec<_> = {
            let mut state = inner.state.lock();
            state.shutdown = true;
            state.idle.drain(..).collect()
        };

        for entry in entries {
            let _ = inner.resource.cleanup(entry.instance).await;
            inner.state.lock().stats.destroyed += 1;
            if let Some(bus) = &inner.event_bus {
                bus.emit(ResourceEvent::CleanedUp {
                    resource_id: inner.resource.id().to_string(),
                    reason: CleanupReason::Shutdown,
                });
            }
        }

        inner.state.lock().stats.idle = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{Config, Resource};
    use crate::scope::Scope;

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

        fn id(&self) -> &str {
            "test-resource"
        }

        async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            Ok(format!("{}-instance", config.prefix))
        }
    }

    fn test_ctx() -> Context {
        Context::new(Scope::Global, "wf-1", "ex-1")
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
        let guard = pool.acquire(&test_ctx()).await.unwrap();
        assert_eq!(*guard, "test-instance");
    }

    #[tokio::test]
    async fn pool_reuses_instances() {
        let pool = Pool::new(TestResource, test_config(), PoolConfig::default()).unwrap();

        // Acquire and drop to return to pool
        {
            let _guard = pool.acquire(&test_ctx()).await.unwrap();
        }
        // Give the spawn a moment to return the instance
        tokio::time::sleep(Duration::from_millis(50)).await;

        let stats = pool.stats();
        assert_eq!(stats.created, 1);

        // Acquire again — should reuse
        let _guard = pool.acquire(&test_ctx()).await.unwrap();
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

        let _g1 = pool.acquire(&test_ctx()).await.unwrap();
        let _g2 = pool.acquire(&test_ctx()).await.unwrap();

        // Third acquire should timeout
        let result = pool.acquire(&test_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_cleans_idle() {
        let pool = Pool::new(TestResource, test_config(), PoolConfig::default()).unwrap();

        {
            let _g = pool.acquire(&test_ctx()).await.unwrap();
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

        fn id(&self) -> &str {
            "invalidating"
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
            let _g = pool.acquire(&test_ctx()).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

        let stats = pool.stats();
        assert_eq!(stats.idle, 1, "instance should be in idle pool");

        // Next acquire: idle instance fails is_valid, gets destroyed, new one created
        let _g = pool.acquire(&test_ctx()).await.unwrap();

        let stats = pool.stats();
        assert_eq!(stats.destroyed, 1, "invalid instance should be destroyed");
        assert!(stats.created >= 2, "should have created a replacement");
    }

    // -- Resource that fails recycle --

    struct RecycleFailResource;

    impl Resource for RecycleFailResource {
        type Config = TestConfig;
        type Instance = String;

        fn id(&self) -> &str {
            "recycle-fail"
        }

        async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            Ok(format!("{}-inst", config.prefix))
        }

        async fn recycle(&self, _instance: &mut Self::Instance) -> Result<()> {
            Err(Error::Internal {
                resource_id: "recycle-fail".to_string(),
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
            let _g = pool.acquire(&test_ctx()).await.unwrap();
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
            let _g = pool.acquire(&test_ctx()).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Should still be able to acquire (permit was returned even though recycle failed)
        let _g = pool
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
            let _g = pool.acquire(&test_ctx()).await.unwrap();
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
            max_size: 3,
            acquire_timeout: Duration::from_millis(200),
            ..Default::default()
        };
        let pool = Pool::new(TestResource, test_config(), pool_config).unwrap();

        // Acquire max_size instances concurrently
        let mut guards = Vec::new();
        for _ in 0..3 {
            guards.push(pool.acquire(&test_ctx()).await.unwrap());
        }

        // Next acquire should fail
        let result = pool.acquire(&test_ctx()).await;
        assert!(result.is_err(), "should not exceed max_size");

        // Drop one guard, wait for return
        guards.pop();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Should succeed now
        let _g = pool
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

        // Acquire a guard, then shutdown the pool while still holding it
        let guard = pool.acquire(&test_ctx()).await.unwrap();
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

        fn id(&self) -> &str {
            "failing-create"
        }

        async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            let remaining = self
                .remaining_failures
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if remaining > 0 {
                return Err(Error::Initialization {
                    resource_id: "failing-create".to_string(),
                    reason: "intentional failure".to_string(),
                    source: None,
                });
            }
            Ok(format!("{}-inst", config.prefix))
        }
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
        assert!(result.is_err(), "first acquire should fail");

        // Second acquire should succeed (permit was returned, create now works)
        let guard = pool
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
        let _g = pool.acquire(&test_ctx()).await.unwrap();

        // Create a context with a cancellation token
        let token = tokio_util::sync::CancellationToken::new();
        let ctx = Context::new(Scope::Global, "wf-1", "ex-1").with_cancellation(token.clone());

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
            fn id(&self) -> &str {
                "counting"
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
        let g0 = pool.acquire(&test_ctx()).await.unwrap();
        let g1 = pool.acquire(&test_ctx()).await.unwrap();
        assert_eq!(*g0, "inst-0");
        assert_eq!(*g1, "inst-1");

        // Return both (g0 first, then g1 — so queue is [inst-0, inst-1])
        drop(g0);
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(g1);
        tokio::time::sleep(Duration::from_millis(30)).await;

        // FIFO: next acquire should return inst-0 (oldest)
        let g_next = pool.acquire(&test_ctx()).await.unwrap();
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
            fn id(&self) -> &str {
                "counting"
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
        let g0 = pool.acquire(&test_ctx()).await.unwrap();
        let g1 = pool.acquire(&test_ctx()).await.unwrap();
        assert_eq!(*g0, "inst-0");
        assert_eq!(*g1, "inst-1");

        // Return both (g0 first, then g1 — so queue is [inst-0, inst-1])
        drop(g0);
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(g1);
        tokio::time::sleep(Duration::from_millis(30)).await;

        // LIFO: next acquire should return inst-1 (most recently used)
        let g_next = pool.acquire(&test_ctx()).await.unwrap();
        assert_eq!(*g_next, "inst-1", "LIFO should return newest idle first");
    }
}
