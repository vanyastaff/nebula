//! Pool runtime — manages a pool of N interchangeable resource instances.
//!
//! The acquire path: try idle queue -> check broken -> test_on_checkout -> prepare -> return handle.
//! If no idle instance: create new (respecting semaphore for max_size).
//! If semaphore full: wait with timeout.
//!
//! The release path (via [`ReleaseQueue`]): tainted? -> stale fingerprint? -> max_lifetime? ->
//! recycle() -> Keep/Drop.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::ctx::Ctx;
use crate::error::Error;
use crate::handle::ResourceHandle;
use crate::metrics::ResourceMetrics;
use crate::options::AcquireOptions;
use crate::release_queue::ReleaseQueue;
use crate::resource::Resource;
use crate::topology::pooled::config::Config;
use crate::topology::pooled::{InstanceMetrics, Pooled, RecycleDecision};
use crate::topology_tag::TopologyTag;

/// A single pooled instance with its metrics, config fingerprint, and semaphore permit.
///
/// The permit lives with the entry for its entire lifecycle — from creation
/// through idle queue and checkout cycles until final destruction.
struct PoolEntry<R: Resource> {
    runtime: R::Runtime,
    metrics: InstanceMetrics,
    fingerprint: u64,
    permit: OwnedSemaphorePermit,
}

/// Runtime state for a pool topology.
///
/// Manages an idle queue of instances, a semaphore for max-size enforcement,
/// and acquire/release logic with broken-check, recycle, and lifetime policies.
pub struct PoolRuntime<R: Resource> {
    idle: Arc<Mutex<VecDeque<PoolEntry<R>>>>,
    semaphore: Arc<Semaphore>,
    config: Config,
    current_fingerprint: Arc<AtomicU64>,
}

impl<R: Resource> PoolRuntime<R> {
    /// Creates a new pool runtime with the given config and initial fingerprint.
    pub fn new(config: Config, fingerprint: u64) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_size as usize));
        Self {
            idle: Arc::new(Mutex::new(VecDeque::new())),
            semaphore,
            config,
            current_fingerprint: Arc::new(AtomicU64::new(fingerprint)),
        }
    }

    /// Returns the current pool configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Updates the config fingerprint (e.g., after hot-reload).
    pub fn set_fingerprint(&self, fingerprint: u64) {
        self.current_fingerprint
            .store(fingerprint, Ordering::Release);
    }

    /// Returns the number of idle instances currently in the pool.
    pub async fn idle_count(&self) -> usize {
        self.idle.lock().await.len()
    }
}

