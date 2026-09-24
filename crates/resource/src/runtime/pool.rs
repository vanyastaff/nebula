//! Pool topology — manages a pool of N interchangeable resource instances.
//!
//! `Pooled<R>` is the built-in framework pool topology. It supplies the
//! **entry-centric** [`Topology<R>`] hooks the framework acquire loop drives —
//! `create_entry` (create a `PoolEntry<R>`), `accept` (post-checkout validation),
//! `prepare` (per-checkout session init), `on_release` (recycle decision),
//! `idle_evictable` (maintenance predicate), and the rotation / fingerprint /
//! admission surface. **It owns no idle store and runs no checkout / fence /
//! destroy loop** — those live in the framework
//! ([`ManagedResource::run_acquire_loop`](crate::runtime::managed::ManagedResource)),
//! over the framework-owned [`InstanceStore<PoolEntry<R>>`]. The revoke-epoch
//! fence is therefore framework-owned for the pool exactly as it is for a custom
//! topology — no author/pool discipline involved.
//!
//! [`Topology<R>`]: crate::topology::Topology

use std::{
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use tokio::sync::Semaphore;

use crate::{
    context::ResourceContext,
    error::Error,
    resource::Provider,
    topology::{
        AdmissionPhase, Load, MaintenanceSchedule, Ticket, Topology, Unavailable,
        pooled::{InstanceMetrics, PoolProvider, RecycleDecision, config::Config},
        store::{InstanceStore, StoreView},
    },
    topology_tag::TopologyTag,
};

// ─── Static error messages ───────────────────────────────────────────────────

/// Pool cannot operate with zero max size.
const ERR_MAX_SIZE_ZERO: &str = "Pooled: config.max_size must be > 0 (got 0 — would \
     deadlock the checkout semaphore on first acquire)";

/// Pool cannot operate with a zero create budget.
const ERR_CREATE_TIMEOUT_ZERO: &str = "Pooled: config.create_timeout must be positive (got 0 — \
     every create would time out immediately)";

/// The create-semaphore was closed (pool is shutting down).
const ERR_CREATE_SEMAPHORE_CLOSED: &str = "pool: create semaphore closed";

/// Timed out waiting for a create-semaphore permit.
const ERR_CREATE_SEMAPHORE_TIMEOUT: &str =
    "pool: create timed out waiting for create-semaphore permit";

/// The `resource.create()` call exceeded `create_timeout`.
const ERR_CREATE_TIMED_OUT: &str = "pool: create timed out";

/// Max-lifetime jitter spread: each entry's own eviction threshold is
/// drawn once, at creation, from `[0.95 * max_lifetime, max_lifetime]`.
///
/// This is HikariCP's `maxLifetime` attenuation band, deliberately much
/// narrower than the recovery gate's `[nominal/2, nominal]` equal-jitter
/// (`EQUAL_JITTER_SPREAD` in `recovery::gate`): a backoff retry has no
/// meaningful "too early" cost, but an idle-pool entry evicted much earlier
/// than its configured lifetime wastes a healthy connection and forces an
/// avoidable reconnect. A small attenuation is enough to de-synchronize a
/// warmup cohort (they only need to *not* all expire on the exact same
/// tick) without meaningfully shortening the effective pool lifetime.
const MAX_LIFETIME_JITTER_SPREAD: f64 = 0.05;

// ─────────────────────────────────────────────────────────────────────────────

/// A single pooled instance with its metrics and config fingerprint — the
/// [`Topology::Entry`] for [`Pooled`].
///
/// The framework holds this entry for the whole lease (the guard owns it via the
/// release closure), so `metrics.created_at` survives checkout → lease → return:
/// max-lifetime eviction keeps firing because the entry's `created_at` is never
/// rebuilt from a bare instance.
///
/// The semaphore permit does **not** live here — it is held in the
/// [`ResourceGuard`](crate::guard::ResourceGuard) so it is returned even if the
/// release callback panics. The credential-revoke snapshot is not a field
/// either: it lives in the framework store's `checkout_epoch`.
pub struct PoolEntry<R: Provider> {
    instance: R::Instance,
    metrics: InstanceMetrics,
    fingerprint: u64,
    /// When this entry was last returned to the idle queue.
    /// `None` for freshly created entries that have never been idle.
    returned_at: Option<Instant>,
    /// This entry's own max-lifetime threshold, computed **once** at
    /// creation via [`apply_jitter`](crate::jitter::apply_jitter) over
    /// `config.max_lifetime` — HikariCP-style attenuation so a warmup burst
    /// of entries created in the same instant does not all reach
    /// `max_lifetime` on the same maintenance tick. `None` when
    /// `max_lifetime` is unset (unaffected). Stable for the entry's whole
    /// lifetime: recomputing jitter on every reaper tick would make
    /// eviction timing flap non-deterministically between ticks for the
    /// same entry instead of converging once.
    jittered_max_lifetime: Option<Duration>,
}

impl<R: Provider> std::fmt::Debug for PoolEntry<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolEntry")
            .field("fingerprint", &self.fingerprint)
            .field("checkout_count", &self.metrics.checkout_count)
            .field("returned_at", &self.returned_at)
            .finish()
    }
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
#[non_exhaustive]
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
/// checkout/recycle/destroy over a framework-owned idle store.
///
/// `Pooled<R>` holds only the concurrency semaphore, the create-concurrency cap,
/// the immutable pool [`Config`], and the live config fingerprint. The idle
/// queue is the framework's [`InstanceStore<PoolEntry<R>>`]
/// (`ManagedResource::store`); the framework runs every checkout / return /
/// evict against it. The pool implements the entry-centric [`Topology<R>`]
/// hooks the framework loop calls — it never touches the store directly except
/// through the rotation fan-out's `lock_idle` (which the framework grants it
/// transiently; the author can never name it).
///
/// [`Topology<R>`]: crate::topology::Topology
pub struct Pooled<R: Provider> {
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
    /// `Pooled<R>` is keyed to its resource through the `Topology<R>` impl and
    /// `PoolEntry<R>` entry type, but holds no `R`-typed field directly (the
    /// resource lives in `ManagedResource`). `fn() -> R` keeps `Pooled<R>`
    /// covariant + `Send + Sync` regardless of `R`'s own auto-traits.
    _marker: PhantomData<fn() -> R>,
}

