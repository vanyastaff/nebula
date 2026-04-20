//! Pool runtime — manages a pool of N interchangeable resource instances.
//!
//! The acquire path: try idle queue -> check broken -> test_on_checkout -> prepare -> return
//! handle. If no idle instance: create new (respecting semaphore for max_size).
//! If semaphore full: wait with timeout.
//!
//! The release path (via [`ReleaseQueue`]): tainted? -> stale fingerprint? -> max_lifetime? ->
//! recycle() -> Keep/Drop.

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::{
    ctx::Ctx,
    error::Error,
    handle::ResourceHandle,
    metrics::ResourceOpsMetrics,
    options::AcquireOptions,
    release_queue::ReleaseQueue,
    resource::Resource,
    topology::pooled::{InstanceMetrics, Pooled, RecycleDecision, config::Config},
    topology_tag::TopologyTag,
};

/// A single pooled instance with its metrics and config fingerprint.
///
/// The semaphore permit no longer lives here — it is held in
/// `HandleInner::Guarded` so that it is returned even if the release
/// callback panics.
struct PoolEntry<R: Resource> {
    runtime: R::Runtime,
    metrics: InstanceMetrics,
    fingerprint: u64,
    /// When this entry was last returned to the idle queue.
    /// `None` for freshly created entries that have never been idle.
    returned_at: Option<Instant>,
}

/// Result of attempting to pop an idle instance from the pool.
enum IdleResult<R: Resource> {
    /// A valid idle instance was found — wrapped in a handle.
    Found(ResourceHandle<R>),
    /// No usable idle instance — the permit is returned so the caller
    /// can create a new instance.
    Empty(OwnedSemaphorePermit),
}

/// A point-in-time snapshot of pool utilization.
///
/// Returned by [`PoolRuntime::stats`] and [`Manager::pool_stats`](crate::Manager::pool_stats).
///
/// # Note
///
/// `idle` and `in_use` are sampled separately and may not add up to `capacity`
/// precisely due to concurrent activity between reads.
#[derive(Debug, Clone, Copy)]
pub struct PoolStats {
    /// Number of instances currently sitting idle in the pool.
    pub idle: usize,
    /// Maximum number of concurrently active leases (`max_size` from config).
    pub capacity: u32,
    /// Number of permits currently available in the semaphore.
    ///
    /// A value of `capacity - in_use` in a quiescent pool.
    pub available_permits: usize,
    /// Number of instances currently checked out by callers.
    pub in_use: usize,
}

/// Runtime state for a pool topology.
///
/// Manages an idle queue of instances, a semaphore for max-size enforcement,
/// and acquire/release logic with broken-check, recycle, and lifetime policies.
pub struct PoolRuntime<R: Resource> {
    idle: Arc<Mutex<VecDeque<PoolEntry<R>>>>,
    semaphore: Arc<Semaphore>,
    /// Bounds concurrent invocations of `create_entry` (#390).
    ///
    /// The checkout semaphore gates active leases; this one gates
    /// *creation* so a burst of concurrent acquires cannot fan out into
    /// `max_size` parallel `Resource::create` calls against a fragile
    /// backend.
    create_semaphore: Arc<Semaphore>,
    config: Config,
    current_fingerprint: Arc<AtomicU64>,
}