impl<R> PoolRuntime<R>
where
    R: Pooled + Clone + Send + Sync + 'static,
    R::Runtime: Into<R::Lease>,
    R::Lease: Into<R::Runtime>,
    R::Runtime: Clone,
{
    /// Acquires an instance from the pool.
    ///
    /// 1. Try to pop from the idle queue.
    /// 2. Check `is_broken` — if broken, destroy and try next.
    /// 3. If `test_on_checkout` — run `check()`.
    /// 4. Run `prepare(ctx)`.
    /// 5. Return a guarded handle whose drop submits release to the queue.
    /// 6. If no idle: create a new instance (respecting semaphore for max_size).
    /// 7. If semaphore is full: wait with `create_timeout`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Backpressure`] if the pool is full and the timeout expires.
    /// - [`ErrorKind::Transient`] if creation or preparation fails.
    // Reason: `options` is a separate concern from the existing resource/config/ctx
    // tuple and will be reduced when we bundle resource+config into a single arg.
    #[allow(clippy::too_many_arguments)]
    pub async fn acquire(
        &self,
        resource: &R,
        resource_config: &R::Config,
        credential: &R::Credential,
        ctx: &dyn Ctx,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        options: &AcquireOptions,
        metrics: Arc<ResourceMetrics>,
    ) -> Result<ResourceHandle<R>, Error> {
        // Try to get an idle instance first.
        if let Some(handle) = self
            .try_acquire_idle(
                resource,
                ctx,
                release_queue,
                generation,
                Arc::clone(&metrics),
            )
            .await?
        {
            return Ok(handle);
        }

        // No idle instance available — acquire a semaphore permit for a new slot.
        let permit = self.acquire_semaphore_permit(options).await?;

        let entry = match self
            .create_entry(resource, resource_config, credential, ctx, permit)
            .await
        {
            Ok(e) => e,
            Err(e) => return Err(e),
        };

        // Prepare the new instance.
        if let Err(e) = resource.prepare(&entry.runtime, ctx).await {
            let _ = resource.destroy(entry.runtime).await;
            // permit is dropped with entry
            return Err(e.into());
        }

        let lease: R::Lease = entry.runtime.clone().into();
        Ok(self.build_guarded_handle(
            lease,
            entry,
            resource.clone(),
            release_queue.clone(),
            generation,
            metrics,
        ))
    }

    /// Attempts to pop and validate an idle instance.
    ///
    /// Returns `Ok(Some(handle))` if a valid instance was found,
    /// `Ok(None)` if the idle queue is empty, or `Err` on failure.
    async fn try_acquire_idle(
        &self,
        resource: &R,
        ctx: &dyn Ctx,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        metrics: Arc<ResourceMetrics>,
    ) -> Result<Option<ResourceHandle<R>>, Error> {
        loop {
            let entry = {
                let mut idle = self.idle.lock().await;
                if self.config.strategy == crate::topology::pooled::config::PoolStrategy::Lifo {
                    idle.pop_back()
                } else {
                    idle.pop_front()
                }
            };

            let Some(mut entry) = entry else {
                return Ok(None);
            };

            // Stale fingerprint — destroy silently (permit drops with entry).
            let current_fp = self.current_fingerprint.load(Ordering::Acquire);
            if entry.fingerprint != current_fp {
                let _ = resource.destroy(entry.runtime).await;
                continue;
            }

            // Max lifetime check.
            if self
                .config
                .max_lifetime
                .is_some_and(|max| entry.metrics.created_at.elapsed() > max)
            {
                let _ = resource.destroy(entry.runtime).await;
                continue;
            }

            // Broken check (sync, O(1)).
            if resource.is_broken(&entry.runtime).is_broken() {
                let _ = resource.destroy(entry.runtime).await;
                continue;
            }

            // Optional health check on checkout.
            if self.config.test_on_checkout && resource.check(&entry.runtime).await.is_err() {
                let _ = resource.destroy(entry.runtime).await;
                continue;
            }

            // Prepare for this execution context.
            if let Err(e) = resource.prepare(&entry.runtime, ctx).await {
                let _ = resource.destroy(entry.runtime).await;
                return Err(e.into());
            }

            entry.metrics.checkout_count += 1;

            let lease: R::Lease = entry.runtime.clone().into();
            return Ok(Some(self.build_guarded_handle(
                lease,
                entry,
                resource.clone(),
                release_queue.clone(),
                generation,
                metrics,
            )));
        }
    }

    /// Waits for a semaphore permit with the configured timeout.
    ///
    /// If the caller provided a deadline via [`AcquireOptions`], the remaining
    /// time is used instead of the pool's `create_timeout`.
    async fn acquire_semaphore_permit(
        &self,
        options: &AcquireOptions,
    ) -> Result<OwnedSemaphorePermit, Error> {
        let timeout = options.remaining().unwrap_or(self.config.create_timeout);
        match tokio::time::timeout(timeout, self.semaphore.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_closed)) => Err(Error::permanent("pool semaphore closed")),
            Err(_timeout) => Err(Error::backpressure(
                "pool full: timed out waiting for available slot",
            )),
        }
    }

    /// Creates a new pool entry via `resource.create()`.
    async fn create_entry(
        &self,
        resource: &R,
        config: &R::Config,
        credential: &R::Credential,
        ctx: &dyn Ctx,
        permit: OwnedSemaphorePermit,
    ) -> Result<PoolEntry<R>, Error> {
        let runtime = match tokio::time::timeout(
            self.config.create_timeout,
            resource.create(config, credential, ctx),
        )
        .await
        {
            Ok(Ok(rt)) => rt,
            Ok(Err(e)) => return Err(e.into()),
            Err(_timeout) => {
                return Err(Error::transient("pool: create timed out"));
            }
        };

        Ok(PoolEntry {
            runtime,
            metrics: InstanceMetrics {
                error_count: 0,
                checkout_count: 1,
                created_at: Instant::now(),
            },
            fingerprint: self.current_fingerprint.load(Ordering::Acquire),
            permit,
        })
    }

    /// Builds a guarded handle with an on-release callback that submits
    /// async recycle work to the [`ReleaseQueue`].
    fn build_guarded_handle(
        &self,
        lease: R::Lease,
        entry: PoolEntry<R>,
        resource: R,
        release_queue: Arc<ReleaseQueue>,
        generation: u64,
        metrics: Arc<ResourceMetrics>,
    ) -> ResourceHandle<R> {
        let idle = self.idle.clone();
        let current_fp_ref = self.current_fingerprint.clone();
        let max_lifetime = self.config.max_lifetime;

        ResourceHandle::guarded(
            lease,
            R::key(),
            TopologyTag::Pool,
            generation,
            move |returned_lease: R::Lease, tainted| {
                metrics.record_release();

                let runtime: R::Runtime = returned_lease.into();
                let entry = PoolEntry {
                    runtime,
                    metrics: entry.metrics.clone(),
                    fingerprint: entry.fingerprint,
                    permit: entry.permit,
                };

                // Load fingerprint at release time (not checkout time) to detect
                // config changes that happened while the handle was checked out.
                let current_fp = current_fp_ref.load(Ordering::Acquire);
                release_queue.submit(move || {
                    Box::pin(release_entry(
                        resource,
                        entry,
                        tainted,
                        current_fp,
                        max_lifetime,
                        idle,
                    ))
                });
            },
        )
    }
}