impl<R: Provider> std::fmt::Debug for Pooled<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pooled")
            .field("config", &self.config)
            .field(
                "fingerprint",
                &self.current_fingerprint.load(Ordering::Relaxed),
            )
            .field("available_permits", &self.semaphore.available_permits())
            .finish_non_exhaustive()
    }
}

impl<R: Provider> Pooled<R> {
    /// Fallibly creates a new pool topology, returning a typed
    /// [`Error::permanent`] instead of aborting on an invalid
    /// `(min_size, max_size)` topology.
    ///
    /// This is the constructor the **registration path must use**. A
    /// `Pooled<R>` built from operator-/JSON-supplied config (the engine
    /// activation registrar feeding [`Manager::register`](crate::Manager::register) /
    /// [`ResourceFactory::register`](crate::ResourceFactory::register)) flows
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
        if config.create_timeout.is_zero() {
            // A zero budget times out every create before it can start.
            return Err(Error::permanent(ERR_CREATE_TIMEOUT_ZERO));
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
            semaphore,
            create_semaphore,
            config,
            current_fingerprint: Arc::new(AtomicU64::new(fingerprint)),
            _marker: PhantomData,
        }
    }

    /// Returns the current pool configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns a snapshot of current pool utilization over the framework store.
    ///
    /// `idle` is read from the framework store; `available_permits` is read
    /// atomically from the semaphore. Both reads are best-effort and may be
    /// slightly inconsistent in high-concurrency scenarios.
    pub async fn stats(&self, store: &InstanceStore<PoolEntry<R>>) -> PoolStats {
        let idle = store.len().await;
        let available_permits = self.semaphore.available_permits();
        let in_use = (self.config.max_size as usize).saturating_sub(available_permits);
        PoolStats {
            idle,
            capacity: self.config.max_size,
            available_permits,
            in_use,
        }
    }

    /// Whether `entry` has exceeded its max-lifetime deadline.
    ///
    /// Compares against the entry's own **jittered** threshold
    /// ([`PoolEntry::jittered_max_lifetime`], stamped once at creation via
    /// [`MAX_LIFETIME_JITTER_SPREAD`]), falling back to the raw
    /// `config.max_lifetime` only when no jittered value was stamped
    /// (defensive — `jittered_max_lifetime` is `None` exactly when
    /// `max_lifetime` itself is unset, so the two branches agree there is no
    /// deadline in that case).
    ///
    /// The single chokepoint every max-lifetime comparison in this module
    /// goes through: `idle_evictable`/`should_evict_nonrevoke` (the reaper),
    /// `accept` (post-checkout), and `on_release` (pre-recycle) all call
    /// this instead of separately comparing `created_at.elapsed()` against
    /// the raw config value — before this, only the reaper path used the
    /// jittered deadline, so an entry already past its own jittered TTL
    /// (but not yet past the unjittered `max_lifetime`) could still be
    /// checked out or recycled instead of evicted, defeating the jitter's
    /// de-synchronization purpose for exactly the entries it was supposed to
    /// spread out.
    fn exceeded_max_lifetime(&self, entry: &PoolEntry<R>) -> bool {
        let deadline = entry.jittered_max_lifetime.or(self.config.max_lifetime);
        deadline.is_some_and(|max| entry.metrics.created_at.elapsed() > max)
    }

    /// Whether a pool entry should be evicted for a non-revoke reason (stale
    /// fingerprint, max lifetime, idle timeout). The revoke arm is owned by the
    /// framework store's epoch fence, not this predicate.
    fn should_evict_nonrevoke(&self, entry: &PoolEntry<R>) -> bool {
        let current_fp = self.current_fingerprint.load(Ordering::Acquire);
        // Stale fingerprint.
        if entry.fingerprint != current_fp {
            return true;
        }
        // Max lifetime exceeded — see `exceeded_max_lifetime`.
        if self.exceeded_max_lifetime(entry) {
            return true;
        }
        // Idle timeout exceeded.
        if let (Some(idle_timeout), Some(returned_at)) =
            (self.config.idle_timeout, entry.returned_at)
        {
            return Instant::now().duration_since(returned_at) > idle_timeout;
        }
        false
    }
}

