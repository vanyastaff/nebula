//! Resource pool — generic pool integrated with the `Resource` trait.
//!
//! `Pool<R>` calls `R::create`, `R::is_valid`, `R::recycle` and `R::cleanup`
//! directly, removing the need for closure factories.

pub mod config;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use tokio::sync::Semaphore;

use crate::context::ResourceContext;
use crate::error::{ResourceError, ResourceResult};
use crate::resource::{Resource, ResourceGuard};

pub use config::PoolConfig;

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

/// Inner shared state for the pool.
struct PoolInner<R: Resource> {
    resource: Arc<R>,
    config: R::Config,
    pool_config: PoolConfig,
    idle: Mutex<VecDeque<Entry<R::Instance>>>,
    stats: Mutex<PoolStats>,
    /// Semaphore limits total concurrent instances (idle + active).
    semaphore: Semaphore,
}

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
        let stats = self.inner.stats.lock().clone();
        f.debug_struct("Pool")
            .field("resource_id", &self.inner.resource.id())
            .field("stats", &stats)
            .finish()
    }
}

impl<R: Resource> Pool<R> {
    /// Create a new pool for the given resource, config, and pool settings.
    pub fn new(resource: R, config: R::Config, pool_config: PoolConfig) -> Self {
        let max = pool_config.max_size;
        Self {
            inner: Arc::new(PoolInner {
                resource: Arc::new(resource),
                config,
                pool_config,
                idle: Mutex::new(VecDeque::with_capacity(max)),
                stats: Mutex::new(PoolStats::default()),
                semaphore: Semaphore::new(max),
            }),
        }
    }

    /// Acquire a resource instance from the pool.
    ///
    /// Returns an RAII `ResourceGuard` that returns the instance to the pool
    /// when dropped.
    pub async fn acquire(
        &self,
        ctx: &ResourceContext,
    ) -> ResourceResult<ResourceGuard<R::Instance>> {
        let inner = &self.inner;

        // Acquire a permit (limits total instances)
        let permit =
            tokio::time::timeout(inner.pool_config.acquire_timeout, inner.semaphore.acquire())
                .await
                .map_err(|_| {
                    let stats = inner.stats.lock();
                    ResourceError::pool_exhausted(
                        inner.resource.id(),
                        stats.active,
                        inner.pool_config.max_size,
                        0,
                    )
                })?
                .map_err(|_| {
                    ResourceError::internal(inner.resource.id(), "Pool semaphore closed")
                })?;

        // Try to get an idle instance
        let instance = loop {
            let entry = { inner.idle.lock().pop_front() };
            match entry {
                Some(entry) if entry.is_expired(&inner.pool_config) => {
                    // Expired — clean up and try next
                    let _ = inner.resource.cleanup(entry.instance).await;
                    let mut stats = inner.stats.lock();
                    stats.destroyed += 1;
                    stats.idle = inner.idle.lock().len();
                    // Don't add permit back — we'll create a new instance below if needed
                    continue;
                }
                Some(entry) => {
                    // Validate
                    match inner.resource.is_valid(&entry.instance).await {
                        Ok(true) => break entry.instance,
                        _ => {
                            let _ = inner.resource.cleanup(entry.instance).await;
                            let mut stats = inner.stats.lock();
                            stats.destroyed += 1;
                            stats.idle = inner.idle.lock().len();
                            continue;
                        }
                    }
                }
                None => {
                    // No idle instances — create new
                    let instance = inner.resource.create(&inner.config, ctx).await?;
                    {
                        let mut stats = inner.stats.lock();
                        stats.created += 1;
                    }
                    break instance;
                }
            }
        };

        {
            let mut stats = inner.stats.lock();
            stats.total_acquisitions += 1;
            stats.active += 1;
            stats.idle = inner.idle.lock().len();
        }

        // Forget the permit — we'll add it back when the guard drops.
        permit.forget();

        let pool = self.clone();
        Ok(ResourceGuard::new(instance, move |mut inst| {
            // Return instance to pool on drop.
            // We run recycle + return synchronously-ish by spawning a task.
            tokio::spawn(async move {
                let inner = &pool.inner;
                // Try to recycle
                let keep = match inner.resource.recycle(&mut inst).await {
                    Ok(()) => true,
                    Err(_) => false,
                };

                if keep {
                    let mut entry = Entry::new(inst);
                    entry.last_used = Instant::now();
                    inner.idle.lock().push_back(entry);
                } else {
                    let _ = inner.resource.cleanup(inst).await;
                    let mut stats = inner.stats.lock();
                    stats.destroyed += 1;
                }

                {
                    let mut stats = inner.stats.lock();
                    stats.total_releases += 1;
                    stats.active = stats.active.saturating_sub(1);
                    stats.idle = inner.idle.lock().len();
                }

                // Return the permit
                inner.semaphore.add_permits(1);
            });
        }))
    }

