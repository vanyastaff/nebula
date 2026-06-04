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
    context::ResourceContext,
    error::Error,
    guard::ResourceGuard,
    metrics::ResourceOpsMetrics,
    options::AcquireOptions,
    release_queue::ReleaseQueue,
    resource::Resource,
    topology::pooled::{InstanceMetrics, Pooled, RecycleDecision, config::Config},
    topology_tag::TopologyTag,
};

// ─── Static error messages ───────────────────────────────────────────────────

/// Pool cannot operate with zero max size.
const ERR_MAX_SIZE_ZERO: &str = "PoolRuntime: config.max_size must be > 0 (got 0 — would \
     deadlock the checkout semaphore on first acquire)";

/// The checkout semaphore was closed (pool is shutting down).
const ERR_SEMAPHORE_CLOSED: &str = "pool semaphore closed";

/// All pool slots are in use and the timeout expired.
const ERR_POOL_FULL_TIMEOUT: &str = "pool full: timed out waiting for available slot";

/// The create-semaphore was closed (pool is shutting down).
const ERR_CREATE_SEMAPHORE_CLOSED: &str = "pool: create semaphore closed";

/// Timed out waiting for a create-semaphore permit.
const ERR_CREATE_SEMAPHORE_TIMEOUT: &str =
    "pool: create timed out waiting for create-semaphore permit";

/// The `resource.create()` call exceeded `create_timeout`.
const ERR_CREATE_TIMED_OUT: &str = "pool: create timed out";

// ─────────────────────────────────────────────────────────────────────────────

/// A single pooled instance with its metrics and config fingerprint.
///
/// The semaphore permit no longer lives here — it is held in
/// `GuardInner::Guarded` so that it is returned even if the release
/// callback panics.
struct PoolEntry<R: Resource> {
    runtime: R::Runtime,
    metrics: InstanceMetrics,
    fingerprint: u64,
    /// Snapshot of the pool's revoke epoch at the moment this instance
    /// began creation (or, for an instance reconstructed on release, the
    /// epoch carried verbatim from its original creation).
    ///
    /// Distinct from `fingerprint`: `fingerprint` tracks *config* changes,
    /// this tracks *credential revocations*. A credential revoke bumps the
    /// pool's live counter; every return-to-idle path compares this stored
    /// snapshot against the live counter and destroys (never recycles or
    /// admits) the instance when the counter advanced past it, so an
    /// instance created or checked out against a since-revoked credential
    /// can never re-enter the idle queue or be handed onward. The snapshot
    /// is taken once at creation and carried unchanged through
    /// idle → checkout → release: re-reading it at release time would read
    /// the post-revoke value on a pre-revoke instance and fail to fence it.
    revoke_epoch: u64,
    /// When this entry was last returned to the idle queue.
    /// `None` for freshly created entries that have never been idle.
    returned_at: Option<Instant>,
}

/// Result of attempting to pop an idle instance from the pool.
enum IdleResult<R: Resource> {
    /// A valid idle instance was found — wrapped in a handle.
    Found(ResourceGuard<R>),
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
    /// Monotonic per-pool credential-revoke counter.
    ///
    /// Bumped synchronously by the manager when a credential bound to this
    /// pool is revoked (before the revoke hook is dispatched, the same
    /// synchronous-before-`.await` discipline as the resource taint). Every
    /// instance carries the value this counter held when it began creation
    /// ([`PoolEntry::revoke_epoch`]); every path that would return an
    /// instance to the idle queue destroys it instead when this counter has
    /// advanced past the instance's snapshot. This closes the revoke →
    /// recycle / in-flight-create / warmup / maintenance window in which an
    /// instance authenticated with a since-revoked credential could
    /// otherwise re-enter idle and be served to the next acquirer
    /// (cross-tenant reuse). Separate from `current_fingerprint` so a
    /// config reload and a credential revoke remain independent triggers.
    revoke_epoch: Arc<AtomicU64>,
}