/// Async release logic extracted to avoid excessive nesting inside closures.
///
/// Decides whether to recycle or destroy a returned pool entry. The semaphore
/// permit lives inside `entry` and is dropped automatically when the entry is
/// destroyed, freeing a slot for new instances.
async fn release_entry<R>(
    resource: R,
    entry: PoolEntry<R>,
    tainted: bool,
    current_fp: u64,
    max_lifetime: Option<std::time::Duration>,
    idle: Arc<Mutex<VecDeque<PoolEntry<R>>>>,
) where
    R: Pooled + Send + Sync + 'static,
{
    // Tainted — destroy immediately.
    if tainted {
        let _ = resource.destroy(entry.runtime).await;
        return;
    }

    // Stale fingerprint — config changed since checkout.
    if entry.fingerprint != current_fp {
        let _ = resource.destroy(entry.runtime).await;
        return;
    }

    // Max lifetime exceeded.
    if max_lifetime.is_some_and(|max| entry.metrics.created_at.elapsed() > max) {
        let _ = resource.destroy(entry.runtime).await;
        return;
    }

    // Broken check (sync).
    if resource.is_broken(&entry.runtime).is_broken() {
        let _ = resource.destroy(entry.runtime).await;
        return;
    }

    // Async recycle check.
    match resource.recycle(&entry.runtime, &entry.metrics).await {
        Ok(RecycleDecision::Keep) => {
            idle.lock().await.push_back(entry);
        }
        Ok(RecycleDecision::Drop) | Err(_) => {
            let _ = resource.destroy(entry.runtime).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::BasicCtx;
    use crate::options::AcquireOptions;
    use crate::resource::{ResourceConfig, ResourceMetadata};
    use crate::topology::pooled::BrokenCheck;
    use nebula_core::{ExecutionId, ResourceKey, resource_key};
    use std::sync::atomic::AtomicBool;

    // -- Mock resource implementing Pooled --

    #[derive(Clone)]
    struct MockPool {
        fail_create: Arc<AtomicBool>,
        fail_check: Arc<AtomicBool>,
        break_on_return: Arc<AtomicBool>,
    }

    impl MockPool {
        fn new() -> Self {
            Self {
                fail_create: Arc::new(AtomicBool::new(false)),
                fail_check: Arc::new(AtomicBool::new(false)),
                break_on_return: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    #[derive(Debug, Clone)]
    struct MockError(String);

    impl std::fmt::Display for MockError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for MockError {}

    impl From<MockError> for Error {
        fn from(e: MockError) -> Self {
            Error::transient(e.0)
        }
    }

    #[derive(Clone)]
    struct PoolTestConfig;

    impl ResourceConfig for PoolTestConfig {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
    }

    impl Resource for MockPool {
        type Config = PoolTestConfig;
        type Runtime = u32;
        type Lease = u32;
        type Error = MockError;
        type Credential = ();

        fn key() -> ResourceKey {
            resource_key!("mock-pool")
        }

        fn create(
            &self,
            _config: &PoolTestConfig,
            _credential: &(),
            _ctx: &dyn Ctx,
        ) -> impl std::future::Future<Output = Result<u32, MockError>> + Send {
            let fail = self.fail_create.load(Ordering::Relaxed);
            async move {
                if fail {
                    Err(MockError("create failed".into()))
                } else {
                    Ok(1)
                }
            }
        }

        fn check(
            &self,
            _runtime: &u32,
        ) -> impl std::future::Future<Output = Result<(), MockError>> + Send {
            let fail = self.fail_check.load(Ordering::Relaxed);
            async move {
                if fail {
                    Err(MockError("check failed".into()))
                } else {
                    Ok(())
                }
            }
        }

        fn destroy(
            &self,
            _runtime: u32,
        ) -> impl std::future::Future<Output = Result<(), MockError>> + Send {
            async { Ok(()) }
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Pooled for MockPool {
        fn is_broken(&self, _runtime: &u32) -> BrokenCheck {
            if self.break_on_return.load(Ordering::Relaxed) {
                BrokenCheck::Broken("forced break".into())
            } else {
                BrokenCheck::Healthy
            }
        }
    }

    fn test_ctx() -> BasicCtx {
        BasicCtx::new(ExecutionId::new())
    }

    #[tokio::test]
    async fn acquire_creates_new_instance_when_idle_empty() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 2,
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let ctx = test_ctx();

        let handle = pool
            .acquire(
                &resource,
                &PoolTestConfig,
                &(),
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                Arc::new(ResourceMetrics::new()),
            )
            .await;
        assert!(handle.is_ok());
        let handle = handle.unwrap();
        assert_eq!(*handle, 1);
        assert_eq!(handle.topology_tag(), TopologyTag::Pool);

        drop(handle);
        // Give release queue time to process.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Instance should be recycled back to idle.
        assert_eq!(pool.idle_count().await, 1);

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }

    #[tokio::test]
    async fn acquire_reuses_idle_instance() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 2,
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let ctx = test_ctx();

        // Acquire and release to populate idle queue.
        let handle = pool
            .acquire(
                &resource,
                &PoolTestConfig,
                &(),
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                Arc::new(ResourceMetrics::new()),
            )
            .await
            .unwrap();
        drop(handle);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(pool.idle_count().await, 1);

        // Second acquire should reuse the idle instance.
        let handle2 = pool
            .acquire(
                &resource,
                &PoolTestConfig,
                &(),
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                Arc::new(ResourceMetrics::new()),
            )
            .await;
        assert!(handle2.is_ok());

        drop(handle2);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }

    #[tokio::test]
    async fn broken_instance_is_destroyed_on_acquire() {
        let resource = MockPool::new();
        resource.break_on_return.store(false, Ordering::Relaxed);

        let config = Config {
            max_size: 2,
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let ctx = test_ctx();

        // Acquire and release to populate idle queue.
        let handle = pool
            .acquire(
                &resource,
                &PoolTestConfig,
                &(),
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                Arc::new(ResourceMetrics::new()),
            )
            .await
            .unwrap();
        drop(handle);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(pool.idle_count().await, 1);

        // Now mark broken — next acquire should destroy the idle instance
        // and create a fresh one.
        resource.break_on_return.store(true, Ordering::Relaxed);

        let handle2 = pool
            .acquire(
                &resource,
                &PoolTestConfig,
                &(),
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                Arc::new(ResourceMetrics::new()),
            )
            .await;
        assert!(handle2.is_ok());

        drop(handle2);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // The broken instance should have been destroyed, not returned to idle.
        assert_eq!(pool.idle_count().await, 0);

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }

    #[tokio::test]
    async fn tainted_handle_destroys_on_release() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 2,
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let ctx = test_ctx();

        let mut handle = pool
            .acquire(
                &resource,
                &PoolTestConfig,
                &(),
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                Arc::new(ResourceMetrics::new()),
            )
            .await
            .unwrap();
        handle.taint();
        drop(handle);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Tainted — should not be in idle.
        assert_eq!(pool.idle_count().await, 0);

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }

    #[tokio::test]
    async fn create_failure_returns_error() {
        let resource = MockPool::new();
        resource.fail_create.store(true, Ordering::Relaxed);

        let config = Config {
            max_size: 2,
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let ctx = test_ctx();

        let result = pool
            .acquire(
                &resource,
                &PoolTestConfig,
                &(),
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                Arc::new(ResourceMetrics::new()),
            )
            .await;
        assert!(result.is_err());

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }

    #[tokio::test]
    async fn semaphore_timeout_sanity() {
        // Minimal test: semaphore with 0 permits should timeout.
        let sem = Arc::new(Semaphore::new(0));
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), sem.acquire_owned()).await;
        assert!(result.is_err(), "should have timed out");
    }

    /// Verifies that the pool correctly returns backpressure error when full.
    ///
    /// Uses a pre-acquired semaphore permit to simulate full pool, avoiding
    /// interaction between tokio timer and spawned ReleaseQueue workers on
    /// the single-thread test runtime.
    #[tokio::test]
    async fn pool_full_returns_backpressure() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 1,
            create_timeout: std::time::Duration::from_millis(200),
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let ctx = test_ctx();

        // Steal the only permit — pool is full without involving acquire().
        let _permit = pool.semaphore.clone().acquire_owned().await.unwrap();

        let result = pool
            .acquire(
                &resource,
                &PoolTestConfig,
                &(),
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                Arc::new(ResourceMetrics::new()),
            )
            .await;
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("expected backpressure error"),
        };
        assert_eq!(*err.kind(), crate::error::ErrorKind::Backpressure);

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }
}