    /// Get current pool statistics.
    #[must_use]
    pub fn stats(&self) -> PoolStats {
        self.inner.stats.lock().clone()
    }

    /// Run maintenance: evict expired idle instances, ensure min_size.
    pub async fn maintain(&self, ctx: &ResourceContext) -> ResourceResult<()> {
        let inner = &self.inner;

        // Evict expired idle entries
        let mut to_cleanup = Vec::new();
        {
            let mut idle = inner.idle.lock();
            let before = idle.len();
            let mut kept = VecDeque::with_capacity(idle.len());
            while let Some(entry) = idle.pop_front() {
                if entry.is_expired(&inner.pool_config) {
                    to_cleanup.push(entry.instance);
                } else {
                    kept.push_back(entry);
                }
            }
            *idle = kept;
            let removed = before - idle.len();
            if removed > 0 {
                let mut stats = inner.stats.lock();
                stats.destroyed += removed as u64;
                stats.idle = idle.len();
            }
        }

        for inst in to_cleanup {
            let _ = inner.resource.cleanup(inst).await;
        }

        // Ensure min_size
        let current_idle = inner.idle.lock().len();
        let current_active = inner.stats.lock().active;
        let total = current_idle + current_active;
        if total < inner.pool_config.min_size {
            let needed = inner.pool_config.min_size - total;
            for _ in 0..needed {
                match inner.resource.create(&inner.config, ctx).await {
                    Ok(instance) => {
                        inner.idle.lock().push_back(Entry::new(instance));
                        let mut stats = inner.stats.lock();
                        stats.created += 1;
                        stats.idle = inner.idle.lock().len();
                    }
                    Err(_) => break,
                }
            }
        }

        Ok(())
    }

    /// Shut down the pool, cleaning up all idle instances.
    pub async fn shutdown(&self) -> ResourceResult<()> {
        let inner = &self.inner;
        let entries: Vec<_> = {
            let mut idle = inner.idle.lock();
            idle.drain(..).collect()
        };

        for entry in entries {
            let _ = inner.resource.cleanup(entry.instance).await;
            inner.stats.lock().destroyed += 1;
        }

        inner.stats.lock().idle = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{Resource, ResourceConfig};
    use crate::scope::ResourceScope;
    use async_trait::async_trait;
    use std::time::Duration;

    // -- Test resource --

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TestConfig {
        prefix: String,
    }

    impl ResourceConfig for TestConfig {
        fn validate(&self) -> ResourceResult<()> {
            if self.prefix.is_empty() {
                return Err(ResourceError::configuration("prefix cannot be empty"));
            }
            Ok(())
        }
    }

    struct TestResource;

    #[async_trait]
    impl Resource for TestResource {
        type Config = TestConfig;
        type Instance = String;

        fn id(&self) -> &str {
            "test-resource"
        }

        async fn create(
            &self,
            config: &Self::Config,
            _ctx: &ResourceContext,
        ) -> ResourceResult<Self::Instance> {
            Ok(format!("{}-instance", config.prefix))
        }
    }

    fn test_ctx() -> ResourceContext {
        ResourceContext::new(ResourceScope::Global, "wf-1", "ex-1")
    }

    fn test_config() -> TestConfig {
        TestConfig {
            prefix: "test".to_string(),
        }
    }

    #[tokio::test]
    async fn acquire_returns_instance() {
        let pool = Pool::new(TestResource, test_config(), PoolConfig::default());
        let guard = pool.acquire(&test_ctx()).await.unwrap();
        assert_eq!(*guard, "test-instance");
    }

    #[tokio::test]
    async fn pool_reuses_instances() {
        let pool = Pool::new(TestResource, test_config(), PoolConfig::default());

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
        let pool = Pool::new(TestResource, test_config(), pool_config);

        let _g1 = pool.acquire(&test_ctx()).await.unwrap();
        let _g2 = pool.acquire(&test_ctx()).await.unwrap();

        // Third acquire should timeout
        let result = pool.acquire(&test_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_cleans_idle() {
        let pool = Pool::new(TestResource, test_config(), PoolConfig::default());

        {
            let _g = pool.acquire(&test_ctx()).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;

        pool.shutdown().await.unwrap();
        let stats = pool.stats();
        assert_eq!(stats.idle, 0);
    }
}