impl<R: Resource> PoolRuntime<R> {
    /// Fallibly creates a new pool runtime, returning a typed
    /// [`Error::permanent`] instead of aborting on an invalid
    /// `(min_size, max_size)` topology.
    ///
    /// This is the constructor the **registration path must use**. A
    /// `TopologyRuntime::Pool` built from operator-/JSON-supplied config
    /// (the engine activation registrar feeding
    /// [`Manager::register`](crate::Manager::register) /
    /// [`register_resolved`](crate::Manager::register_resolved)) flows
    /// untrusted input here, so the #390 `(min_size, max_size)` sanity
    /// check has to fail safely as a registration `Error` rather than
    /// abort the process — an abort on library input is a CLAUDE.md
    /// violation. [`new`](Self::new) is the infallible wrapper retained
    /// only for compile-time-known callers (doctests, const-shaped
    /// fixtures), where an invalid topology is a programmer error.
    ///
    /// The `fingerprint` is a config-change detection token; see
    /// [`new`](Self::new) for its semantics.
    ///
    /// # Errors
    ///
    /// - [`Error::permanent`] when `max_size == 0` (would otherwise
    ///   deadlock the checkout semaphore on first acquire).
    /// - [`Error::permanent`] when `min_size > max_size`.
    pub fn try_new(config: Config, fingerprint: u64) -> Result<Self, Error> {
        // #390: reject an unworkable pool topology at construction rather
        // than deadlock on first acquire. On the registration path the
        // config is operator/JSON-derived, so this is a typed
        // `Error::permanent` (aborting on library input is a CLAUDE.md
        // violation); invariants that must hold for the pool to function
        // at all are rejected here, never silently clamped.
        if config.max_size == 0 {
            return Err(Error::permanent(ERR_MAX_SIZE_ZERO));
        }
        if config.min_size > config.max_size {
            return Err(Error::permanent(format!(
                "PoolRuntime: config.min_size ({}) must be <= max_size ({})",
                config.min_size, config.max_size,
            )));
        }

        let semaphore = Arc::new(Semaphore::new(config.max_size as usize));
        // #390: cap concurrent instance creation. `max(1)` protects us
        // from a pathological `max_concurrent_creates = 0` config that
        // would otherwise deadlock the pool on first acquire.
        let create_semaphore = Arc::new(Semaphore::new(
            (config.max_concurrent_creates as usize).max(1),
        ));
        Ok(Self {
            idle: Arc::new(Mutex::new(VecDeque::new())),
            semaphore,
            create_semaphore,
            config,
            current_fingerprint: Arc::new(AtomicU64::new(fingerprint)),
            revoke_epoch: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Creates a new pool runtime with the given configuration.
    ///
    /// The `fingerprint` is a config-change detection token. When
    /// [`Manager::reload_config`](crate::Manager::reload_config) is called,
    /// idle instances whose fingerprint differs from the current one are
    /// evicted. Use `0` as the initial value; the manager updates it
    /// automatically on reload. Implement
    /// [`ResourceConfig::fingerprint()`](crate::ResourceConfig::fingerprint)
    /// on your config type to enable change detection.
    ///
    /// # Panics
    ///
    /// Aborts if `max_size == 0` or `min_size > max_size`. This is the
    /// infallible constructor for **compile-time-known** configs only
    /// (doctests, const-shaped fixtures), where an invalid topology is a
    /// programmer error caught at the first test run. Any path that builds
    /// a pool from runtime/operator/JSON config (registration) **must**
    /// use [`try_new`](Self::try_new), which returns a typed
    /// [`Error::permanent`] instead of aborting the process.
    pub fn new(config: Config, fingerprint: u64) -> Self {
        // #390: fail loudly at construction rather than deadlock on first
        // acquire. `try_new` surfaces the same check as a typed
        // `Error::permanent` for the registration path; this assert form
        // is kept only for direct compile-time-known callers (the README
        // and doctests). Invariants that must hold for the pool to
        // function at all are asserted here rather than silently clamped.
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
            revoke_epoch: Arc::new(AtomicU64::new(0)),
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

    /// Advances the pool's credential-revoke counter by one.
    ///
    /// Called synchronously by the manager when a credential bound to this
    /// pool is revoked — before the revoke hook is dispatched, so by the
    /// time the hook walks the idle queue every still-live instance created
    /// against the now-revoked credential already has a snapshot strictly
    /// behind this counter and is destroyed (never recycled or admitted) on
    /// whichever return-to-idle path it reaches. `Release` pairs with the
    /// `Acquire` load on every return-to-idle site.
    pub fn bump_revoke_epoch(&self) {
        self.revoke_epoch.fetch_add(1, Ordering::Release);
    }

    /// Reads the pool's current credential-revoke counter.
    fn current_revoke_epoch(&self) -> u64 {
        self.revoke_epoch.load(Ordering::Acquire)
    }

    /// Returns the number of idle instances currently in the pool.
    pub async fn idle_count(&self) -> usize {
        self.idle.lock().await.len()
    }

    /// Invokes a per-slot credential rotation hook against every idle
    /// pooled runtime instance, in order.
    ///
    /// Used by the per-slot rotation dispatch: each idle instance is handed
    /// to `Resource::on_credential_refresh` / `on_credential_revoke` so a
    /// connection-bound pool can rebuild against the rotated credential.
    /// Checked-out instances are owned by their `ResourceGuard`; the slot
    /// cell is lock-free on `&self`, so the rotated credential is already
    /// visible to them and they re-read it on their own release/recycle
    /// path. The idle-queue lock is held across the awaited hooks so an
    /// instance cannot be checked out mid-rotation and miss the hook.
    ///
    /// `refresh = true` selects `on_credential_refresh`, `false` selects
    /// `on_credential_revoke`. The hook is called inline (not via a
    /// borrowing closure) so the per-entry `&R::Runtime` never escapes the
    /// idle lock. The first hook error is returned; remaining idle
    /// instances are still visited so one bad instance doesn't skip the
    /// rest.
    ///
    /// Tradeoff: because the idle lock spans every entry's hook `.await`,
    /// a slow hook blocks concurrent idle checkouts for the full rotation
    /// duration (head-of-line blocking). New-instance creation is
    /// unaffected — that path does not take this lock. This is tolerated
    /// because rotation is rare (not a hot path); note the external
    /// timeout the dispatch caller may apply bounds the *whole*
    /// refresh/revoke dispatch, **not** each idle-entry hook — a single
    /// pathologically slow hook can still hold the idle lock for that
    /// hook's full (unbounded) duration. Per-entry hook timeouts are a
    /// tracked deferred design item, not implemented here. Do not
    /// "optimize" by dropping and reacquiring the lock between entries:
    /// that reopens the window for an instance to be checked out
    /// mid-rotation and miss its hook, violating the post-revoke
    /// invariant guaranteed here (credential isolation). If rotation ever moves onto
    /// a hot path, or hook latency becomes unbounded, revisit this only
    /// via a snapshot-with-epoch-reconcile design (capture the idle set
    /// under a brief lock, run hooks lock-free, then reconcile against
    /// the epoch on release) — never by simply widening the unlocked
    /// window.
    pub(crate) async fn dispatch_slot_hook_over_idle(
        &self,
        resource: &R,
        slot: &str,
        refresh: bool,
    ) -> Result<(), R::Error> {
        let idle = self.idle.lock().await;
        let mut first_err: Option<R::Error> = None;
        for entry in &*idle {
            let res = if refresh {
                resource.on_credential_refresh(slot, &entry.runtime).await
            } else {
                resource.on_credential_revoke(slot, &entry.runtime).await
            };
            if let Err(e) = res
                && first_err.is_none()
            {
                first_err = Some(e);
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
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

// `run_maintenance` + `should_evict` need only `R: Resource` (eviction calls
// `Resource::destroy` and reads pool fields — no `Pooled`/`Clone`/`Into`
// conversions), so they live in this weak-bound block. That lets the
// `R: Resource`-only registration path drive the background maintenance
// reaper without the acquire-path topology bounds.
impl<R: Resource> PoolRuntime<R> {
    /// Runs one maintenance cycle: evicts idle-timeout, max-lifetime,
    /// stale-fingerprint, and credential-revoked entries from the idle
    /// queue.
    ///
    /// Returns the number of entries evicted. Each evicted entry is destroyed
    /// via [`Resource::destroy`].
    pub async fn run_maintenance(&self, resource: &R) -> usize {
        let to_destroy = {
            let mut idle = self.idle.lock().await;
            let current_fp = self.current_fingerprint.load(Ordering::Acquire);
            // Read under the idle lock — the same lock the revoke idle-walk
            // holds — so an entry whose credential was revoked (its snapshot
            // now behind this counter) is evicted here rather than
            // re-deposited. The revoke hook visits but does not evict idle
            // entries, so without this arm a non-stale, non-timed-out
            // pre-revoke instance would be `keep.push_back`-ed and served to
            // the next acquirer.
            let current_revoke_epoch = self.current_revoke_epoch();
            let now = Instant::now();

            let mut keep = VecDeque::with_capacity(idle.len());
            let mut evict = Vec::new();

            for entry in idle.drain(..) {
                if Self::should_evict(&entry, &self.config, current_fp, current_revoke_epoch, now) {
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
    fn should_evict(
        entry: &PoolEntry<R>,
        config: &Config,
        current_fp: u64,
        current_revoke_epoch: u64,
        now: Instant,
    ) -> bool {
        // Credential revoked since this instance was created. Distinct from
        // the fingerprint arm: a config reload and a credential revoke are
        // independent triggers, so `should_evict` checks both.
        if entry.revoke_epoch != current_revoke_epoch {
            return true;
        }
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
        ctx: &ResourceContext,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        options: &AcquireOptions,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Result<ResourceGuard<R>, Error> {
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
            .create_entry(resource, resource_config, ctx, false)
            .await
        {
            Ok(e) => e,
            Err(e) => return Err(e),
        };

        // Cancel-safety: if the future is dropped between here and
        // `build_guarded_handle`, the guard submits an async destroy
        // via the ReleaseQueue — without this, only the runtime's
        // sync `Drop` ran, leaking the server-side handle.
        let guard = CreateGuard::new(entry, resource.clone(), Arc::clone(release_queue));

        // Prepare the new instance.
        if let Err(e) = resource.prepare(guard.runtime(), ctx).await {
            let entry = guard.defuse();
            let _ = resource.destroy(entry.runtime).await;
            // permit drops here, returning the slot.
            return Err(e.into());
        }

        let entry = guard.defuse();
        // Fence: a revoke that landed while this create was in flight must
        // not be handed onward (HikariCP #1836).
        let entry = self.fence_freshly_created(entry, resource).await?;
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
        ctx: &ResourceContext,
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
            // validation steps. If cancelled mid-check, the guard submits
            // an async destroy via the ReleaseQueue rather than silently
            // leaking the server-side handle.
            let guard = CreateGuard::new(entry, resource.clone(), Arc::clone(release_queue));

            // Stale fingerprint — destroy silently.
            let current_fp = self.current_fingerprint.load(Ordering::Acquire);
            if guard.entry().fingerprint != current_fp {
                let entry = guard.defuse();
                let _ = resource.destroy(entry.runtime).await;
                continue;
            }

            // Credential revoked while this instance sat idle — destroy it
            // rather than hand it onward. The synchronous acquire-side taint
            // re-check already rejects an acquire that races a revoke, but
            // an idle instance whose credential was revoked must not be
            // served even on a path that somehow reaches here, so this is
            // the same symmetric guard as the stale-fingerprint pop above.
            if guard.entry().revoke_epoch != self.current_revoke_epoch() {
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
            Ok(Err(_closed)) => Err(Error::permanent(ERR_SEMAPHORE_CLOSED)),
            Err(_timeout) => Err(Error::backpressure(ERR_POOL_FULL_TIMEOUT)),
        }
    }

    /// Creates a new pool entry via `resource.create()`.
    ///
    /// All creation goes through this funnel and is gated on
    /// `create_semaphore` so a burst of acquires cannot stampede
    /// a fragile backend with `max_size` parallel connects. The permit
    /// is released as soon as `Resource::create` returns.
    ///
    /// The whole path — permit wait + `resource.create` — shares a
    /// single `create_timeout` budget. Both the create semaphore wait
    /// and the actual create are bounded by the remaining budget so a
    /// slow-creating backend cannot stall callers forever.
    ///
    /// When `non_blocking` is `true` (the `try_acquire` path), the
    /// create-semaphore wait is replaced with a `try_acquire_owned`
    /// that returns `Backpressure` immediately instead of queueing,
    /// preserving the non-blocking contract of `try_acquire`.
    async fn create_entry(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
        non_blocking: bool,
    ) -> Result<PoolEntry<R>, Error> {
        // Snapshot the revoke counter *before* the create-semaphore wait
        // and `resource.create()`. An instance whose creation straddles a
        // revoke (in flight when the credential is revoked, completing
        // after) must be fenced: capturing the epoch at create-start means
        // a revoke landing during the create advances the live counter past
        // this snapshot, so every return-to-idle path destroys the instance
        // instead of admitting it (HikariCP #1836). Reading it after the
        // create returns would capture the already-bumped value and let the
        // post-revoke instance through.
        let revoke_epoch = self.current_revoke_epoch();
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
                    return Err(Error::permanent(ERR_CREATE_SEMAPHORE_CLOSED));
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
                    return Err(Error::permanent(ERR_CREATE_SEMAPHORE_CLOSED));
                },
                Err(_) => {
                    return Err(Error::backpressure(ERR_CREATE_SEMAPHORE_TIMEOUT));
                },
            }
        };

        // Use `timeout_at` with the same absolute deadline so the budget
        // is shared: a long permit wait shortens the time available to
        // `resource.create`.
        let runtime =
            match tokio::time::timeout_at(deadline.into(), resource.create(config, ctx)).await {
                Ok(Ok(rt)) => rt,
                Ok(Err(e)) => return Err(e.into()),
                Err(_timeout) => {
                    return Err(Error::transient(ERR_CREATE_TIMED_OUT));
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
            revoke_epoch,
            returned_at: None,
        })
    }

    /// Fences a freshly created instance before it is handed to the
    /// caller: if a credential revoke advanced the pool's counter while the
    /// `create` was in flight, the instance was authenticated with the
    /// now-revoked credential and must be destroyed, not admitted (HikariCP
    /// #1836 — the in-flight-create-completing-after-revoke race that an
    /// idle-walk / evict-only approach cannot catch).
    ///
    /// `Ok(entry)` when the snapshot still matches the live counter (admit
    /// it); `Err` after destroying the instance when the counter advanced —
    /// the acquire fails closed rather than serving a revoked-credential
    /// runtime. The revoke epoch is captured at the *start* of
    /// `create_entry`, so a revoke that lands any time during the create is
    /// observed here.
    async fn fence_freshly_created(
        &self,
        entry: PoolEntry<R>,
        resource: &R,
    ) -> Result<PoolEntry<R>, Error> {
        if entry.revoke_epoch != self.current_revoke_epoch() {
            let _ = resource.destroy(entry.runtime).await;
            return Err(Error::permanent(format!(
                "{}: credential revoked while a pool instance was being \
                 created — instance destroyed, not handed to the caller",
                R::key(),
            )));
        }
        Ok(entry)
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
    ) -> ResourceGuard<R> {
        let idle = self.idle.clone();
        let current_fp_ref = self.current_fingerprint.clone();
        let revoke_epoch_ref = self.revoke_epoch.clone();
        let max_lifetime = self.config.max_lifetime;

        ResourceGuard::guarded_with_permit(
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
                    // Carry the creation-time revoke snapshot verbatim. It
                    // must NOT be re-read here: a release-time load would
                    // observe the post-revoke counter on a pre-revoke
                    // instance too, so it could never distinguish an
                    // instance whose credential was revoked while it was
                    // checked out from one created after the revoke.
                    revoke_epoch: entry.revoke_epoch,
                    returned_at: None, // set by release_entry on idle push
                };

                // Load fingerprint at release time (not checkout time) to detect
                // config changes that happened while the handle was checked out.
                let current_fp = current_fp_ref.load(Ordering::Acquire);
                // Hand the *live* revoke counter (not a load) into
                // `release_entry`: the recycle decision can park, and a
                // revoke landing during that park must still fence this
                // instance, so the counter is re-read inside the release
                // logic rather than captured here.
                let revoke_epoch = revoke_epoch_ref.clone();
                // Return the teardown future. The guard awaits it inline on
                // `ResourceGuard::release` (surfacing the recycle/destroy
                // `Result`) or submits it to its `ReleaseQueue` on `Drop`
                // (best-effort, `Result` discarded). The revoke-epoch re-read
                // under the idle lock still lives inside `release_entry`.
                Box::pin(release_entry(
                    resource,
                    entry,
                    tainted,
                    current_fp,
                    revoke_epoch,
                    max_lifetime,
                    idle,
                ))
            },
            Some(permit),
            release_queue,
        )
    }

    /// Attempts a non-blocking acquire: returns immediately with
    /// Backpressure if the pool is at capacity (all `max_size` slots hold active ResourceGuards).
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
        ctx: &ResourceContext,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        _options: &AcquireOptions,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Result<ResourceGuard<R>, Error> {
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
        // we return Backpressure instead of waiting.
        let entry = match self
            .create_entry(resource, resource_config, ctx, true)
            .await
        {
            Ok(e) => e,
            Err(e) => return Err(e),
        };

        // Cancel-safety guard: see analogous `acquire`-path comment
        // upstream. Submits async destroy via ReleaseQueue if cancelled.
        let guard = CreateGuard::new(entry, resource.clone(), Arc::clone(release_queue));
        if let Err(e) = resource.prepare(guard.runtime(), ctx).await {
            let entry = guard.defuse();
            let _ = resource.destroy(entry.runtime).await;
            return Err(e.into());
        }

        let entry = guard.defuse();
        // Fence: a revoke that landed while this create was in flight must
        // not be handed onward (HikariCP #1836).
        let entry = self.fence_freshly_created(entry, resource).await?;
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
    /// semaphore permits — permits are only held by active ResourceGuards.
    ///
    /// If a creation fails, warmup stops early and returns the count created
    /// so far (partial warmup is acceptable; on-demand creation handles the rest).
    pub async fn warmup(
        &self,
        resource: &R,
        resource_config: &R::Config,
        ctx: &ResourceContext,
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
                self.warmup_sequential(resource, resource_config, ctx, target)
                    .await
            },
            WarmupStrategy::Staggered { interval } => {
                self.warmup_staggered(resource, resource_config, ctx, target, interval)
                    .await
            },
        }
    }

    /// Admits a freshly warmed instance to the idle queue, or destroys it
    /// if a credential revoke landed during its (possibly slow) creation.
    ///
    /// Returns `true` when the instance was admitted, `false` when it was
    /// fenced and destroyed. The instance carries the revoke snapshot taken
    /// at the start of its `create_entry`; comparing it against the live
    /// counter under the idle lock makes the compare-then-push atomic
    /// against the revoke idle-walk (which holds the same lock), so a
    /// warmup running concurrently with — or after — a revoke can never
    /// deposit an instance authenticated with the revoked credential.
    async fn admit_warmed_entry(&self, mut entry: PoolEntry<R>, resource: &R) -> bool {
        let mut idle = self.idle.lock().await;
        if entry.revoke_epoch != self.current_revoke_epoch() {
            drop(idle);
            let _ = resource.destroy(entry.runtime).await;
            return false;
        }
        entry.returned_at = Some(Instant::now());
        idle.push_back(entry);
        true
    }

    /// Sequential warmup helper: creates one instance at a time.
    async fn warmup_sequential(
        &self,
        resource: &R,
        resource_config: &R::Config,
        ctx: &ResourceContext,
        target: usize,
    ) -> usize {
        let mut created = 0usize;
        for _ in 0..target {
            match self
                .create_entry(resource, resource_config, ctx, false)
                .await
            {
                Ok(entry) => {
                    if self.admit_warmed_entry(entry, resource).await {
                        created += 1;
                        tracing::debug!(
                            key = %R::key(),
                            created,
                            target,
                            "pool warmup: instance created"
                        );
                    } else {
                        tracing::debug!(
                            key = %R::key(),
                            target,
                            "pool warmup: instance fenced by credential revoke, destroyed"
                        );
                    }
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
        ctx: &ResourceContext,
        target: usize,
        interval: Duration,
    ) -> usize {
        let mut created = 0usize;
        for i in 0..target {
            if i > 0 {
                tokio::time::sleep(interval).await;
            }
            match self
                .create_entry(resource, resource_config, ctx, false)
                .await
            {
                Ok(entry) => {
                    if self.admit_warmed_entry(entry, resource).await {
                        created += 1;
                        tracing::debug!(
                            key = %R::key(),
                            created,
                            target,
                            "pool warmup (staggered): instance created"
                        );
                    } else {
                        tracing::debug!(
                            key = %R::key(),
                            target,
                            "pool warmup (staggered): instance fenced by credential revoke, destroyed"
                        );
                    }
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
/// dropped (it lives in `GuardInner::Guarded`, not in the callback closure).
///
/// This **is** the teardown future the guard's release callback returns:
/// `ResourceGuard::release` awaits it and surfaces the `Result`; `Drop`
/// submits it to the [`ReleaseQueue`] discarding the `Result`. The returned
/// `Result` is the `destroy` outcome on every destroy arm (so an awaited
/// `release()` observes a failed destroy), `Ok(())` when the instance is
/// recycled back to idle. Every control-flow branch — tainted, stale
/// fingerprint, the live revoke-epoch re-read, max-lifetime, broken, and the
/// post-`recycle` revoke re-check under the idle lock — is preserved
/// verbatim; only the swallowed `let _ =` becomes a propagated `Result`.
async fn release_entry<R>(
    resource: R,
    entry: PoolEntry<R>,
    tainted: bool,
    current_fp: u64,
    revoke_epoch: Arc<AtomicU64>,
    max_lifetime: Option<Duration>,
    idle: Arc<Mutex<VecDeque<PoolEntry<R>>>>,
) -> Result<(), Error>
where
    R: Pooled + Send + Sync + 'static,
{
    // Tainted — destroy immediately.
    if tainted {
        return resource.destroy(entry.runtime).await.map_err(Into::into);
    }

    // Stale fingerprint — config changed since checkout.
    if entry.fingerprint != current_fp {
        return resource.destroy(entry.runtime).await.map_err(Into::into);
    }

    // Credential revoked while this handle was checked out (or while its
    // release sat queued). The counter is re-read live here, not taken from
    // the value captured when the release was submitted: `recycle()` below
    // can park arbitrarily long, and the revoke that must fence this
    // instance may land *during* that park (after submit). Reading it now,
    // before the recycle decision can keep the instance, guarantees a
    // revoke at any point up to this check destroys the instance instead of
    // returning it to idle.
    if entry.revoke_epoch != revoke_epoch.load(Ordering::Acquire) {
        return resource.destroy(entry.runtime).await.map_err(Into::into);
    }

    // Max lifetime exceeded.
    if max_lifetime.is_some_and(|max| entry.metrics.created_at.elapsed() > max) {
        return resource.destroy(entry.runtime).await.map_err(Into::into);
    }

    // Broken check (sync).
    if resource.is_broken(&entry.runtime).is_broken() {
        return resource.destroy(entry.runtime).await.map_err(Into::into);
    }

    // Async recycle check.
    match resource.recycle(&entry.runtime, &entry.metrics).await {
        Ok(RecycleDecision::Keep) => {
            // Re-check after the recycle await: a revoke can land while
            // `recycle()` is in flight (it may park), so the pre-recycle
            // check above is not sufficient on its own. Holding the idle
            // lock across the compare-then-push makes the decision atomic
            // against the revoke idle-walk (which also takes this lock).
            let mut idle = idle.lock().await;
            if entry.revoke_epoch != revoke_epoch.load(Ordering::Acquire) {
                drop(idle);
                return resource.destroy(entry.runtime).await.map_err(Into::into);
            }
            let mut entry = entry;
            entry.returned_at = Some(Instant::now());
            idle.push_back(entry);
            Ok(())
        },
        Ok(RecycleDecision::Drop) | Err(_) => {
            resource.destroy(entry.runtime).await.map_err(Into::into)
        },
    }
}

/// Cancel-safety guard for the create-then-prepare sequence.
///
/// Wraps a [`PoolEntry`] between creation and handle construction. If
/// the future is cancelled (e.g. via `tokio::select!` or timeout) after
/// `create()` succeeds but before the handle is built, `Drop` submits
/// the freshly-built runtime to the [`ReleaseQueue`] for an async
/// `Resource::destroy` — symmetric with the release-path's
/// `release_entry`. Without this, the runtime's *sync* `Drop` would run
/// inline (closing the local handle) but the server-side resource — DB
/// session, broker subscription, OS-level handle — would leak.
///
/// Call [`defuse`](Self::defuse) to take ownership of the entry once
/// the handle is safely constructed. `defuse` consumes the guard by
/// value, so the borrow checker prevents any use of the guard after
/// `defuse` — `entry()` / `runtime()` cannot be invoked on a defused
/// guard, and the guard's `Drop` never runs against a defused entry.
///
/// `entry` is the only `Option` field; `resource` and `release_queue`
/// stay populated for the guard's whole lifetime so the `Drop` impl
/// never has to inspect `Option` invariants — it clones them (both
/// cheap: `R: Clone` is required by `release_entry` already, and
/// `Arc<ReleaseQueue>` is a refcount bump) into the queued destroy
/// closure. `Drop` therefore carries no `unwrap` / `unreachable!` /
/// `expect` paths — material under cancellation.
struct CreateGuard<R>
where
    R: Pooled + Clone + Send + Sync + 'static,
    R::Lease: Into<R::Runtime>,
{
    /// `None` after [`defuse`](Self::defuse) took it out; `Some(_)`
    /// for any guard a caller can still observe. `Drop` short-circuits
    /// on `None`.
    entry: Option<PoolEntry<R>>,
    /// Cloned resource handle so the Drop path can call
    /// `Resource::destroy(entry.runtime)` from the [`ReleaseQueue`]
    /// without re-entering the originating pool context. Same Clone
    /// requirement that `release_entry` already imposes on `R`.
    resource: R,
    /// Reference to the pool's `ReleaseQueue` so Drop can submit the
    /// async destroy without spawning an orphan `tokio::spawn` (which
    /// would lack the queue's bounded backpressure + shutdown drain).
    release_queue: Arc<ReleaseQueue>,
}

impl<R> CreateGuard<R>
where
    R: Pooled + Clone + Send + Sync + 'static,
    R::Lease: Into<R::Runtime>,
{
    /// Creates a new guard wrapping the given pool entry.
    fn new(entry: PoolEntry<R>, resource: R, release_queue: Arc<ReleaseQueue>) -> Self {
        Self {
            entry: Some(entry),
            resource,
            release_queue,
        }
    }

    /// Returns a reference to the inner entry for inspection.
    fn entry(&self) -> &PoolEntry<R> {
        // guard-justified: `entry()` is private + only called between
        // `new` (which inserts `Some(_)`) and `defuse` (which consumes
        // `self`); reaching here with `None` would mean `entry()` was
        // called *after* `defuse`, which the by-value consumption in
        // `defuse(mut self)` makes a borrow-checker error, not a
        // runtime path.
        self.entry
            .as_ref()
            .unwrap_or_else(|| unreachable!("CreateGuard accessed after defuse"))
    }

    /// Returns a reference to the runtime for use in `prepare()`.
    fn runtime(&self) -> &R::Runtime {
        &self.entry().runtime
    }

    /// Consumes the guard and returns the wrapped entry.
    ///
    /// After this call, the guard is gone; its `Drop` runs against
    /// `entry: None` and short-circuits without submitting a destroy.
    fn defuse(mut self) -> PoolEntry<R> {
        // guard-justified: `defuse` consumes `self` by value, so the
        // borrow checker forbids calling it twice on the same guard.
        // `self.entry` is `Some(_)` for the whole observable lifetime
        // of the guard (set in `new`, only mutated here or in `Drop`,
        // both of which consume the guard), so `take()` cannot return
        // `None` on this path.
        self.entry
            .take()
            .unwrap_or_else(|| unreachable!("CreateGuard defused twice"))
    }
}

impl<R> Drop for CreateGuard<R>
where
    R: Pooled + Clone + Send + Sync + 'static,
    R::Lease: Into<R::Runtime>,
{
    fn drop(&mut self) {
        let Some(entry) = self.entry.take() else {
            return; // defused — nothing to clean up
        };
        // `resource` and `release_queue` are non-`Option` and always
        // populated for the guard's observable lifetime. Cloning them
        // (cheap — `R: Clone` is already a pool requirement,
        // `Arc::clone` is a refcount bump) into the queued destroy
        // closure keeps the `Drop` body free of `unwrap` /
        // `unreachable!` / `expect` paths — important because `Drop`
        // runs in arbitrary cancellation contexts.
        let resource = self.resource.clone();
        let release_queue = Arc::clone(&self.release_queue);
        tracing::warn!(
            resource = %R::key(),
            "cancel-safety: acquire future cancelled mid-create — \
             scheduling async destroy via ReleaseQueue"
        );
        release_queue.submit(move || {
            Box::pin(async move {
                // `release_entry` would re-check fingerprint / revoke /
                // recycle, but a freshly-created instance that was
                // cancelled before reaching the prepare/fence step was
                // never admitted to the idle queue or handed to a
                // caller; the *only* correct cleanup is destroy. Mirror
                // `release_entry`'s tainted arm and discard the error
                // (recorded via the resource's own destroy path).
                let _ = resource.destroy(entry.runtime).await;
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use nebula_core::{ExecutionId, ResourceKey, resource_key};

    use super::*;
    use crate::{
        context::ResourceContext,
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
        /// Counts every `destroy` invocation — used by the cancel-drop
        /// regression to assert the async destroy actually ran via the
        /// `ReleaseQueue` (vs. just being inline-`Drop`-ed).
        destroy_count: Arc<AtomicU64>,
    }

    impl MockPool {
        fn new() -> Self {
            Self {
                fail_create: Arc::new(AtomicBool::new(false)),
                fail_check: Arc::new(AtomicBool::new(false)),
                break_on_return: Arc::new(AtomicBool::new(false)),
                destroy_count: Arc::new(AtomicU64::new(0)),
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

        fn key() -> ResourceKey {
            resource_key!("mock-pool")
        }

        fn create(
            &self,
            _config: &PoolTestConfig,
            _ctx: &ResourceContext,
        ) -> impl Future<Output = Result<u32, MockError>> + Send {
            let fail = self.fail_create.load(Ordering::Relaxed);
            async move {
                if fail {
                    Err(MockError("create failed".into()))
                } else {
                    Ok(1)
                }
            }
        }

        fn check(&self, _runtime: &u32) -> impl Future<Output = Result<(), MockError>> + Send {
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
            self.destroy_count.fetch_add(1, Ordering::Relaxed);
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

    fn test_ctx() -> ResourceContext {
        use nebula_core::scope::Scope;
        use tokio_util::sync::CancellationToken;
        let scope = Scope {
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        };
        ResourceContext::minimal(scope, CancellationToken::new())
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
        tokio::time::sleep(Duration::from_millis(50)).await;

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
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                None,
            )
            .await
            .unwrap();
        drop(handle);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(pool.idle_count().await, 1);

        // Second acquire should reuse the idle instance.
        let handle2 = pool
            .acquire(
                &resource,
                &PoolTestConfig,
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                None,
            )
            .await;
        assert!(handle2.is_ok());

        drop(handle2);
        tokio::time::sleep(Duration::from_millis(50)).await;

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
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                None,
            )
            .await
            .unwrap();
        drop(handle);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(pool.idle_count().await, 1);

        // Now mark broken — next acquire should destroy the idle instance
        // and create a fresh one.
        resource.break_on_return.store(true, Ordering::Relaxed);

        let handle2 = pool
            .acquire(
                &resource,
                &PoolTestConfig,
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                None,
            )
            .await;
        assert!(handle2.is_ok());

        drop(handle2);
        tokio::time::sleep(Duration::from_millis(50)).await;

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
        tokio::time::sleep(Duration::from_millis(50)).await;

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
        let result = tokio::time::timeout(Duration::from_millis(100), sem.acquire_owned()).await;
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
            create_timeout: Duration::from_millis(200),
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

    // -- `try_new` registration-path topology validation (#390) --
    //
    // Ports the behaviour the U6-deleted `register_pooled_rejects_*`
    // tests covered onto the surviving fallible constructor: an invalid
    // `(min_size, max_size)` from operator/JSON config must fail the
    // registration path as a typed `Error::permanent`, never abort the
    // process. `PoolRuntime::new`'s assert is the compile-time-known
    // counterpart and is exercised by the const-shaped `new(...)` calls
    // throughout the rest of this module.

    #[test]
    fn try_new_rejects_max_size_zero_with_permanent_error() {
        let config = Config {
            min_size: 0,
            max_size: 0,
            ..Config::default()
        };
        let err = match PoolRuntime::<MockPool>::try_new(config, 1) {
            Err(e) => e,
            Ok(_) => panic!("max_size == 0 must be a typed registration error, not a pool"),
        };
        assert_eq!(*err.kind(), crate::error::ErrorKind::Permanent);
        assert!(
            err.to_string().contains("max_size"),
            "error message must name max_size, got: {err}",
        );
    }

    #[test]
    fn try_new_rejects_min_greater_than_max_with_permanent_error() {
        let config = Config {
            min_size: 5,
            max_size: 2,
            ..Config::default()
        };
        let err = match PoolRuntime::<MockPool>::try_new(config, 1) {
            Err(e) => e,
            Ok(_) => panic!("min > max must be a typed registration error, not a pool"),
        };
        assert_eq!(*err.kind(), crate::error::ErrorKind::Permanent);
        assert!(
            err.to_string().contains("min_size") && err.to_string().contains("max_size"),
            "error message must name min_size and max_size, got: {err}",
        );
    }

    #[tokio::test]
    async fn try_new_accepts_valid_config_and_pool_is_usable() {
        let resource = MockPool::new();
        let config = Config {
            min_size: 0,
            max_size: 2,
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::try_new(config, 1)
            .expect("valid (min_size, max_size) must construct");
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let ctx = test_ctx();

        let handle = pool
            .acquire(
                &resource,
                &PoolTestConfig,
                &ctx,
                &rq,
                0,
                &AcquireOptions::default(),
                None,
            )
            .await;
        assert!(handle.is_ok(), "valid pool from try_new must acquire");

        drop(handle);
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
            // Stamp the pool's live revoke counter so the revoke-eviction
            // arm is inert for these maintenance tests (no revoke occurred)
            // and they keep asserting the fingerprint / lifetime / timeout
            // behaviour they target.
            revoke_epoch: pool.current_revoke_epoch(),
            returned_at,
        });
    }

    #[tokio::test]
    async fn maintenance_evicts_idle_timeout_entries() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 5,
            idle_timeout: Some(Duration::from_millis(50)),
            max_lifetime: None,
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);

        // Push two entries: one returned long ago (should be evicted),
        // one returned just now (should be kept).
        let long_ago = Instant::now()
            .checked_sub(Duration::from_millis(200))
            .unwrap();
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
            max_lifetime: Some(Duration::from_millis(50)),
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);

        // One old entry (should be evicted), one fresh (should be kept).
        let old_creation = Instant::now()
            .checked_sub(Duration::from_millis(200))
            .unwrap();
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
            idle_timeout: Some(Duration::from_mins(5)),
            max_lifetime: Some(Duration::from_mins(30)),
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

    /// Site 5 (`run_maintenance` re-deposit): a non-stale, non-timed-out
    /// idle entry whose credential was revoked must be evicted by
    /// `should_evict`'s revoke-epoch arm — not `keep.push_back`-ed. With
    /// idle/lifetime disabled and a current fingerprint, the revoke arm is
    /// the *only* arm that can remove it, so this isolates that arm.
    #[tokio::test]
    async fn maintenance_evicts_revoked_idle_entry() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 5,
            idle_timeout: None,
            max_lifetime: None,
            ..Config::default()
        };
        let pool = PoolRuntime::<MockPool>::new(config, 1);

        // Two healthy, current-fingerprint, non-timed-out idle entries.
        push_idle_entry(&pool, Instant::now(), Some(Instant::now()), 1).await;
        push_idle_entry(&pool, Instant::now(), Some(Instant::now()), 1).await;

        // No revoke yet → the revoke arm is inert, nothing is evicted.
        assert_eq!(pool.run_maintenance(&resource).await, 0);
        assert_eq!(pool.idle_count().await, 2);

        // Revoke: bump the per-pool counter. Both entries snapshotted the
        // pre-bump value, so their snapshot is now strictly behind.
        pool.bump_revoke_epoch();

        let evicted = pool.run_maintenance(&resource).await;
        assert_eq!(
            evicted, 2,
            "should_evict's revoke-epoch arm must evict every idle entry \
             whose snapshot is behind the bumped counter (the re-deposit \
             gap), even though fingerprint/lifetime/timeout would all keep \
             them"
        );
        assert_eq!(pool.idle_count().await, 0);
    }

    /// Regression: a `CreateGuard` whose entry is still `Some` when it
    /// drops must schedule `Resource::destroy` via the `ReleaseQueue`
    /// instead of just letting the runtime's sync `Drop` run. Without
    /// this, a `tokio::select!` / `timeout` that cancels the acquire
    /// future *after* `create` succeeded but *before* `defuse` ran
    /// would leak the server-side handle (DB session, broker
    /// subscription, etc.) — only the client-side `Drop` would fire.
    ///
    /// The test constructs a `CreateGuard` by hand around a fabricated
    /// `PoolEntry`, drops it without calling `defuse`, then drains the
    /// `ReleaseQueue` and asserts `MockPool::destroy` ran exactly once.
    #[tokio::test]
    async fn create_guard_drop_submits_destroy_via_release_queue() {
        let resource = MockPool::new();
        let destroy_count = Arc::clone(&resource.destroy_count);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);

        // Fabricate a freshly-built entry (mirrors what `create_entry`
        // returns just before `CreateGuard::new`). The exact field
        // values do not matter — only that `Drop` consumes the runtime.
        let entry = PoolEntry::<MockPool> {
            runtime: 42u32,
            metrics: InstanceMetrics {
                error_count: 0,
                checkout_count: 1,
                created_at: Instant::now(),
            },
            fingerprint: 1,
            revoke_epoch: 0,
            returned_at: None,
        };
        let guard = CreateGuard::new(entry, resource.clone(), Arc::clone(&rq));

        // Simulate a cancelled acquire: the future is dropped *before*
        // `defuse` ran. This is the exact shape `tokio::select!` /
        // `tokio::time::timeout` produces when they win the race.
        drop(guard);

        // The submit is fire-and-forget on a bounded mpsc — give the
        // queue a beat to dequeue + run the destroy future.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Then shut down the queue to drain anything still in flight.
        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;

        assert_eq!(
            destroy_count.load(Ordering::Relaxed),
            1,
            "CreateGuard::Drop must submit destroy via ReleaseQueue \
             when the acquire future is cancelled mid-create — otherwise \
             the server-side handle leaks while the client-side Drop runs",
        );
    }

    /// Companion to the cancel-drop regression: a `CreateGuard` that
    /// runs through `defuse`
    /// normally (the success path) MUST NOT trigger a stray
    /// `Resource::destroy` — that would double-free the runtime that
    /// is now owned by the resulting `ResourceGuard` and will be
    /// released via the normal `release_entry` path on drop. The Drop
    /// destroy-submit is conditional on `entry.take()` returning
    /// `Some` (i.e. `defuse` did not run); this test pins that
    /// short-circuit so a future refactor cannot accidentally make
    /// Drop unconditional.
    #[tokio::test]
    async fn create_guard_defuse_skips_destroy() {
        let resource = MockPool::new();
        let destroy_count = Arc::clone(&resource.destroy_count);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);

        let entry = PoolEntry::<MockPool> {
            runtime: 7u32,
            metrics: InstanceMetrics {
                error_count: 0,
                checkout_count: 1,
                created_at: Instant::now(),
            },
            fingerprint: 1,
            revoke_epoch: 0,
            returned_at: None,
        };
        let guard = CreateGuard::new(entry, resource.clone(), Arc::clone(&rq));
        let _entry = guard.defuse(); // success path consumes the guard
        // `defuse` already took the guard by value; no further `drop(guard)`
        // is possible (or needed). The runtime now lives in `_entry` and is
        // dropped at the end of this scope — sync `Drop` only, never via
        // `Resource::destroy`.

        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;

        assert_eq!(
            destroy_count.load(Ordering::Relaxed),
            0,
            "defused CreateGuard must NOT submit destroy — the runtime \
             was transferred to a ResourceGuard and is destroyed by the \
             normal release_entry path",
        );
    }
}