impl<R: Resource> PoolRuntime<R> {
    /// Creates a new pool runtime with the given config and initial fingerprint.
    /// Creates a new pool runtime with the given configuration.
    ///
    /// The `fingerprint` is a config-change detection token. When
    /// [`Manager::reload_config`](crate::Manager::reload_config) is called,
    /// idle instances whose fingerprint differs from the current one are
    /// evicted. Use `0` as the initial value; the manager updates it
    /// automatically on reload. Implement
    /// [`ResourceConfig::fingerprint()`](crate::ResourceConfig::fingerprint)
    /// on your config type to enable change detection.
    pub fn new(config: Config, fingerprint: u64) -> Self {
        // #390: fail loudly at construction rather than deadlock on first
        // acquire. `Manager::register_pooled*` surfaces the same check as
        // a typed `Error::permanent`, but this guard also protects direct
        // callers of the public `PoolRuntime::new` (e.g. the README and
        // doctests). Invariants that must hold for the pool to function
        // at all are asserted here rather than silently clamped.
        assert!(
            config.max_size > 0,
            "PoolRuntime: config.max_size must be > 0 (got 0 — would deadlock \
             the checkout semaphore on first acquire)",
        );
        assert!(
            config.min_size <= config.max_size,
            "PoolRuntime: config.min_size ({}) must be <= max_size ({})",
            config.min_size,
            config.max_size,
        );

        let semaphore = Arc::new(Semaphore::new(config.max_size as usize));
        // #390: cap concurrent instance creation. `max(1)` protects us
        // from a pathological `max_concurrent_creates = 0` config that
        // would otherwise deadlock the pool on first acquire.
        let create_semaphore = Arc::new(Semaphore::new(
            (config.max_concurrent_creates as usize).max(1),
        ));
        Self {
            idle: Arc::new(Mutex::new(VecDeque::new())),
            semaphore,
            create_semaphore,
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

    /// Returns a snapshot of current pool utilization.
    ///
    /// `idle` is sampled under the idle queue lock; `available_permits` is
    /// read atomically from the semaphore. Both reads are best-effort and
    /// may be slightly inconsistent in high-concurrency scenarios.
    pub async fn stats(&self) -> PoolStats {
        let idle = self.idle.lock().await.len();
        let available_permits = self.semaphore.available_permits();
        let in_use = (self.config.max_size as usize).saturating_sub(available_permits);
        PoolStats {
            idle,
            capacity: self.config.max_size,
            available_permits,
            in_use,
        }
    }
}

impl<R> PoolRuntime<R>
where
    R: Pooled + Clone + Send + Sync + 'static,
    R::Runtime: Into<R::Lease>,
    R::Lease: Into<R::Runtime>,
    R::Runtime: Clone,
{
    /// Runs one maintenance cycle: evicts idle-timeout, max-lifetime, and
    /// stale-fingerprint entries from the idle queue.
    ///
    /// Returns the number of entries evicted. Each evicted entry is destroyed
    /// via [`Resource::destroy`].
    pub async fn run_maintenance(&self, resource: &R) -> usize {
        let to_destroy = {
            let mut idle = self.idle.lock().await;
            let current_fp = self.current_fingerprint.load(Ordering::Acquire);
            let now = Instant::now();

            let mut keep = VecDeque::with_capacity(idle.len());
            let mut evict = Vec::new();

            for entry in idle.drain(..) {
                if Self::should_evict(&entry, &self.config, current_fp, now) {
                    evict.push(entry.runtime);
                } else {
                    keep.push_back(entry);
                }
            }
            *idle = keep;
            evict
        };

        let evicted = to_destroy.len();
        for runtime in to_destroy {
            let _ = resource.destroy(runtime).await;
        }

        if evicted > 0 {
            tracing::debug!(evicted, "pool maintenance: evicted idle/expired entries");
        }
        evicted
    }

    /// Checks whether a pool entry should be evicted during maintenance.
    fn should_evict(entry: &PoolEntry<R>, config: &Config, current_fp: u64, now: Instant) -> bool {
        // Stale fingerprint.
        if entry.fingerprint != current_fp {
            return true;
        }
        // Max lifetime exceeded.
        if config
            .max_lifetime
            .is_some_and(|max| now.duration_since(entry.metrics.created_at) > max)
        {
            return true;
        }
        // Idle timeout exceeded.
        if let (Some(idle_timeout), Some(returned_at)) = (config.idle_timeout, entry.returned_at) {
            return now.duration_since(returned_at) > idle_timeout;
        }
        false
    }

    /// Acquires an instance from the pool.
    ///
    /// 1. Acquire a semaphore permit (waits with timeout if pool is full).
    /// 2. Try to pop from the idle queue.
    /// 3. Check `is_broken` — if broken, destroy and try next.
    /// 4. If `test_on_checkout` — run `check()`.
    /// 5. Run `prepare(ctx)`.
    /// 6. Return a guarded handle whose drop submits release to the queue.
    /// 7. If no idle: create a new instance with the acquired permit.
    ///
    /// The semaphore permit lives in the handle (not the release callback),
    /// so it is returned even if the callback panics.
    ///
    /// # Errors
    ///
    /// - Backpressure if the pool is full and the timeout expires.
    /// - Transient if creation or preparation fails.
    // Reason: `options` is a separate concern from the existing resource/config/ctx
    // tuple and will be reduced when we bundle resource+config into a single arg.
    #[expect(
        clippy::too_many_arguments,
        reason = "`options` is a separate concern from resource/config/ctx; will reduce when bundled"
    )]
    pub async fn acquire(
        &self,
        resource: &R,
        resource_config: &R::Config,
        auth: &R::Auth,
        ctx: &dyn Ctx,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        options: &AcquireOptions,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Result<ResourceHandle<R>, Error> {
        // Acquire a semaphore permit first — this is the concurrency gate.
        // If idle instances exist their permits were already returned on
        // handle drop, so a permit is immediately available.
        let permit = self.acquire_semaphore_permit(options).await?;

        // Try to get an idle instance.
        let permit = match self
            .try_acquire_idle(
                resource,
                ctx,
                release_queue,
                generation,
                permit,
                metrics.clone(),
            )
            .await?
        {
            IdleResult::Found(handle) => return Ok(handle),
            IdleResult::Empty(permit) => permit,
        };

        // No idle instance available — create a new one with our permit.
        let entry = match self
            .create_entry(resource, resource_config, auth, ctx, false)
            .await
        {
            Ok(e) => e,
            Err(e) => return Err(e),
        };

        // Cancel-safety: if the future is dropped between here and
        // `build_guarded_handle`, the guard ensures we log the leak
        // and drop the runtime (triggering its native `Drop`).
        let mut guard = CreateGuard::new(entry);

        // Prepare the new instance.
        if let Err(e) = resource.prepare(guard.runtime(), ctx).await {
            let entry = guard.defuse();
            let _ = resource.destroy(entry.runtime).await;
            // permit drops here, returning the slot.
            return Err(e.into());
        }

        let entry = guard.defuse();
        let lease: R::Lease = entry.runtime.clone().into();
        Ok(self.build_guarded_handle(
            lease,
            entry,
            permit,
            resource.clone(),
            release_queue.clone(),
            generation,
            metrics,
        ))
    }

    /// Attempts to pop and validate an idle instance.
    ///
    /// On success returns the handle. On empty idle queue (or all entries
    /// destroyed) returns the permit back so the caller can use it for a
    /// fresh creation. On hard error the permit is dropped (returning the
    /// slot to the semaphore).
    async fn try_acquire_idle(
        &self,
        resource: &R,
        ctx: &dyn Ctx,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        permit: OwnedSemaphorePermit,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Result<IdleResult<R>, Error> {
        loop {
            let entry = {
                let mut idle = self.idle.lock().await;
                if self.config.strategy == crate::topology::pooled::config::PoolStrategy::Lifo {
                    idle.pop_back()
                } else {
                    idle.pop_front()
                }
            };

            let Some(entry) = entry else {
                return Ok(IdleResult::Empty(permit));
            };

            // Cancel-safety: guard the popped entry through all async
            // validation steps. If cancelled mid-check, we log + drop
            // rather than silently leaking the instance.
            let mut guard = CreateGuard::new(entry);

            // Stale fingerprint — destroy silently.
            let current_fp = self.current_fingerprint.load(Ordering::Acquire);
            if guard.entry().fingerprint != current_fp {
                let entry = guard.defuse();
                let _ = resource.destroy(entry.runtime).await;
                continue;
            }

            // Max lifetime check.
            if self
                .config
                .max_lifetime
                .is_some_and(|max| guard.entry().metrics.created_at.elapsed() > max)
            {
                let entry = guard.defuse();
                let _ = resource.destroy(entry.runtime).await;
                continue;
            }

            // Broken check (sync, O(1)).
            if resource.is_broken(guard.runtime()).is_broken() {
                let entry = guard.defuse();
                let _ = resource.destroy(entry.runtime).await;
                continue;
            }

            // Optional health check on checkout.
            if self.config.test_on_checkout && resource.check(guard.runtime()).await.is_err() {
                let entry = guard.defuse();
                let _ = resource.destroy(entry.runtime).await;
                continue;
            }

            // Prepare for this execution context.
            if let Err(e) = resource.prepare(guard.runtime(), ctx).await {
                let entry = guard.defuse();
                let _ = resource.destroy(entry.runtime).await;
                return Err(e.into());
            }

            let mut entry = guard.defuse();
            entry.metrics.checkout_count += 1;

            let lease: R::Lease = entry.runtime.clone().into();
            return Ok(IdleResult::Found(self.build_guarded_handle(
                lease,
                entry,
                permit,
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
    ///
    /// All creation goes through this funnel and is gated on
    /// `create_semaphore` (#390) so a burst of acquires cannot stampede
    /// a fragile backend with `max_size` parallel connects. The permit
    /// is released as soon as `Resource::create` returns.
    ///
    /// The whole path — permit wait + `resource.create` — shares a
    /// single `create_timeout` budget. Both the create semaphore wait
    /// and the actual create are bounded by the remaining budget so a
    /// slow-creating backend cannot stall callers forever (also raised
    /// in PR #399 review: the create-semaphore acquire used to be
    /// unbounded).
    ///
    /// When `non_blocking` is `true` (the `try_acquire` path), the
    /// create-semaphore wait is replaced with a `try_acquire_owned`
    /// that returns `Backpressure` immediately instead of queueing,
    /// preserving the non-blocking contract of `try_acquire`.
    async fn create_entry(
        &self,
        resource: &R,
        config: &R::Config,
        auth: &R::Auth,
        ctx: &dyn Ctx,
        non_blocking: bool,
    ) -> Result<PoolEntry<R>, Error> {
        let deadline = Instant::now() + self.config.create_timeout;

        let _create_permit = if non_blocking {
            match self.create_semaphore.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(tokio::sync::TryAcquireError::NoPermits) => {
                    return Err(Error::backpressure(format!(
                        "{}: create-semaphore full — all {} concurrent creates \
                         in use (non-blocking acquire, #390)",
                        R::key(),
                        self.config.max_concurrent_creates,
                    )));
                },
                Err(tokio::sync::TryAcquireError::Closed) => {
                    return Err(Error::permanent("pool: create semaphore closed"));
                },
            }
        } else {
            match tokio::time::timeout_at(
                deadline.into(),
                self.create_semaphore.clone().acquire_owned(),
            )
            .await
            {
                Ok(Ok(permit)) => permit,
                Ok(Err(_)) => {
                    return Err(Error::permanent("pool: create semaphore closed"));
                },
                Err(_) => {
                    return Err(Error::backpressure(
                        "pool: create timed out waiting for create-semaphore permit",
                    ));
                },
            }
        };

        // Use `timeout_at` with the same absolute deadline so the budget
        // is shared: a long permit wait shortens the time available to
        // `resource.create`.
        let runtime = match tokio::time::timeout_at(
            deadline.into(),
            resource.create(config, auth, ctx),
        )
        .await
        {
            Ok(Ok(rt)) => rt,
            Ok(Err(e)) => return Err(e.into()),
            Err(_timeout) => {
                return Err(Error::transient("pool: create timed out"));
            },
        };

        Ok(PoolEntry {
            runtime,
            metrics: InstanceMetrics {
                error_count: 0,
                checkout_count: 1,
                created_at: Instant::now(),
            },
            fingerprint: self.current_fingerprint.load(Ordering::Acquire),
            returned_at: None,
        })
    }

    /// Builds a guarded handle with an on-release callback that submits
    /// async recycle work to the [`ReleaseQueue`].
    ///
    /// The semaphore permit is stored directly in the handle, not inside
    /// the callback closure. This ensures the permit is returned even if
    /// the callback panics.
    // Reason: `permit` must be a separate argument — it cannot live in
    // `PoolEntry` because it needs to be stored in the handle, not the
    // callback closure. Bundling into a struct would add complexity for
    // a single call site.
    #[expect(
        clippy::too_many_arguments,
        reason = "`permit` must be separate — cannot live in `PoolEntry`; bundling adds complexity for one call site"
    )]
    fn build_guarded_handle(
        &self,
        lease: R::Lease,
        entry: PoolEntry<R>,
        permit: OwnedSemaphorePermit,
        resource: R,
        release_queue: Arc<ReleaseQueue>,
        generation: u64,
        metrics: Option<ResourceOpsMetrics>,
    ) -> ResourceHandle<R> {
        let idle = self.idle.clone();
        let current_fp_ref = self.current_fingerprint.clone();
        let max_lifetime = self.config.max_lifetime;

        ResourceHandle::guarded_with_permit(
            lease,
            R::key(),
            TopologyTag::Pool,
            generation,
            move |returned_lease: R::Lease, tainted| {
                if let Some(m) = &metrics {
                    m.record_release();
                }

                let runtime: R::Runtime = returned_lease.into();
                // Track tainted returns in error_count so `Pooled::recycle`
                // implementations can make informed keep-or-drop decisions
                // based on accumulated failure history.
                let mut instance_metrics = entry.metrics.clone();
                if tainted {
                    instance_metrics.error_count += 1;
                }
                let entry = PoolEntry {
                    runtime,
                    metrics: instance_metrics,
                    fingerprint: entry.fingerprint,
                    returned_at: None, // set by release_entry on idle push
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
            Some(permit),
        )
    }

    /// Attempts a non-blocking acquire: returns immediately with
    /// Backpressure if the pool is at capacity (all `max_size` slots hold active ResourceHandles).
    ///
    /// Unlike acquire, this method never queues — use it
    /// when you want to shed load rather than queue callers.
    ///
    /// # Errors
    ///
    /// - Backpressure if all `max_size` slots are occupied.
    /// - Transient if creation or preparation fails.
    // Reason: same as acquire — options is a distinct concern from resource/config/ctx.
    #[expect(
        clippy::too_many_arguments,
        reason = "options is a distinct concern from resource/config/ctx — same rationale as acquire"
    )]
    pub async fn try_acquire(
        &self,
        resource: &R,
        resource_config: &R::Config,
        auth: &R::Auth,
        ctx: &dyn Ctx,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        _options: &AcquireOptions,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Result<ResourceHandle<R>, Error> {
        // Non-blocking semaphore attempt — fail immediately if pool is full.
        let permit = match self.semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                return Err(Error::backpressure(format!(
                    "{}: pool full — all {} slots in use (non-blocking acquire returned immediately)",
                    R::key(),
                    self.config.max_size,
                )));
            },
        };

        // Try the idle queue first.
        let permit = match self
            .try_acquire_idle(
                resource,
                ctx,
                release_queue,
                generation,
                permit,
                metrics.clone(),
            )
            .await?
        {
            IdleResult::Found(handle) => return Ok(handle),
            IdleResult::Empty(permit) => permit,
        };

        // No idle instance — create a new one. The `true` flag keeps
        // the non-blocking contract: if the create semaphore is full,
        // we return Backpressure instead of waiting (PR #399 review).
        let entry = match self
            .create_entry(resource, resource_config, auth, ctx, true)
            .await
        {
            Ok(e) => e,
            Err(e) => return Err(e),
        };

        let mut guard = CreateGuard::new(entry);
        if let Err(e) = resource.prepare(guard.runtime(), ctx).await {
            let entry = guard.defuse();
            let _ = resource.destroy(entry.runtime).await;
            return Err(e.into());
        }

        let entry = guard.defuse();
        let lease: R::Lease = entry.runtime.clone().into();
        Ok(self.build_guarded_handle(
            lease,
            entry,
            permit,
            resource.clone(),
            release_queue.clone(),
            generation,
            metrics,
        ))
    }

    /// Warms up the pool by pre-creating up to `min_size` instances and
    /// depositing them into the idle queue.
    ///
    /// Should be called once after registration, before the pool receives
    /// production traffic, to eliminate cold-start latency on the first
    /// batch of requests.
    ///
    /// Returns the number of instances successfully created.
    ///
    /// # Strategy behaviour
    ///
    /// | Strategy | Behaviour |
    /// |----------|-----------|
    /// | `None` | No-op — returns 0 immediately. |
    /// | `Sequential` | Creates instances one at a time until `min_size` or failure. |
    /// | `Parallel` | Falls back to `Sequential`; true parallel warmup planned for a future release. |
    /// | `Staggered { interval }` | Creates one instance, sleeps `interval`, repeats. |
    ///
    /// Instances are pushed directly into the idle queue without consuming
    /// semaphore permits — permits are only held by active ResourceHandles.
    ///
    /// If a creation fails, warmup stops early and returns the count created
    /// so far (partial warmup is acceptable; on-demand creation handles the rest).
    pub async fn warmup(
        &self,
        resource: &R,
        resource_config: &R::Config,
        auth: &R::Auth,
        ctx: &dyn Ctx,
    ) -> usize {
        use crate::topology::pooled::config::WarmupStrategy;

        let target = self.config.min_size as usize;
        if target == 0 {
            return 0;
        }

        match self.config.warmup {
            WarmupStrategy::None => 0,
            // Parallel warmup requires `Arc<dyn Ctx>` to share across spawned
            // tasks. Until the Ctx API exposes an Arc variant, we fall back to
            // sequential to avoid an API-breaking change.
            WarmupStrategy::Sequential | WarmupStrategy::Parallel => {
                self.warmup_sequential(resource, resource_config, auth, ctx, target)
                    .await
            },
            WarmupStrategy::Staggered { interval } => {
                self.warmup_staggered(resource, resource_config, auth, ctx, target, interval)
                    .await
            },
        }
    }

    /// Sequential warmup helper: creates one instance at a time.
    async fn warmup_sequential(
        &self,
        resource: &R,
        resource_config: &R::Config,
        auth: &R::Auth,
        ctx: &dyn Ctx,
        target: usize,
    ) -> usize {
        let mut created = 0usize;
        for _ in 0..target {
            match self
                .create_entry(resource, resource_config, auth, ctx, false)
                .await
            {
                Ok(mut entry) => {
                    entry.returned_at = Some(Instant::now());
                    self.idle.lock().await.push_back(entry);
                    created += 1;
                    tracing::debug!(
                        key = %R::key(),
                        created,
                        target,
                        "pool warmup: instance created"
                    );
                },
                Err(e) => {
                    tracing::warn!(
                        key = %R::key(),
                        error = %e,
                        created,
                        target,
                        "pool warmup: create failed, stopping early"
                    );
                    break;
                },
            }
        }
        if created > 0 {
            tracing::info!(key = %R::key(), created, target, "pool warmup complete");
        }
        created
    }

    /// Staggered warmup helper: sleeps `interval` between each instance creation.
    async fn warmup_staggered(
        &self,
        resource: &R,
        resource_config: &R::Config,
        auth: &R::Auth,
        ctx: &dyn Ctx,
        target: usize,
        interval: Duration,
    ) -> usize {
        let mut created = 0usize;
        for i in 0..target {
            if i > 0 {
                tokio::time::sleep(interval).await;
            }
            match self
                .create_entry(resource, resource_config, auth, ctx, false)
                .await
            {
                Ok(mut entry) => {
                    entry.returned_at = Some(Instant::now());
                    self.idle.lock().await.push_back(entry);
                    created += 1;
                    tracing::debug!(
                        key = %R::key(),
                        created,
                        target,
                        "pool warmup (staggered): instance created"
                    );
                },
                Err(e) => {
                    tracing::warn!(
                        key = %R::key(),
                        error = %e,
                        created,
                        target,
                        "pool warmup (staggered): create failed, stopping early"
                    );
                    break;
                },
            }
        }
        if created > 0 {
            tracing::info!(
                key = %R::key(),
                created,
                target,
                "pool warmup (staggered) complete"
            );
        }
        created
    }
}

/// Async release logic extracted to avoid excessive nesting inside closures.
///
/// Decides whether to recycle or destroy a returned pool entry. The semaphore
/// permit is **not** held here — it was already returned when the handle
/// dropped (it lives in `HandleInner::Guarded`, not in the callback closure).
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
            let mut entry = entry;
            entry.returned_at = Some(Instant::now());
            idle.lock().await.push_back(entry);
        },
        Ok(RecycleDecision::Drop) | Err(_) => {
            let _ = resource.destroy(entry.runtime).await;
        },
    }
}

