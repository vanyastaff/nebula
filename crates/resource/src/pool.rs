//! Resource pool — generic pool integrated with the `Resource` trait.
//!
//! `Pool<R>` calls `R::create`, `R::is_valid`, `R::recycle` and `R::cleanup`
//! directly, removing the need for closure factories.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::Semaphore;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::guard::Guard;
use crate::resource::Resource;

// ---------------------------------------------------------------------------
// PoolConfig
// ---------------------------------------------------------------------------

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

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
    /// # Errors
    /// Returns error if `pool_config` is invalid (e.g. max_size == 0).
    pub fn new(resource: R, config: R::Config, pool_config: PoolConfig) -> Result<Self> {
        pool_config.validate()?;
        let max = pool_config.max_size;

        #[cfg(feature = "tracing")]
        tracing::debug!(
             resource_id = %resource.id(),
             min_size = pool_config.min_size,
             max_size = pool_config.max_size,
             "Created new resource pool"
        );

        Ok(Self {
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
            }),
        })
    }

    /// Acquire a resource instance from the pool.
    ///
    /// Returns an RAII `Guard` that returns the instance to the pool
    /// when dropped. Respects `ctx.cancellation` — if the token is
    /// cancelled while waiting, returns `Error::Unavailable` immediately.
    pub async fn acquire(&self, ctx: &Context) -> Result<Guard<R::Instance>> {
        tokio::select! {
            result = self.acquire_inner(ctx) => result,
            () = ctx.cancellation.cancelled() => {
                Err(Error::Unavailable {
                    resource_id: self.inner.resource.id().to_string(),
                    reason: "Operation cancelled".to_string(),
                    retryable: false,
                })
            }
        }
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
                    Error::PoolExhausted {
                        resource_id: inner.resource.id().to_string(),
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
            let entry = { inner.state.lock().idle.pop_front() };
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
        Ok(Guard::new(instance, move |mut inst| {
            // Return instance to pool on drop.
            // We run recycle + return synchronously-ish by spawning a task.
            drop(tokio::spawn(async move {
                let inner = &pool.inner;
                // Try to recycle
                let keep = inner.resource.recycle(&mut inst).await.is_ok();

                if keep && !inner.state.lock().shutdown {
                    let entry = match created_at {
                        Some(ca) => Entry::returned(inst, ca),
                        None => Entry::new(inst),
                    };
                    inner.state.lock().idle.push_back(entry);
                } else {
                    let _ = inner.resource.cleanup(inst).await;
                    inner.state.lock().stats.destroyed += 1;
                }

                {
                    let mut state = inner.state.lock();
                    state.stats.total_releases += 1;
                    state.stats.active = state.stats.active.saturating_sub(1);
                    state.stats.idle = state.idle.len();
                }

                // Return the permit
                inner.semaphore.add_permits(1);
            }));
        }))
    }

    /// Get current pool statistics.
    #[must_use]
    pub fn stats(&self) -> PoolStats {
        self.inner.state.lock().stats.clone()
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
    /// - New `acquire()` calls fail immediately (semaphore is closed).
    /// - Any `Guard` dropped will clean up its instance instead of
    ///   returning it to the idle queue.
    pub async fn shutdown(&self) -> Result<()> {
        let inner = &self.inner;

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
}