impl<R> Pooled<R>
where
    R: PoolProvider + Clone + Send + Sync + 'static,
{
    /// Creates a new pool entry via `resource.create()`.
    ///
    /// All creation goes through this funnel and is gated on `create_semaphore`
    /// so a burst of acquires cannot stampede a fragile backend with `max_size`
    /// parallel connects. The permit is released as soon as `Provider::create`
    /// returns. The whole path — permit wait + `resource.create` — shares a
    /// single `create_timeout` budget so a slow-creating backend cannot stall
    /// callers forever.
    ///
    /// The framework loop wraps the returned entry in its cancel-safety guard and
    /// fences a fresh-create that straddled a revoke on the store-return path
    /// (the store stamps the live epoch under the idle lock).
    ///
    /// # Errors
    ///
    /// - [`Error::backpressure`] when the create-semaphore wait times out.
    /// - [`Error::permanent`] when the create-semaphore is closed.
    /// - [`Error::transient`] when `Provider::create` itself times out.
    /// - Propagates the `Provider::create` error otherwise.
    async fn create_pool_entry(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
    ) -> Result<PoolEntry<R>, Error> {
        // `create_timeout` is operator-supplied; a huge value means "no
        // practical deadline", not a panic on `Instant` overflow.
        let deadline = crate::deadline::deadline_after(
            Instant::now(),
            self.config.create_timeout,
            crate::deadline::UNBOUNDED_HORIZON,
        );

        let _create_permit = match tokio::time::timeout_at(
            deadline.into(),
            self.create_semaphore.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_closed)) => return Err(Error::permanent(ERR_CREATE_SEMAPHORE_CLOSED)),
            Err(_timeout) => return Err(Error::backpressure(ERR_CREATE_SEMAPHORE_TIMEOUT)),
        };

        // Use `timeout_at` with the same absolute deadline so the budget is
        // shared: a long permit wait shortens the time available to
        // `resource.create`.
        let instance =
            match tokio::time::timeout_at(deadline.into(), resource.create(config, ctx)).await {
                Ok(Ok(rt)) => rt,
                Ok(Err(e)) => return Err(e),
                Err(_timeout) => return Err(Error::transient(ERR_CREATE_TIMED_OUT)),
            };

        Ok(PoolEntry {
            instance,
            metrics: InstanceMetrics {
                checkout_count: 1,
                created_at: Instant::now(),
            },
            fingerprint: self.current_fingerprint.load(Ordering::Acquire),
            returned_at: None,
            jittered_max_lifetime: self
                .config
                .max_lifetime
                .map(|max| crate::jitter::apply_jitter(max, MAX_LIFETIME_JITTER_SPREAD)),
        })
    }
}

// ─── Topology impl for Pooled ────────────────────────────────────────────────
//
// `Pooled<R>` supplies the entry-centric hooks the framework acquire loop drives
// over the framework-owned `InstanceStore<PoolEntry<R>>`. The pool owns no store,
// runs no checkout/destroy/fence loop, and never compares epochs — that is all
// the framework's job.