/// Cancel-safety guard for the create-then-prepare sequence.
///
/// Wraps a [`PoolEntry`] between creation and handle construction. If
/// the future is cancelled (e.g. via `tokio::select!` or timeout) after
/// `create()` succeeds but before the handle is built, `Drop` logs the
/// leak and drops the runtime — triggering its native `Drop` impl
/// (which closes TCP sockets, file handles, etc.).
///
/// Call [`defuse`](Self::defuse) to take ownership of the entry once
/// the handle is safely constructed.
struct CreateGuard<R: Resource> {
    entry: Option<PoolEntry<R>>,
}

impl<R: Resource> CreateGuard<R> {
    /// Creates a new guard wrapping the given pool entry.
    fn new(entry: PoolEntry<R>) -> Self {
        Self { entry: Some(entry) }
    }

    /// Returns a reference to the inner entry for inspection.
    fn entry(&self) -> &PoolEntry<R> {
        // Invariant: entry() is only called between new() and defuse(),
        // both are private with single call sites in the same function.
        self.entry
            .as_ref()
            .unwrap_or_else(|| unreachable!("CreateGuard accessed after defuse"))
    }

    /// Returns a reference to the runtime for use in `prepare()`.
    fn runtime(&self) -> &R::Runtime {
        &self.entry().runtime
    }

