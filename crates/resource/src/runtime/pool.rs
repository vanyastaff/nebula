//! Pool topology — manages a pool of N interchangeable resource instances.
//!
//! The acquire path: try idle queue -> check broken -> test_on_checkout -> prepare -> return
//! handle. If no idle instance: create new (respecting semaphore for max_size).
//! If semaphore full: wait with timeout.
//!
//! The release path (via [`ReleaseQueue`]): tainted? -> stale fingerprint? -> max_lifetime? ->
//! recycle() -> Keep/Drop.
//!
//! # Storage re-seat
//!
//! Idle instances live in the framework-owned [`InstanceStore<PoolSlot<R>>`]
//! (not a local `VecDeque`). The store owns the revoke-epoch counter and runs
//! the credential-revoke fence on **every** return-to-idle direction —
//! [`checkout`](InstanceStore::checkout) (on pop), `return_slot` / `deposit_fresh`
//! (on push), and `evict_stale` (reaper sweep). [`PoolSlot`] therefore carries
//! no `revoke_epoch`: the store stamps each slot's `checkout_epoch` when it is
//! deposited and re-reads the live counter under the idle lock on return, the
//! exact atomic compare-then-push the recycle `Keep` arm performed before.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{
    context::ResourceContext,
    error::Error,
    guard::ResourceGuard,
    metrics::ResourceOpsMetrics,
    options::AcquireOptions,
    release_queue::ReleaseQueue,
    resource::Provider,
    topology::{
        pooled::{InstanceMetrics, PoolProvider, RecycleDecision, config::Config},
        store::{InstanceStore, ReturnOutcome},
    },
    topology_tag::TopologyTag,
};

// ─── Static error messages ───────────────────────────────────────────────────

/// Pool cannot operate with zero max size.
const ERR_MAX_SIZE_ZERO: &str = "Pooled: config.max_size must be > 0 (got 0 — would \
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
/// This is the [`InstanceStore`] slot type for [`Pooled`]. The semaphore
/// permit no longer lives here — it is held in `GuardInner::Guarded` so that
/// it is returned even if the release callback panics.
///
/// **The credential-revoke snapshot is no longer a field here.** It moved to
/// the store's `checkout_epoch`: the store stamps each slot with the live
/// revoke counter on deposit and re-reads the counter under the idle lock on
/// return, so the fence is uniform across built-in and custom topologies and
/// the author never sees the epoch.
pub struct PoolSlot<R: Provider> {
    runtime: R::Instance,
    metrics: InstanceMetrics,
    fingerprint: u64,
    /// When this slot was last returned to the idle queue.
    /// `None` for freshly created slots that have never been idle.
    returned_at: Option<Instant>,
}

/// Result of attempting to pop an idle instance from the pool.
enum IdleResult<R: Provider> {
    /// A valid idle instance was found — wrapped in a handle.
    Found(ResourceGuard<R>),
    /// No usable idle instance — the permit is returned so the caller
    /// can create a new instance.
    Empty(OwnedSemaphorePermit),
}

/// A point-in-time snapshot of pool utilization.
///
/// Returned by [`Pooled::stats`] and [`Manager::pool_stats`](crate::Manager::pool_stats).
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

/// Framework pool topology — N interchangeable instances with
/// checkout/recycle/destroy.
///
/// `Pooled<R>` holds the resource handle and an [`InstanceStore<PoolSlot<R>>`]
/// idle queue, and drives the [`PoolProvider`] hooks (`is_broken` / `recycle`
/// / `prepare`) plus the [`Provider`] lifecycle (`create` / `destroy` /
/// `check`) itself. It implements the open [`Topology`](crate::topology::Topology)
/// contract so a resource that declares `type Topology = Pooled<Self>` is
/// dispatched through the uniform framework acquire pipeline.
///
/// The store owns the revoke-epoch fence; this struct holds no separate
/// `revoke_epoch` counter. [`bump_revoke_epoch`](Self::bump_revoke_epoch)
/// delegates to the store. The resource handle is **not** held here — the
/// framework manager owns it (`ManagedResource::resource`) and passes it to
/// every topology operation, so the pool never needs a second copy.
pub struct Pooled<R: Provider> {
    store: InstanceStore<PoolSlot<R>>,
    semaphore: Arc<Semaphore>,
    /// Bounds concurrent invocations of `create_entry` (#390).
    ///
    /// The checkout semaphore gates active leases; this one gates
    /// *creation* so a burst of concurrent acquires cannot fan out into
    /// `max_size` parallel `Provider::create` calls against a fragile
    /// backend.
    create_semaphore: Arc<Semaphore>,
    config: Config,
    current_fingerprint: Arc<AtomicU64>,
}