impl<R> Topology<R> for Pooled<R>
where
    R: Provider<Topology = Pooled<R>> + PoolProvider + Clone + Send + Sync + 'static,
{
    type Entry = PoolEntry<R>;

    fn try_reserve(&self, _store: StoreView<'_, PoolEntry<R>>) -> Result<Ticket, Unavailable> {
        self.semaphore
            .clone()
            .try_acquire_owned()
            .map(Ticket::permit)
            .map_err(|_| Unavailable::Saturated { retry_after: None })
    }

    async fn create_entry(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
        _retained: &crate::RetainedStore<Self::Entry>,
    ) -> Result<crate::topology::CreatedEntry<PoolEntry<R>>, Error> {
        self.create_pool_entry(resource, config, ctx)
            .await
            .map(crate::topology::CreatedEntry::new)
    }

    fn entry_instance<'s>(&self, entry: &'s PoolEntry<R>) -> &'s R::Instance {
        &entry.instance
    }

    fn into_owned_instance(&self, entry: PoolEntry<R>) -> Option<R::Instance> {
        Some(entry.instance)
    }

    async fn accept(&self, entry: &mut PoolEntry<R>, resource: &R, _ctx: &ResourceContext) -> bool {
        // Post-checkout validation: stale fingerprint / max lifetime / broken /
        // optional health check. `false` ⇒ the framework destroys this entry and
        // loops to the next idle entry, then create.
        let current_fp = self.current_fingerprint.load(Ordering::Acquire);
        if entry.fingerprint != current_fp {
            return false;
        }
        if self.exceeded_max_lifetime(entry) {
            return false;
        }
        if resource.is_broken(&entry.instance).is_broken() {
            return false;
        }
        if self.config.test_on_checkout && resource.check(&entry.instance).await.is_err() {
            return false;
        }
        entry.metrics.checkout_count += 1;
        true
    }

    async fn prepare(
        &self,
        entry: &mut PoolEntry<R>,
        resource: &R,
        ctx: &ResourceContext,
    ) -> Result<(), Error> {
        resource.prepare(&entry.instance, ctx).await
    }

    async fn on_release(&self, entry: &mut PoolEntry<R>, resource: &R) -> Result<bool, Error> {
        // Recycle decision (the framework already destroyed a *tainted* lease
        // before calling this, and runs the revoke-epoch fence on `return_entry`
        // AFTER this returns `true`).
        let current_fp = self.current_fingerprint.load(Ordering::Acquire);
        if entry.fingerprint != current_fp {
            return Ok(false);
        }
        if self.exceeded_max_lifetime(entry) {
            return Ok(false);
        }
        if resource.is_broken(&entry.instance).is_broken() {
            return Ok(false);
        }
        match resource.recycle(&entry.instance, &entry.metrics).await {
            Ok(RecycleDecision::Keep) => {
                // Stamp the return time so idle-timeout can fire on the next
                // sweep.
                entry.returned_at = Some(Instant::now());
                Ok(true)
            },
            Ok(RecycleDecision::Drop) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn pools(&self) -> bool {
        true
    }

    fn store_capacity(&self) -> Option<usize> {
        // Cap the idle queue at `max_size`: an idle entry beyond the concurrency
        // cap can never be leased, so it is pure waste.
        Some(self.config.max_size as usize)
    }

    fn queue_strategy(&self) -> crate::topology::store::PoolStrategy {
        self.config.strategy
    }

    fn warmup_target(&self, _config: &R::Config) -> usize {
        self.config.min_size as usize
    }

    fn warmup_strategy(&self) -> crate::topology::pooled::config::WarmupStrategy {
        self.config.warmup
    }

    fn idle_evictable(&self, entry: &PoolEntry<R>) -> bool {
        self.should_evict_nonrevoke(entry)
    }

    fn maintenance_schedule(&self) -> Option<MaintenanceSchedule> {
        Some(MaintenanceSchedule {
            idle_timeout: self.config.idle_timeout,
            max_lifetime: self.config.max_lifetime,
            maintenance_interval: self.config.maintenance_interval,
        })
    }

    async fn dispatch_credential_hook(
        &self,
        resource: &R,
        store: StoreView<'_, PoolEntry<R>>,
        _retained: &crate::RetainedStore<Self::Entry>,
        slot: &str,
        refresh: bool,
    ) -> Result<(), crate::topology::HookFault> {
        // Walk the framework idle store under its lock so no checkout / return
        // can interleave mid-rotation — the same lock `checkout` / `return_entry`
        // take. The store reference is granted by the framework dispatcher; the
        // author never names it.
        //
        // Tradeoff: because the idle lock spans every entry's hook `.await`, a
        // slow hook blocks concurrent idle checkouts for the full rotation
        // duration (head-of-line blocking). New-entry creation is unaffected.
        // This is tolerated because rotation is rare (not a hot path). Do not
        // "optimize" by dropping and reacquiring the lock between entries: that
        // reopens the window for an instance to be checked out mid-rotation and
        // miss its hook (credential isolation).
        //
        // Two-tier hook ceiling: each entry's hook is individually bounded by
        // `DEFAULT_AUTHOR_HOOK_CEILING` below (the inner tier, timing only the
        // hook body — the idle lock is already held by the time it starts).
        // `Manager::refresh_slot` / `drain_and_revoke` additionally wrap the
        // *whole* fan-out (every entry) in a looser framework-owned outer
        // backstop (`MAX_ROTATION_DISPATCH_CEILING`); a fully-hung pool's
        // rotation is therefore bounded overall, but an entry count large
        // enough to exceed the outer backstop at the per-entry ceiling means
        // not every idle instance is guaranteed a hook attempt — accepted
        // (rotation is rare, and a hung per-entry hook is itself the
        // pathological case this bounds).
        let idle = store.read_idle().await;
        let mut first_fault: Option<crate::topology::HookFault> = None;
        let hook_op = if refresh {
            "on_credential_refresh"
        } else {
            "on_credential_revoke"
        };
        for entry in idle.iter() {
            // Bound + isolate this one entry's hook.
            //
            // SAFETY (unwind): `idle` (the idle-lock `MutexGuard`) is a local
            // of this function, not captured by the wrapped future below
            // (which only borrows `item.entry.instance`); a caught panic
            // never unwinds past this loop iteration — `catch_unwind` stops
            // at its own boundary and the loop continues normally, so the
            // lock still releases via ordinary `Drop` when this function
            // returns.
            let result = match crate::hook_guard::guard_author_hook(
                crate::hook_guard::DEFAULT_AUTHOR_HOOK_CEILING,
                async {
                    if refresh {
                        resource.on_credential_refresh(slot, &entry.instance).await
                    } else {
                        resource.on_credential_revoke(slot, &entry.instance).await
                    }
                },
            )
            .await
            {
                Ok(result) => result.map_err(crate::topology::HookFault::Failed),
                Err(fault) => {
                    fault.observe(&R::key(), "rotation");
                    Err(match fault {
                        crate::hook_guard::HookFault::Panicked => {
                            crate::topology::HookFault::Failed(Error::permanent(format!(
                                "pool {hook_op} hook panicked for an idle instance — caught \
                                 and isolated under panic=unwind (fan-out not crashed); \
                                 inert under panic=abort"
                            )))
                        },
                        crate::hook_guard::HookFault::TimedOut => {
                            crate::topology::HookFault::TimedOut
                        },
                    })
                },
            };
            if let Err(fault) = result {
                // Surface EVERY per-entry hook failure. Returning only the
                // first would silently hide a partial rotation where some
                // idle instances refreshed and others did not, leaving them
                // in an uncertain credential state. The hook error is the
                // author's, already redacted (never credential material).
                match &fault {
                    crate::topology::HookFault::Failed(error) => tracing::warn!(
                        resource = %R::key(),
                        slot,
                        refresh,
                        %error,
                        "credential rotation hook failed for an idle pool instance"
                    ),
                    crate::topology::HookFault::TimedOut => tracing::warn!(
                        resource = %R::key(),
                        slot,
                        refresh,
                        "credential rotation hook timed out for an idle pool instance"
                    ),
                }
                if first_fault.is_none() {
                    first_fault = Some(fault);
                }
            }
        }
        match first_fault {
            Some(fault) => Err(fault),
            None => Ok(()),
        }
    }

    fn set_fingerprint(&self, fingerprint: u64) {
        self.current_fingerprint
            .store(fingerprint, Ordering::Release);
    }

    fn phase(&self, _store: StoreView<'_, PoolEntry<R>>) -> AdmissionPhase {
        if self.semaphore.available_permits() == 0 {
            AdmissionPhase::Saturated
        } else {
            AdmissionPhase::Ready
        }
    }

    fn load(&self, _store: StoreView<'_, PoolEntry<R>>) -> Option<Load> {
        let available = self.semaphore.available_permits();
        let capacity = self.config.max_size as usize;
        let used = capacity.saturating_sub(available);
        Some(Load::permits(used, capacity))
    }

    fn tag(&self) -> TopologyTag {
        TopologyTag::Pool
    }
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