    /// Takes the entry out of the guard — it has been safely consumed.
    ///
    /// After this call, `Drop` is a no-op.
    fn defuse(&mut self) -> PoolEntry<R> {
        // Invariant: defuse() is called exactly once, right before
        // constructing the ResourceHandle.
        self.entry
            .take()
            .unwrap_or_else(|| unreachable!("CreateGuard defused twice"))
    }
}

impl<R: Resource> Drop for CreateGuard<R> {
    fn drop(&mut self) {
        if let Some(entry) = self.entry.take() {
            tracing::warn!(
                resource = %R::key(),
                "cancel-safety: pool entry dropped without async destroy \
                 (create succeeded but acquire future was cancelled)"
            );
            drop(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use nebula_core::{ExecutionId, ResourceKey, resource_key};

    use super::*;
    use crate::{
        ctx::BasicCtx,
        options::AcquireOptions,
        resource::{ResourceConfig, ResourceMetadata},
        topology::pooled::BrokenCheck,
    };

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

    nebula_schema::impl_empty_has_schema!(PoolTestConfig);

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
        type Auth = ();

        fn key() -> ResourceKey {
            resource_key!("mock-pool")
        }

        fn create(
            &self,
            _config: &PoolTestConfig,
            _auth: &(),
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

        async fn destroy(&self, _runtime: u32) -> Result<(), MockError> {
            Ok(())
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
                None,
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
                None,
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
                None,
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
                None,
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
                None,
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
                None,
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
                None,
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
                None,
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

    // -- Maintenance tests --

    /// Helper: pushes a fake idle entry with custom timestamps.
    async fn push_idle_entry(
        pool: &PoolRuntime<MockPool>,
        created_at: Instant,
        returned_at: Option<Instant>,
        fingerprint: u64,
    ) {
        pool.idle.lock().await.push_back(PoolEntry {
            runtime: 42,
            metrics: InstanceMetrics {
                error_count: 0,
                checkout_count: 1,
                created_at,
            },
            fingerprint,
            returned_at,
        });
    }

    #[tokio::test]
    async fn maintenance_evicts_idle_timeout_entries() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 5,
            idle_timeout: Some(std::time::Duration::from_millis(50)),
            max_lifetime: None,
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);

        // Push two entries: one returned long ago (should be evicted),
        // one returned just now (should be kept).
        let long_ago = Instant::now() - std::time::Duration::from_millis(200);
        push_idle_entry(&pool, Instant::now(), Some(long_ago), 1).await;
        push_idle_entry(&pool, Instant::now(), Some(Instant::now()), 1).await;

        assert_eq!(pool.idle_count().await, 2);
        let evicted = pool.run_maintenance(&resource).await;
        assert_eq!(evicted, 1);
        assert_eq!(pool.idle_count().await, 1);
    }

    #[tokio::test]
    async fn maintenance_evicts_max_lifetime_entries() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 5,
            idle_timeout: None,
            max_lifetime: Some(std::time::Duration::from_millis(50)),
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);

        // One old entry (should be evicted), one fresh (should be kept).
        let old_creation = Instant::now() - std::time::Duration::from_millis(200);
        push_idle_entry(&pool, old_creation, Some(Instant::now()), 1).await;
        push_idle_entry(&pool, Instant::now(), Some(Instant::now()), 1).await;

        let evicted = pool.run_maintenance(&resource).await;
        assert_eq!(evicted, 1);
        assert_eq!(pool.idle_count().await, 1);
    }

    #[tokio::test]
    async fn maintenance_evicts_stale_fingerprint_entries() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 5,
            idle_timeout: None,
            max_lifetime: None,
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);