impl<R: Provider> Pooled<R> {
    /// Fallibly creates a new pool topology, returning a typed
    /// [`Error::permanent`] instead of aborting on an invalid
    /// `(min_size, max_size)` topology.
    ///
    /// This is the constructor the **registration path must use**. A
    /// `Pooled<R>` built from operator-/JSON-supplied config (the engine
    /// activation registrar feeding [`Manager::register`](crate::Manager::register) /
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
                "Pooled: config.min_size ({}) must be <= max_size ({})",
                config.min_size, config.max_size,
            )));
        }

        Ok(Self::build(config, fingerprint))
    }

    /// Creates a new pool topology with the given configuration.
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
            "Pooled: config.max_size must be > 0 (got 0 — would deadlock \
             the checkout semaphore on first acquire)",
        );
        assert!(
            config.min_size <= config.max_size,
            "Pooled: config.min_size ({}) must be <= max_size ({})",
            config.min_size,
            config.max_size,
        );

        Self::build(config, fingerprint)
    }

    /// Shared constructor body for [`new`](Self::new) / [`try_new`](Self::try_new).
    fn build(config: Config, fingerprint: u64) -> Self {
        let semaphore = Arc::new(Semaphore::new(config.max_size as usize));
        // #390: cap concurrent instance creation. `max(1)` protects us
        // from a pathological `max_concurrent_creates = 0` config that
        // would otherwise deadlock the pool on first acquire.
        let create_semaphore = Arc::new(Semaphore::new(
            (config.max_concurrent_creates as usize).max(1),
        ));
        Self {
            // Cap the idle queue at `max_size`: an idle slot beyond the
            // concurrency cap can never be leased, so it is pure waste.
            store: InstanceStore::new(Some(config.max_size as usize)),
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

    /// Advances the pool's credential-revoke counter by one.
    ///
    /// Called synchronously by the manager when a credential bound to this
    /// pool is revoked — before the revoke hook is dispatched, so by the
    /// time the hook walks the idle queue every still-live instance created
    /// against the now-revoked credential already has a `checkout_epoch`
    /// strictly behind this counter and is destroyed (never recycled or
    /// admitted) on whichever return-to-idle path it reaches. Delegates to
    /// the store, which owns the counter.
    pub fn bump_revoke_epoch(&self) {
        self.store.bump_revoke_epoch();
    }

    /// Returns the number of idle instances currently in the pool.
    pub async fn idle_count(&self) -> usize {
        self.store.len().await
    }

    /// Invokes a per-slot credential rotation hook against every idle
    /// pooled instance, in order.
    ///
    /// Used by the per-slot rotation dispatch: each idle instance is handed
    /// to `Provider::on_credential_refresh` / `on_credential_revoke` so a
    /// connection-bound pool can rebuild against the rotated credential.
    /// Checked-out instances are owned by their `ResourceGuard`; the slot
    /// cell is lock-free on `&self`, so the rotated credential is already
    /// visible to them and they re-read it on their own release/recycle
    /// path. The idle-queue lock is held across the awaited hooks so an
    /// instance cannot be checked out mid-rotation and miss the hook.
    ///
    /// `refresh = true` selects `on_credential_refresh`, `false` selects
    /// `on_credential_revoke`. The hook is called inline (not via a
    /// borrowing closure) so the per-entry `&R::Instance` never escapes the
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
    /// invariant guaranteed here (credential isolation). The lock taken
    /// here ([`InstanceStore::lock_idle`]) is the same lock
    /// `checkout`/`return_slot` take, so no checkout can interleave
    /// mid-rotation. If rotation ever moves onto a hot path, or hook
    /// latency becomes unbounded, revisit this only via a
    /// snapshot-with-epoch-reconcile design (capture the idle set under a
    /// brief lock, run hooks lock-free, then reconcile against the epoch
    /// on release) — never by simply widening the unlocked window.
    pub(crate) async fn dispatch_slot_hook_over_idle(
        &self,
        resource: &R,
        slot: &str,
        refresh: bool,
    ) -> Result<(), Error> {
        let idle = self.store.lock_idle().await;
        let mut first_err: Option<Error> = None;
        for entry in &*idle {
            let res = if refresh {
                resource
                    .on_credential_refresh(slot, &entry.slot.runtime)
                    .await
            } else {
                resource
                    .on_credential_revoke(slot, &entry.slot.runtime)
                    .await
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
        let idle = self.store.len().await;
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

// `run_maintenance` needs only `R: Provider` (eviction calls
// `Provider::destroy` and reads pool fields — no `PoolProvider`/`Clone`
// conversions), so it lives in this weak-bound block. That lets the
// `R: Provider`-only registration path drive the background maintenance
// reaper without the acquire-path topology bounds.
impl<R: Provider> Pooled<R> {
    /// Runs one maintenance cycle: evicts idle-timeout, max-lifetime,
    /// stale-fingerprint, and credential-revoked entries from the idle
    /// queue.
    ///
    /// Returns the number of entries evicted. Each evicted entry is destroyed
    /// via [`Provider::destroy`].
    ///
    /// Complexity: O(n) over the idle queue (average and worst case), bounded
    /// by the configured idle capacity (`max_size`).
    pub async fn run_maintenance(&self, resource: &R) -> usize {
        // The credential-revoke arm runs through the store's epoch fence
        // (`evict_stale`): an entry whose `checkout_epoch` is behind the live
        // counter was leased under a since-revoked credential. The
        // fingerprint / max-lifetime / idle-timeout arms run through the
        // store's `retain` over the slot's own policy fields. Both run under
        // the idle lock, atomic against checkout/return.
        let mut to_destroy = self.store.evict_stale().await;

        let current_fp = self.current_fingerprint.load(Ordering::Acquire);
        let config = &self.config;
        let now = Instant::now();
        let nonrevoke = self
            .store
            .retain(|slot, _epoch| Self::should_evict_nonrevoke(slot, config, current_fp, now))
            .await;
        to_destroy.extend(nonrevoke);

        let evicted = to_destroy.len();
        for slot in to_destroy {
            let _ = resource.destroy(slot.runtime).await;
        }

        if evicted > 0 {
            tracing::debug!(evicted, "pool maintenance: evicted idle/expired entries");
        }
        evicted
    }

    /// Whether a pool slot should be evicted for a non-revoke reason
    /// (stale fingerprint, max lifetime, idle timeout). The revoke arm is
    /// owned by the store's epoch fence ([`InstanceStore::evict_stale`]),
    /// not this predicate.
    fn should_evict_nonrevoke(
        slot: &PoolSlot<R>,
        config: &Config,
        current_fp: u64,
        now: Instant,
    ) -> bool {
        // Stale fingerprint.
        if slot.fingerprint != current_fp {
            return true;
        }
        // Max lifetime exceeded.
        if config
            .max_lifetime
            .is_some_and(|max| now.duration_since(slot.metrics.created_at) > max)
        {
            return true;
        }
        // Idle timeout exceeded.
        if let (Some(idle_timeout), Some(returned_at)) = (config.idle_timeout, slot.returned_at) {
            return now.duration_since(returned_at) > idle_timeout;
        }
        false
    }
}

impl<R> Pooled<R>
where
    R: PoolProvider + Clone + Send + Sync + 'static,
    R::Instance: Clone,
{
    /// Acquires an instance from the pool.
    ///
    /// 1. Acquire a semaphore permit (waits with timeout if pool is full).
    /// 2. Try to pop a fresh idle instance (the store's checkout fence
    ///    discards and destroys any since-revoked idle slots).
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
        let (slot, created_epoch) = match self
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
        let guard = CreateGuard::new(slot, resource.clone(), Arc::clone(release_queue));

        // Prepare the new instance.
        if let Err(e) = resource.prepare(guard.runtime(), ctx).await {
            let slot = guard.defuse();
            let _ = resource.destroy(slot.runtime).await;
            // permit drops here, returning the slot.
            return Err(e);
        }

        let slot = guard.defuse();
        // Fence: a revoke that landed while this create was in flight must
        // not be handed onward (HikariCP #1836).
        let slot = self
            .fence_freshly_created(slot, created_epoch, resource)
            .await?;
        Ok(self.build_guarded_handle(
            slot.runtime.clone(),
            slot,
            created_epoch,
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
            // The store's checkout fence pops the first FRESH slot and
            // returns every since-revoked stale slot for destruction. The
            // revoke re-check the old `try_acquire_idle` did inline
            // (`entry.revoke_epoch != current`) is now done inside
            // `InstanceStore::checkout` under the idle lock — uniform with
            // custom topologies.
            let checkout = self.store.checkout().await;
            for stale in checkout.stale {
                let _ = resource.destroy(stale.runtime).await;
            }
            let Some(checked_out) = checkout.fresh else {
                return Ok(IdleResult::Empty(permit));
            };
            let (entry, checkout_epoch) = checked_out.into_parts();

            // Cancel-safety: guard the popped entry through all async
            // validation steps. If cancelled mid-check, the guard submits
            // an async destroy via the ReleaseQueue rather than silently
            // leaking the server-side handle.
            let guard = CreateGuard::new(entry, resource.clone(), Arc::clone(release_queue));

            // Stale fingerprint — destroy silently.
            let current_fp = self.current_fingerprint.load(Ordering::Acquire);
            if guard.entry().fingerprint != current_fp {
                let slot = guard.defuse();
                let _ = resource.destroy(slot.runtime).await;
                continue;
            }

            // Max lifetime check.
            if self
                .config
                .max_lifetime
                .is_some_and(|max| guard.entry().metrics.created_at.elapsed() > max)
            {
                let slot = guard.defuse();
                let _ = resource.destroy(slot.runtime).await;
                continue;
            }

            // Broken check (sync, O(1)).
            if resource.is_broken(guard.runtime()).is_broken() {
                let slot = guard.defuse();
                let _ = resource.destroy(slot.runtime).await;
                continue;
            }

            // Optional health check on checkout.
            if self.config.test_on_checkout && resource.check(guard.runtime()).await.is_err() {
                let slot = guard.defuse();
                let _ = resource.destroy(slot.runtime).await;
                continue;
            }

            // Prepare for this execution context.
            if let Err(e) = resource.prepare(guard.runtime(), ctx).await {
                let slot = guard.defuse();
                let _ = resource.destroy(slot.runtime).await;
                return Err(e);
            }

            let mut slot = guard.defuse();
            slot.metrics.checkout_count += 1;

            return Ok(IdleResult::Found(self.build_guarded_handle(
                slot.runtime.clone(),
                slot,
                checkout_epoch,
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

    /// Creates a new pool slot via `resource.create()`.
    ///
    /// Returns the slot and the revoke-epoch snapshot taken at the **start**
    /// of creation. All creation goes through this funnel and is gated on
    /// `create_semaphore` so a burst of acquires cannot stampede a fragile
    /// backend with `max_size` parallel connects. The permit is released as
    /// soon as `Provider::create` returns.
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
    ) -> Result<(PoolSlot<R>, u64), Error> {
        // Snapshot the revoke counter *before* the create-semaphore wait
        // and `resource.create()`. An instance whose creation straddles a
        // revoke (in flight when the credential is revoked, completing
        // after) must be fenced: capturing the epoch at create-start means
        // a revoke landing during the create advances the live counter past
        // this snapshot, so every return-to-idle path destroys the instance
        // instead of admitting it (HikariCP #1836). Reading it after the
        // create returns would capture the already-bumped value and let the
        // post-revoke instance through.
        let created_epoch = self.store.stamp_epoch();
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
                Ok(Err(e)) => return Err(e),
                Err(_timeout) => {
                    return Err(Error::transient(ERR_CREATE_TIMED_OUT));
                },
            };

        Ok((
            PoolSlot {
                runtime,
                metrics: InstanceMetrics {
                    error_count: 0,
                    checkout_count: 1,
                    created_at: Instant::now(),
                },
                fingerprint: self.current_fingerprint.load(Ordering::Acquire),
                returned_at: None,
            },
            created_epoch,
        ))
    }

    /// Fences a freshly created instance before it is handed to the
    /// caller: if a credential revoke advanced the store's counter while the
    /// `create` was in flight, the instance was authenticated with the
    /// now-revoked credential and must be destroyed, not admitted (HikariCP
    /// #1836 — the in-flight-create-completing-after-revoke race that an
    /// idle-walk / evict-only approach cannot catch).
    ///
    /// `Ok(slot)` when the snapshot still matches the live counter (admit
    /// it); `Err` after destroying the instance when the counter advanced —
    /// the acquire fails closed rather than serving a revoked-credential
    /// runtime. The revoke epoch is captured at the *start* of
    /// `create_entry`, so a revoke that lands any time during the create is
    /// observed here.
    async fn fence_freshly_created(
        &self,
        slot: PoolSlot<R>,
        created_epoch: u64,
        resource: &R,
    ) -> Result<PoolSlot<R>, Error> {
        if created_epoch != self.store.current_revoke_epoch() {
            let _ = resource.destroy(slot.runtime).await;
            return Err(Error::permanent(format!(
                "{}: credential revoked while a pool instance was being \
                 created — instance destroyed, not handed to the caller",
                R::key(),
            )));
        }
        Ok(slot)
    }

    /// Builds a guarded handle with an on-release callback that submits
    /// async recycle work to the [`ReleaseQueue`].
    ///
    /// The semaphore permit is stored directly in the handle, not inside
    /// the callback closure. This ensures the permit is returned even if
    /// the callback panics.
    // Reason: `permit` must be a separate argument — it cannot live in
    // `PoolSlot` because it needs to be stored in the handle, not the
    // callback closure. Bundling into a struct would add complexity for
    // a single call site.
    #[expect(
        clippy::too_many_arguments,
        reason = "`permit` must be separate — cannot live in `PoolSlot`; bundling adds complexity for one call site"
    )]
    fn build_guarded_handle(
        &self,
        runtime: R::Instance,
        slot: PoolSlot<R>,
        checkout_epoch: u64,
        permit: OwnedSemaphorePermit,
        resource: R,
        release_queue: Arc<ReleaseQueue>,
        generation: u64,
        metrics: Option<ResourceOpsMetrics>,
    ) -> ResourceGuard<R> {
        let store = self.store.clone();
        let current_fp_ref = self.current_fingerprint.clone();
        let max_lifetime = self.config.max_lifetime;

        ResourceGuard::guarded_with_permit(
            runtime,
            R::key(),
            TopologyTag::Pool,
            generation,
            move |returned_runtime: R::Instance, tainted| {
                if let Some(m) = &metrics {
                    m.record_release();
                }

                let runtime = returned_runtime;
                // Track tainted returns in error_count so `PoolProvider::recycle`
                // implementations can make informed keep-or-drop decisions
                // based on accumulated failure history.
                let mut instance_metrics = slot.metrics.clone();
                if tainted {
                    instance_metrics.error_count += 1;
                }
                let slot = PoolSlot {
                    runtime,
                    metrics: instance_metrics,
                    fingerprint: slot.fingerprint,
                    returned_at: None, // set by release_entry on idle push
                };

                // Load fingerprint at release time (not checkout time) to detect
                // config changes that happened while the handle was checked out.
                let current_fp = current_fp_ref.load(Ordering::Acquire);
                // Return the teardown future. The guard awaits it inline on
                // `ResourceGuard::release` (surfacing the recycle/destroy
                // `Result`) or submits it to its `ReleaseQueue` on `Drop`
                // (best-effort, `Result` discarded). The store's
                // `return_slot` re-reads the revoke epoch under the idle
                // lock — that is the authoritative fence; the pre-recycle
                // `current_revoke_epoch` compare inside `release_entry` is
                // only a perf early-out.
                Box::pin(release_entry(
                    resource,
                    slot,
                    checkout_epoch,
                    tainted,
                    current_fp,
                    store,
                    max_lifetime,
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
        let (slot, created_epoch) = match self
            .create_entry(resource, resource_config, ctx, true)
            .await
        {
            Ok(e) => e,
            Err(e) => return Err(e),
        };

        // Cancel-safety guard: see analogous `acquire`-path comment
        // upstream. Submits async destroy via ReleaseQueue if cancelled.
        let guard = CreateGuard::new(slot, resource.clone(), Arc::clone(release_queue));
        if let Err(e) = resource.prepare(guard.runtime(), ctx).await {
            let slot = guard.defuse();
            let _ = resource.destroy(slot.runtime).await;
            return Err(e);
        }

        let slot = guard.defuse();
        // Fence: a revoke that landed while this create was in flight must
        // not be handed onward (HikariCP #1836).
        let slot = self
            .fence_freshly_created(slot, created_epoch, resource)
            .await?;
        Ok(self.build_guarded_handle(
            slot.runtime.clone(),
            slot,
            created_epoch,
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
    /// fenced and destroyed. The `created_epoch` is the revoke snapshot
    /// taken at the start of its `create_entry`; the store's `deposit_fresh`
    /// compares it against the live counter **under the idle lock**, making
    /// the compare-then-push atomic against the revoke idle-walk (which holds
    /// the same lock), so a warmup running concurrently with — or after — a
    /// revoke can never deposit an instance authenticated with the revoked
    /// credential.
    async fn admit_warmed_entry(
        &self,
        mut slot: PoolSlot<R>,
        created_epoch: u64,
        resource: &R,
    ) -> bool {
        slot.returned_at = Some(Instant::now());
        match self.store.deposit_fresh(slot, created_epoch).await {
            ReturnOutcome::Recycled => true,
            ReturnOutcome::Evict(slot) => {
                let _ = resource.destroy(slot.runtime).await;
                false
            },
        }
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
                Ok((slot, created_epoch)) => {
                    if self.admit_warmed_entry(slot, created_epoch, resource).await {
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
                Ok((slot, created_epoch)) => {
                    if self.admit_warmed_entry(slot, created_epoch, resource).await {
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
/// Decides whether to recycle or destroy a returned pool slot. The semaphore
/// permit is **not** held here — it was already returned when the handle
/// dropped (it lives in `GuardInner::Guarded`, not in the callback closure).
///
/// This **is** the teardown future the guard's release callback returns:
/// `ResourceGuard::release` awaits it and surfaces the `Result`; `Drop`
/// submits it to the [`ReleaseQueue`] discarding the `Result`. The returned
/// `Result` is the `destroy` outcome on every destroy arm (so an awaited
/// `release()` observes a failed destroy), `Ok(())` when the instance is
/// recycled back to idle.
///
/// # Atomicity (revoke fence)
///
/// The authoritative revoke fence is [`InstanceStore::return_slot`]: it
/// re-reads the live revoke epoch **under the idle lock** and pushes (or
/// evicts) the slot under that same lock — the exact continuous critical
/// section the old `RecycleDecision::Keep` arm performed inline. The pipeline
/// runs the pre-recycle pool policy (tainted / stale fingerprint / max-lifetime
/// / broken) and `recycle()` *first*, then hands the slot to `return_slot`
/// *last*, so a revoke landing during the (possibly parking) `recycle()` still
/// evicts on return. The pre-recycle `current_revoke_epoch` compare is a perf
/// early-out only (destroy before an expensive `recycle()`), not the fence.
async fn release_entry<R>(
    resource: R,
    slot: PoolSlot<R>,
    checkout_epoch: u64,
    tainted: bool,
    current_fp: u64,
    store: InstanceStore<PoolSlot<R>>,
    max_lifetime: Option<Duration>,
) -> Result<(), Error>
where
    R: PoolProvider + Send + Sync + 'static,
{
    // Tainted — destroy immediately.
    if tainted {
        return resource.destroy(slot.runtime).await;
    }

    // Stale fingerprint — config changed since checkout.
    if slot.fingerprint != current_fp {
        return resource.destroy(slot.runtime).await;
    }

    // Credential revoked while this handle was checked out (or while its
    // release sat queued). Perf early-out: skip an expensive `recycle()` when
    // the revoke already landed. NOT the authoritative fence — `return_slot`
    // below re-reads the epoch under the idle lock, so a revoke landing during
    // the `recycle()` park is still caught.
    if checkout_epoch != store.current_revoke_epoch() {
        return resource.destroy(slot.runtime).await;
    }

    // Max lifetime exceeded.
    if max_lifetime.is_some_and(|max| slot.metrics.created_at.elapsed() > max) {
        return resource.destroy(slot.runtime).await;
    }

    // Broken check (sync).
    if resource.is_broken(&slot.runtime).is_broken() {
        return resource.destroy(slot.runtime).await;
    }

    // Async recycle check.
    match resource.recycle(&slot.runtime, &slot.metrics).await {
        Ok(RecycleDecision::Keep) => {
            // The store's `return_slot` re-reads the revoke epoch under the
            // idle lock and pushes (or evicts) under that same lock — atomic
            // against the revoke idle-walk. A revoke that landed while
            // `recycle()` parked is caught here even though the pre-recycle
            // early-out above missed it.
            let mut slot = slot;
            slot.returned_at = Some(Instant::now());
            match store.return_slot(slot, checkout_epoch).await {
                ReturnOutcome::Recycled => Ok(()),
                ReturnOutcome::Evict(slot) => resource.destroy(slot.runtime).await,
            }
        },
        Ok(RecycleDecision::Drop) | Err(_) => resource.destroy(slot.runtime).await,
    }
}

/// Cancel-safety guard for the create-then-prepare sequence.
///
/// Wraps a [`PoolSlot`] between creation and handle construction. If
/// the future is cancelled (e.g. via `tokio::select!` or timeout) after
/// `create()` succeeds but before the handle is built, `Drop` submits
/// the freshly-built runtime to the [`ReleaseQueue`] for an async
/// `Provider::destroy` — symmetric with the release-path's
/// `release_entry`. Without this, the runtime's *sync* `Drop` would run
/// inline (closing the local handle) but the server-side resource — DB
/// session, broker subscription, OS-level handle — would leak.
///
/// Call [`defuse`](Self::defuse) to take ownership of the slot once
/// the handle is safely constructed. `defuse` consumes the guard by
/// value, so the borrow checker prevents any use of the guard after
/// `defuse` — `entry()` / `runtime()` cannot be invoked on a defused
/// guard, and the guard's `Drop` never runs against a defused slot.
///
/// `slot` is the only `Option` field; `resource` and `release_queue`
/// stay populated for the guard's whole lifetime so the `Drop` impl
/// never has to inspect `Option` invariants — it clones them (both
/// cheap: `R: Clone` is required by `release_entry` already, and
/// `Arc<ReleaseQueue>` is a refcount bump) into the queued destroy
/// closure. `Drop` therefore carries no `unwrap` / `unreachable!` /
/// `expect` paths — material under cancellation.
struct CreateGuard<R>
where
    R: PoolProvider + Clone + Send + Sync + 'static,
{
    /// `None` after [`defuse`](Self::defuse) took it out; `Some(_)`
    /// for any guard a caller can still observe. `Drop` short-circuits
    /// on `None`.
    slot: Option<PoolSlot<R>>,
    /// Cloned resource handle so the Drop path can call
    /// `Provider::destroy(slot.runtime)` from the [`ReleaseQueue`]
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
    R: PoolProvider + Clone + Send + Sync + 'static,
{
    /// Creates a new guard wrapping the given pool slot.
    fn new(slot: PoolSlot<R>, resource: R, release_queue: Arc<ReleaseQueue>) -> Self {
        Self {
            slot: Some(slot),
            resource,
            release_queue,
        }
    }

    /// Returns a reference to the inner slot for inspection.
    fn entry(&self) -> &PoolSlot<R> {
        // guard-justified: `entry()` is private + only called between
        // `new` (which inserts `Some(_)`) and `defuse` (which consumes
        // `self`); reaching here with `None` would mean `entry()` was
        // called *after* `defuse`, which the by-value consumption in
        // `defuse(mut self)` makes a borrow-checker error, not a
        // runtime path.
        self.slot
            .as_ref()
            .unwrap_or_else(|| unreachable!("CreateGuard accessed after defuse"))
    }

    /// Returns a reference to the runtime for use in `prepare()`.
    fn runtime(&self) -> &R::Instance {
        &self.entry().runtime
    }

    /// Consumes the guard and returns the wrapped slot.
    ///
    /// After this call, the guard is gone; its `Drop` runs against
    /// `slot: None` and short-circuits without submitting a destroy.
    fn defuse(mut self) -> PoolSlot<R> {
        // guard-justified: `defuse` consumes `self` by value, so the
        // borrow checker forbids calling it twice on the same guard.
        // `self.slot` is `Some(_)` for the whole observable lifetime
        // of the guard (set in `new`, only mutated here or in `Drop`,
        // both of which consume the guard), so `take()` cannot return
        // `None` on this path.
        self.slot
            .take()
            .unwrap_or_else(|| unreachable!("CreateGuard defused twice"))
    }
}

impl<R> Drop for CreateGuard<R>
where
    R: PoolProvider + Clone + Send + Sync + 'static,
{
    fn drop(&mut self) {
        let Some(slot) = self.slot.take() else {
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
                let _ = resource.destroy(slot.runtime).await;
            })
        });
    }
}

// ─── Topology impl for Pooled ────────────────────────────────────────────────
//
// `Pooled<R>` is the framework pool topology: it drives the full idle-queue
// acquire pipeline (checkout/create/prepare/recycle) over its own
// `InstanceStore<PoolSlot<R>>` and holds the resource handle so it can call
// the `Provider` / `PoolProvider` hooks itself. `type Slot = ()` because the
// store the open contract hands to `Topology` methods is a *throwaway* sentinel
// for the open trait — `Pooled` ignores it and uses its own internal store,
// the same way a permit-only custom topology ignores its store. The lease the
// open pipeline produces carries only the permit; the real instance lifecycle
// runs through the inherent `Pooled::acquire` the manager pipeline calls.

use async_trait::async_trait;

use crate::topology::{AdmissionPhase, Lease, Load, Ticket, Topology, Unavailable};

#[async_trait]
impl<R> Topology for Pooled<R>
where
    R: Provider
        + PoolProvider
        + crate::resource::HasCredentialSlots
        + Clone
        + Send
        + Sync
        + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    /// Permit-only slot for the open contract: the pool manages instances
    /// internally in its own `InstanceStore`.
    type Slot = ();

    /// Non-blocking concurrency gate. Returns a permit-bearing [`Ticket`] if a
    /// semaphore slot is available, or [`Unavailable::Saturated`] when the pool
    /// is at max capacity.
    fn try_reserve(&self, _store: &InstanceStore<()>) -> Result<Ticket<()>, Unavailable> {
        self.semaphore
            .clone()
            .try_acquire_owned()
            .map(Ticket::permit)
            .map_err(|_| Unavailable::Saturated { retry_after: None })
    }

    /// Consumes the ticket and returns a lease wrapping the held permit.
    ///
    /// The full idle-queue acquire (checkout / create / prepare) runs through
    /// the inherent [`Pooled::acquire`] the framework manager pipeline calls;
    /// this open-contract method satisfies the [`Topology`] trait so the pool
    /// and custom topologies are uniformly expressible.
    async fn acquire(
        &self,
        ticket: Ticket<()>,
        _store: &InstanceStore<()>,
    ) -> Result<Lease<()>, Error> {
        let (_, permit) = ticket.take_slot();
        Ok(Lease::new((), 0, permit))
    }

    /// Advisory admission phase. `Saturated` when no permits are available.
    fn phase(&self, _store: &InstanceStore<()>) -> AdmissionPhase {
        if self.semaphore.available_permits() == 0 {
            AdmissionPhase::Saturated
        } else {
            AdmissionPhase::Ready
        }
    }

    /// Advisory load snapshot: `saturation = in_use / capacity`.
    fn load(&self, _store: &InstanceStore<()>) -> Option<Load> {
        let available = self.semaphore.available_permits();
        let capacity = self.config.max_size as usize;
        let used = capacity.saturating_sub(available);
        Some(Load::permits(used, capacity))
    }

    fn tag(&self) -> TopologyTag {
        TopologyTag::Pool
    }
}

// ─── TopologyDispatch bridge for Pooled ──────────────────────────────────────
//
// The framework manager pipeline reaches `Pooled<R>` through this bridge: the
// typed `acquire_guard` runs the full idle-queue pipeline (the inherent
// `Pooled::acquire`), and the rotation / maintenance / revoke-fence operations
// drive the resource handle the manager owns.

use crate::runtime::managed::{MaintenanceSchedule, TopologyDispatch};

#[async_trait]
impl<R> TopologyDispatch<R> for Pooled<R>
where
    R: Provider<Topology = Pooled<R>>
        + PoolProvider
        + crate::resource::HasCredentialSlots
        + Clone
        + Send
        + Sync
        + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    async fn acquire_guard(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        options: &AcquireOptions,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Result<ResourceGuard<R>, Error> {
        self.acquire(
            resource,
            config,
            ctx,
            release_queue,
            generation,
            options,
            metrics,
        )
        .await
    }

    async fn warmup(&self, resource: &R, config: &R::Config, ctx: &ResourceContext) -> usize {
        Pooled::warmup(self, resource, config, ctx).await
    }

    async fn run_maintenance(&self, resource: &R) -> usize {
        Pooled::run_maintenance(self, resource).await
    }

    fn maintenance_schedule(&self) -> Option<MaintenanceSchedule> {
        Some(MaintenanceSchedule {
            idle_timeout: self.config.idle_timeout,
            max_lifetime: self.config.max_lifetime,
            maintenance_interval: self.config.maintenance_interval,
        })
    }

    fn bump_revoke_epoch(&self) {
        Pooled::bump_revoke_epoch(self);
    }

    fn set_fingerprint(&self, fingerprint: u64) {
        Pooled::set_fingerprint(self, fingerprint);
    }

    async fn dispatch_credential_hook(
        &self,
        resource: &R,
        slot: &str,
        refresh: bool,
    ) -> Result<(), Error> {
        self.dispatch_slot_hook_over_idle(resource, slot, refresh)
            .await
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
        resource::{HasCredentialSlots, ResourceConfig, ResourceMetadata},
        topology::pooled::BrokenCheck,
    };

    // -- Mock resource implementing PoolProvider --

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

    #[derive(Clone)]
    struct PoolTestConfig;

    nebula_schema::impl_empty_has_schema!(PoolTestConfig);

    impl ResourceConfig for PoolTestConfig {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }

        fn fingerprint(&self) -> u64 {
            // Unit struct: all instances identical — constant 0 is correct.
            0
        }
    }

    #[async_trait::async_trait]
    impl Provider for MockPool {
        type Config = PoolTestConfig;
        type Instance = u32;
        type Topology = Pooled<Self>;

        fn key() -> ResourceKey {
            resource_key!("mock-pool")
        }

        async fn create(
            &self,
            _config: &PoolTestConfig,
            _ctx: &ResourceContext,
        ) -> Result<u32, Error> {
            if self.fail_create.load(Ordering::Relaxed) {
                Err(Error::transient("create failed"))
            } else {
                Ok(1)
            }
        }

        async fn check(&self, _runtime: &u32) -> Result<(), Error> {
            if self.fail_check.load(Ordering::Relaxed) {
                Err(Error::transient("check failed"))
            } else {
                Ok(())
            }
        }

        async fn destroy(&self, _runtime: u32) -> Result<(), Error> {
            self.destroy_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl HasCredentialSlots for MockPool {
        fn credential_slot_epoch(&self) -> u64 {
            0
        }
    }

    impl PoolProvider for MockPool {
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

    fn mock_pool(config: Config, fingerprint: u64) -> Pooled<MockPool> {
        Pooled::<MockPool>::new(config, fingerprint)
    }

    #[tokio::test]
    async fn acquire_creates_new_instance_when_idle_empty() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 2,
            ..Config::default()
        };
        let pool = mock_pool(config, 1);
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
        let pool = mock_pool(config, 1);
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
        let pool = mock_pool(config, 1);
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
        let pool = mock_pool(config, 1);
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
        let pool = mock_pool(config, 1);
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
        let pool = mock_pool(config, 1);
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
    // An invalid `(min_size, max_size)` from operator/JSON config must fail
    // the registration path as a typed `Error::permanent`, never abort the
    // process. `Pooled::new`'s assert is the compile-time-known counterpart
    // and is exercised by the const-shaped `new(...)` calls throughout the
    // rest of this module.

    #[test]
    fn try_new_rejects_max_size_zero_with_permanent_error() {
        let config = Config {
            min_size: 0,
            max_size: 0,
            ..Config::default()
        };
        let err = match Pooled::<MockPool>::try_new(config, 1) {
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
        let err = match Pooled::<MockPool>::try_new(config, 1) {
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
        let pool = Pooled::<MockPool>::try_new(config, 1)
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

    /// Helper: deposits a fake idle slot with custom timestamps at the
    /// store's current revoke epoch (so the revoke arm is inert for the
    /// fingerprint / lifetime / timeout maintenance tests).
    async fn push_idle_entry(
        pool: &Pooled<MockPool>,
        created_at: Instant,
        returned_at: Option<Instant>,
        fingerprint: u64,
    ) {
        let slot = PoolSlot {
            runtime: 42,
            metrics: InstanceMetrics {
                error_count: 0,
                checkout_count: 1,
                created_at,
            },
            fingerprint,
            returned_at,
        };
        let epoch = pool.store.stamp_epoch();
        let outcome = pool.store.deposit_fresh(slot, epoch).await;
        assert!(
            !outcome.is_evict(),
            "test idle slot deposit must not be fenced at the live epoch"
        );
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
        let pool = mock_pool(config, 1);

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
        let pool = mock_pool(config, 1);

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
        let pool = mock_pool(config, 1);

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
        let pool = mock_pool(config, 1);

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
        let pool = mock_pool(config, 1);

        let evicted = pool.run_maintenance(&resource).await;
        assert_eq!(evicted, 0);
    }

    /// A non-stale, non-timed-out idle entry whose credential was revoked
    /// must be evicted by the store's epoch fence (`evict_stale`) — not
    /// kept. With idle/lifetime disabled and a current fingerprint, the
    /// revoke arm is the *only* arm that can remove it, so this isolates it.
    #[tokio::test]
    async fn maintenance_evicts_revoked_idle_entry() {
        let resource = MockPool::new();
        let config = Config {
            max_size: 5,
            idle_timeout: None,
            max_lifetime: None,
            ..Config::default()
        };
        let pool = mock_pool(config, 1);

        // Two healthy, current-fingerprint, non-timed-out idle entries.
        push_idle_entry(&pool, Instant::now(), Some(Instant::now()), 1).await;
        push_idle_entry(&pool, Instant::now(), Some(Instant::now()), 1).await;

        // No revoke yet → the revoke arm is inert, nothing is evicted.
        assert_eq!(pool.run_maintenance(&resource).await, 0);
        assert_eq!(pool.idle_count().await, 2);

        // Revoke: bump the store counter. Both entries were stamped with the
        // pre-bump value, so their checkout epoch is now strictly behind.
        pool.bump_revoke_epoch();

        let evicted = pool.run_maintenance(&resource).await;
        assert_eq!(
            evicted, 2,
            "the store's revoke-epoch fence must evict every idle entry \
             whose checkout epoch is behind the bumped counter, even though \
             fingerprint/lifetime/timeout would all keep them"
        );
        assert_eq!(pool.idle_count().await, 0);
    }

    /// Regression: a `CreateGuard` whose slot is still `Some` when it drops
    /// must schedule `Provider::destroy` via the `ReleaseQueue` instead of
    /// just letting the runtime's sync `Drop` run.
    #[tokio::test]
    async fn create_guard_drop_submits_destroy_via_release_queue() {
        let resource = MockPool::new();
        let destroy_count = Arc::clone(&resource.destroy_count);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);

        // Fabricate a freshly-built slot (mirrors what `create_entry`
        // returns just before `CreateGuard::new`). The exact field
        // values do not matter — only that `Drop` consumes the runtime.
        let slot = PoolSlot::<MockPool> {
            runtime: 42u32,
            metrics: InstanceMetrics {
                error_count: 0,
                checkout_count: 1,
                created_at: Instant::now(),
            },
            fingerprint: 1,
            returned_at: None,
        };
        let guard = CreateGuard::new(slot, resource.clone(), Arc::clone(&rq));

        // Simulate a cancelled acquire: the future is dropped *before*
        // `defuse` ran.
        drop(guard);

        tokio::time::sleep(Duration::from_millis(50)).await;

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

    /// Companion to the cancel-drop regression: a `CreateGuard` that runs
    /// through `defuse` normally (the success path) MUST NOT trigger a stray
    /// `Provider::destroy`.
    #[tokio::test]
    async fn create_guard_defuse_skips_destroy() {
        let resource = MockPool::new();
        let destroy_count = Arc::clone(&resource.destroy_count);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);

        let slot = PoolSlot::<MockPool> {
            runtime: 7u32,
            metrics: InstanceMetrics {
                error_count: 0,
                checkout_count: 1,
                created_at: Instant::now(),
            },
            fingerprint: 1,
            returned_at: None,
        };
        let guard = CreateGuard::new(slot, resource.clone(), Arc::clone(&rq));
        let _slot = guard.defuse(); // success path consumes the guard

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