        // Entry with old fingerprint.
        push_idle_entry(&pool, Instant::now(), Some(Instant::now()), 1).await;
        // Entry with current fingerprint.
        push_idle_entry(&pool, Instant::now(), Some(Instant::now()), 1).await;

        // Update fingerprint — first two entries now stale.
        pool.set_fingerprint(2);

        let evicted = pool.run_maintenance(&resource).await;
        assert_eq!(evicted, 2);
        assert_eq!(pool.idle_count().await, 0);
    }

    #[tokio::test]
    async fn maintenance_no_op_when_all_healthy() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 5,
            idle_timeout: Some(std::time::Duration::from_secs(300)),
            max_lifetime: Some(std::time::Duration::from_secs(1800)),
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);

        push_idle_entry(&pool, Instant::now(), Some(Instant::now()), 1).await;
        push_idle_entry(&pool, Instant::now(), Some(Instant::now()), 1).await;

        let evicted = pool.run_maintenance(&resource).await;
        assert_eq!(evicted, 0);
        assert_eq!(pool.idle_count().await, 2);
    }

    #[tokio::test]
    async fn maintenance_no_op_on_empty_pool() {
        let resource = MockPool::new();
        let config = Config::default();
        let pool = PoolRuntime::<MockPool>::new(config, 1);

        let evicted = pool.run_maintenance(&resource).await;
        assert_eq!(evicted, 0);
    }
}
