//! Pool topology — manages a pool of N interchangeable resource instances.
//!
//! `Pooled<R>` is the built-in framework pool topology. It supplies the
//! **slot-centric** [`Topology<R>`] hooks the framework acquire loop drives —
//! `create_slot` (create a `PoolSlot<R>`), `accept` (post-checkout validation),
//! `prepare` (per-checkout session init), `on_release` (recycle decision),
//! `idle_evictable` (maintenance predicate), and the rotation / fingerprint /
//! admission surface. **It owns no idle store and runs no checkout / fence /
//! destroy loop** — those live in the framework
//! ([`ManagedResource::run_acquire_loop`](crate::runtime::managed::ManagedResource)),
//! over the framework-owned [`InstanceStore<PoolSlot<R>>`]. The revoke-epoch
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
    time::Instant,
};

use async_trait::async_trait;
use tokio::sync::Semaphore;

use crate::{
    context::ResourceContext,
    error::Error,
    resource::Provider,
    topology::{
        AdmissionPhase, Load, MaintenanceSchedule, Ticket, Topology, Unavailable,
        pooled::{InstanceMetrics, PoolProvider, RecycleDecision, config::Config},
        store::InstanceStore,
    },
    topology_tag::TopologyTag,
};

// ─── Static error messages ───────────────────────────────────────────────────

/// Pool cannot operate with zero max size.
const ERR_MAX_SIZE_ZERO: &str = "Pooled: config.max_size must be > 0 (got 0 — would \
     deadlock the checkout semaphore on first acquire)";

/// The create-semaphore was closed (pool is shutting down).
const ERR_CREATE_SEMAPHORE_CLOSED: &str = "pool: create semaphore closed";

/// Timed out waiting for a create-semaphore permit.
const ERR_CREATE_SEMAPHORE_TIMEOUT: &str =
    "pool: create timed out waiting for create-semaphore permit";

/// The `resource.create()` call exceeded `create_timeout`.
const ERR_CREATE_TIMED_OUT: &str = "pool: create timed out";

// ─────────────────────────────────────────────────────────────────────────────

/// A single pooled instance with its metrics and config fingerprint — the
/// [`Topology::Slot`](crate::topology::Topology::Slot) for [`Pooled`].
///
/// The framework holds this slot for the whole lease (the guard owns it via the
/// release closure), so `metrics.created_at` survives checkout → lease → return:
/// max-lifetime eviction keeps firing because the slot's `created_at` is never
/// rebuilt from a bare instance.
///
/// The semaphore permit does **not** live here — it is held in the
/// [`ResourceGuard`](crate::guard::ResourceGuard) so it is returned even if the
/// release callback panics. The credential-revoke snapshot is not a field
/// either: it lives in the framework store's `checkout_epoch`.
pub struct PoolSlot<R: Provider> {
    runtime: R::Instance,
    metrics: InstanceMetrics,
    fingerprint: u64,
    /// When this slot was last returned to the idle queue.
    /// `None` for freshly created slots that have never been idle.
    returned_at: Option<Instant>,
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
/// checkout/recycle/destroy over a framework-owned idle store.
///
/// `Pooled<R>` holds only the concurrency semaphore, the create-concurrency cap,
/// the immutable pool [`Config`], and the live config fingerprint. The idle
/// queue is the framework's [`InstanceStore<PoolSlot<R>>`]
/// (`ManagedResource::store`); the framework runs every checkout / return /
/// evict against it. The pool implements the slot-centric [`Topology<R>`]
/// hooks the framework loop calls — it never touches the store directly except
/// through the rotation fan-out's `lock_idle` (which the framework grants it
/// transiently; the author can never name it).
///
/// [`Topology<R>`]: crate::topology::Topology
pub struct Pooled<R: Provider> {
    semaphore: Arc<Semaphore>,
    /// Bounds concurrent invocations of `create_slot` (#390).
    ///
    /// The checkout semaphore gates active leases; this one gates
    /// *creation* so a burst of concurrent acquires cannot fan out into
    /// `max_size` parallel `Provider::create` calls against a fragile
    /// backend.
    create_semaphore: Arc<Semaphore>,
    config: Config,
    current_fingerprint: Arc<AtomicU64>,
    /// `Pooled<R>` is keyed to its resource through the `Topology<R>` impl and
    /// `PoolSlot<R>` slot type, but holds no `R`-typed field directly (the
    /// resource lives in `ManagedResource`). `fn() -> R` keeps `Pooled<R>`
    /// covariant + `Send + Sync` regardless of `R`'s own auto-traits.
    _marker: PhantomData<fn() -> R>,
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
    pub async fn stats(&self, store: &InstanceStore<PoolSlot<R>>) -> PoolStats {
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

    /// Whether a pool slot should be evicted for a non-revoke reason (stale
    /// fingerprint, max lifetime, idle timeout). The revoke arm is owned by the
    /// framework store's epoch fence, not this predicate.
    fn should_evict_nonrevoke(&self, slot: &PoolSlot<R>) -> bool {
        let current_fp = self.current_fingerprint.load(Ordering::Acquire);
        let now = Instant::now();
        // Stale fingerprint.
        if slot.fingerprint != current_fp {
            return true;
        }
        // Max lifetime exceeded.
        if self
            .config
            .max_lifetime
            .is_some_and(|max| now.duration_since(slot.metrics.created_at) > max)
        {
            return true;
        }
        // Idle timeout exceeded.
        if let (Some(idle_timeout), Some(returned_at)) =
            (self.config.idle_timeout, slot.returned_at)
        {
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
    /// Creates a new pool slot via `resource.create()`.
    ///
    /// All creation goes through this funnel and is gated on `create_semaphore`
    /// so a burst of acquires cannot stampede a fragile backend with `max_size`
    /// parallel connects. The permit is released as soon as `Provider::create`
    /// returns. The whole path — permit wait + `resource.create` — shares a
    /// single `create_timeout` budget so a slow-creating backend cannot stall
    /// callers forever.
    ///
    /// The framework loop wraps the returned slot in its cancel-safety guard and
    /// fences a fresh-create that straddled a revoke on the store-return path
    /// (the store stamps the live epoch under the idle lock).
    ///
    /// # Errors
    ///
    /// - [`Error::backpressure`] when the create-semaphore wait times out.
    /// - [`Error::permanent`] when the create-semaphore is closed.
    /// - [`Error::transient`] when `Provider::create` itself times out.
    /// - Propagates the `Provider::create` error otherwise.
    async fn create_pool_slot(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
    ) -> Result<PoolSlot<R>, Error> {
        let deadline = Instant::now() + self.config.create_timeout;

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
        let runtime =
            match tokio::time::timeout_at(deadline.into(), resource.create(config, ctx)).await {
                Ok(Ok(rt)) => rt,
                Ok(Err(e)) => return Err(e),
                Err(_timeout) => return Err(Error::transient(ERR_CREATE_TIMED_OUT)),
            };

        Ok(PoolSlot {
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
}

// ─── Topology impl for Pooled ────────────────────────────────────────────────
//
// `Pooled<R>` supplies the slot-centric hooks the framework acquire loop drives
// over the framework-owned `InstanceStore<PoolSlot<R>>`. The pool owns no store,
// runs no checkout/destroy/fence loop, and never compares epochs — that is all
// the framework's job.

#[async_trait]
impl<R> Topology<R> for Pooled<R>
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
    type Slot = PoolSlot<R>;

    fn try_reserve(&self, _store: &InstanceStore<PoolSlot<R>>) -> Result<Ticket, Unavailable> {
        self.semaphore
            .clone()
            .try_acquire_owned()
            .map(Ticket::permit)
            .map_err(|_| Unavailable::Saturated { retry_after: None })
    }

    async fn create_slot(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
    ) -> Result<PoolSlot<R>, Error> {
        self.create_pool_slot(resource, config, ctx).await
    }

    fn slot_instance<'s>(&self, slot: &'s PoolSlot<R>) -> &'s R::Instance {
        &slot.runtime
    }

    fn into_instance(&self, slot: PoolSlot<R>) -> R::Instance {
        slot.runtime
    }

    async fn accept(&self, slot: &mut PoolSlot<R>, resource: &R, _ctx: &ResourceContext) -> bool {
        // Post-checkout validation: stale fingerprint / max lifetime / broken /
        // optional health check. `false` ⇒ the framework destroys this slot and
        // loops to the next idle slot, then create.
        let current_fp = self.current_fingerprint.load(Ordering::Acquire);
        if slot.fingerprint != current_fp {
            return false;
        }
        if self
            .config
            .max_lifetime
            .is_some_and(|max| slot.metrics.created_at.elapsed() > max)
        {
            return false;
        }
        if resource.is_broken(&slot.runtime).is_broken() {
            return false;
        }
        if self.config.test_on_checkout && resource.check(&slot.runtime).await.is_err() {
            return false;
        }
        slot.metrics.checkout_count += 1;
        true
    }

    async fn prepare(
        &self,
        slot: &mut PoolSlot<R>,
        resource: &R,
        ctx: &ResourceContext,
    ) -> Result<(), Error> {
        resource.prepare(&slot.runtime, ctx).await
    }

    async fn on_release(&self, slot: &mut PoolSlot<R>, resource: &R) -> Result<bool, Error> {
        // Recycle decision (the framework already destroyed a *tainted* lease
        // before calling this, and runs the revoke-epoch fence on `return_slot`
        // AFTER this returns `true`).
        let current_fp = self.current_fingerprint.load(Ordering::Acquire);
        if slot.fingerprint != current_fp {
            return Ok(false);
        }
        if self
            .config
            .max_lifetime
            .is_some_and(|max| slot.metrics.created_at.elapsed() > max)
        {
            return Ok(false);
        }
        if resource.is_broken(&slot.runtime).is_broken() {
            return Ok(false);
        }
        match resource.recycle(&slot.runtime, &slot.metrics).await {
            Ok(RecycleDecision::Keep) => {
                // Stamp the return time so idle-timeout can fire on the next
                // sweep.
                slot.returned_at = Some(Instant::now());
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
        // Cap the idle queue at `max_size`: an idle slot beyond the concurrency
        // cap can never be leased, so it is pure waste.
        Some(self.config.max_size as usize)
    }

    fn warmup_target(&self, _config: &R::Config) -> usize {
        self.config.min_size as usize
    }

    fn idle_evictable(&self, slot: &PoolSlot<R>) -> bool {
        self.should_evict_nonrevoke(slot)
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
        store: &InstanceStore<PoolSlot<R>>,
        slot: &str,
        refresh: bool,
    ) -> Result<(), Error> {
        // Walk the framework idle store under its lock so no checkout / return
        // can interleave mid-rotation — the same lock `checkout` / `return_slot`
        // take. The store reference is granted by the framework dispatcher; the
        // author never names it.
        //
        // Tradeoff: because the idle lock spans every entry's hook `.await`, a
        // slow hook blocks concurrent idle checkouts for the full rotation
        // duration (head-of-line blocking). New-slot creation is unaffected.
        // This is tolerated because rotation is rare (not a hot path). Do not
        // "optimize" by dropping and reacquiring the lock between entries: that
        // reopens the window for an instance to be checked out mid-rotation and
        // miss its hook (credential isolation).
        let idle = store.lock_idle().await;
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

    fn set_fingerprint(&self, fingerprint: u64) {
        self.current_fingerprint
            .store(fingerprint, Ordering::Release);
    }

    fn phase(&self, _store: &InstanceStore<PoolSlot<R>>) -> AdmissionPhase {
        if self.semaphore.available_permits() == 0 {
            AdmissionPhase::Saturated
        } else {
            AdmissionPhase::Ready
        }
    }

    fn load(&self, _store: &InstanceStore<PoolSlot<R>>) -> Option<Load> {
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
mod tests {
    use std::{future::Future, sync::atomic::AtomicBool, time::Duration};

    use nebula_core::{ExecutionId, ResourceKey, resource_key};

    use super::*;
    use crate::{
        context::ResourceContext,
        resource::{HasCredentialSlots, ResourceConfig, ResourceMetadata},
        topology::{pooled::BrokenCheck, store::ReturnOutcome},
    };

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

    #[derive(Clone)]
    struct MockPool {
        created: Arc<AtomicU64>,
        destroyed: Arc<AtomicU64>,
        broken: Arc<AtomicBool>,
        fail_check: Arc<AtomicBool>,
        recycle_drop: Arc<AtomicBool>,
        revoke_calls: Arc<AtomicU64>,
    }

    impl MockPool {
        fn new() -> Self {
            Self {
                created: Arc::new(AtomicU64::new(0)),
                destroyed: Arc::new(AtomicU64::new(0)),
                broken: Arc::new(AtomicBool::new(false)),
                fail_check: Arc::new(AtomicBool::new(false)),
                recycle_drop: Arc::new(AtomicBool::new(false)),
                revoke_calls: Arc::new(AtomicU64::new(0)),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for MockPool {
        type Config = PoolTestConfig;
        type Instance = u64;
        type Topology = Pooled<Self>;

        fn key() -> ResourceKey {
            resource_key!("mock-pool")
        }

        async fn create(
            &self,
            _config: &PoolTestConfig,
            _ctx: &ResourceContext,
        ) -> Result<u64, Error> {
            Ok(self.created.fetch_add(1, Ordering::SeqCst))
        }

        async fn check(&self, _runtime: &u64) -> Result<(), Error> {
            if self.fail_check.load(Ordering::SeqCst) {
                Err(Error::transient("check failed"))
            } else {
                Ok(())
            }
        }

        async fn destroy(&self, _runtime: u64) -> Result<(), Error> {
            self.destroyed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn on_credential_revoke(&self, _slot: &str, _runtime: &u64) -> Result<(), Error> {
            self.revoke_calls.fetch_add(1, Ordering::SeqCst);
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
        fn is_broken(&self, _runtime: &u64) -> BrokenCheck {
            if self.broken.load(Ordering::SeqCst) {
                BrokenCheck::Broken("forced break".into())
            } else {
                BrokenCheck::Healthy
            }
        }

        fn recycle(
            &self,
            _instance: &u64,
            _metrics: &InstanceMetrics,
        ) -> impl Future<Output = Result<RecycleDecision, Error>> + Send {
            let drop = self.recycle_drop.load(Ordering::SeqCst);
            async move {
                if drop {
                    Ok(RecycleDecision::Drop)
                } else {
                    Ok(RecycleDecision::Keep)
                }
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
    async fn create_slot_builds_pool_slot_with_metrics() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create_slot must succeed");
        assert_eq!(slot.metrics.checkout_count, 1);
        assert_eq!(slot.metrics.error_count, 0);
        assert_eq!(resource.created.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn slot_instance_and_into_instance_round_trip() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        let id = *topo.slot_instance(&slot);
        let owned = topo.into_instance(slot);
        assert_eq!(owned, id, "into_instance returns the same runtime");
    }

    #[tokio::test]
    async fn accept_accepts_healthy_and_bumps_checkout() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let mut slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        let before = slot.metrics.checkout_count;
        assert!(topo.accept(&mut slot, &resource, &test_ctx()).await);
        assert_eq!(slot.metrics.checkout_count, before + 1);
    }

    #[tokio::test]
    async fn accept_rejects_broken() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let mut slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        resource.broken.store(true, Ordering::SeqCst);
        assert!(!topo.accept(&mut slot, &resource, &test_ctx()).await);
    }

    #[tokio::test]
    async fn accept_rejects_stale_fingerprint() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let mut slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        topo.set_fingerprint(7);
        assert!(!topo.accept(&mut slot, &resource, &test_ctx()).await);
    }

    #[tokio::test]
    async fn accept_rejects_failed_health_check() {
        let resource = MockPool::new();
        let cfg = Config {
            test_on_checkout: true,
            ..Default::default()
        };
        let topo = mock_pool(cfg, 0);
        let mut slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        resource.fail_check.store(true, Ordering::SeqCst);
        assert!(!topo.accept(&mut slot, &resource, &test_ctx()).await);
    }

    #[tokio::test]
    async fn on_release_keeps_clean_slot() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let mut slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        assert!(
            topo.on_release(&mut slot, &resource)
                .await
                .expect("release")
        );
        assert!(slot.returned_at.is_some(), "on_release stamps returned_at");
    }

    #[tokio::test]
    async fn on_release_drops_stale_fingerprint() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let mut slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        topo.set_fingerprint(99);
        assert!(
            !topo
                .on_release(&mut slot, &resource)
                .await
                .expect("release")
        );
    }

    #[tokio::test]
    async fn on_release_drops_broken() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let mut slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        resource.broken.store(true, Ordering::SeqCst);
        assert!(
            !topo
                .on_release(&mut slot, &resource)
                .await
                .expect("release")
        );
    }

    #[tokio::test]
    async fn on_release_honours_recycle_drop() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let mut slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        resource.recycle_drop.store(true, Ordering::SeqCst);
        assert!(
            !topo
                .on_release(&mut slot, &resource)
                .await
                .expect("release")
        );
    }

    #[tokio::test]
    async fn idle_evictable_stale_fingerprint() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        assert!(!topo.idle_evictable(&slot), "a fresh slot is not evictable");
        topo.set_fingerprint(5);
        assert!(
            topo.idle_evictable(&slot),
            "a stale-fingerprint slot is idle-evictable"
        );
    }

    #[tokio::test]
    async fn idle_evictable_max_lifetime() {
        let cfg = Config {
            max_lifetime: Some(Duration::from_nanos(1)),
            ..Default::default()
        };
        let resource = MockPool::new();
        let topo = mock_pool(cfg, 0);
        let slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(topo.idle_evictable(&slot));
    }

    #[tokio::test]
    async fn try_reserve_grants_then_saturates() {
        let topo = mock_pool(
            Config {
                max_size: 1,
                ..Default::default()
            },
            0,
        );
        let store: InstanceStore<PoolSlot<MockPool>> = InstanceStore::new(None);
        let ticket = topo.try_reserve(&store).expect("first ticket");
        assert!(
            matches!(topo.try_reserve(&store), Err(Unavailable::Saturated { .. })),
            "a pool of 1 is saturated after one ticket"
        );
        assert_eq!(topo.phase(&store), AdmissionPhase::Saturated);
        drop(ticket);
        assert_eq!(topo.phase(&store), AdmissionPhase::Ready);
    }

    #[tokio::test]
    async fn load_reflects_usage() {
        let topo = mock_pool(
            Config {
                max_size: 2,
                ..Default::default()
            },
            0,
        );
        let store: InstanceStore<PoolSlot<MockPool>> = InstanceStore::new(None);
        let load = topo.load(&store).expect("pool reports load");
        assert!(load.saturation.abs() < f32::EPSILON, "idle pool is 0.0");
        let _t = topo.try_reserve(&store).expect("ticket");
        let load = topo.load(&store).expect("load");
        assert!(
            (load.saturation - 0.5).abs() < f32::EPSILON,
            "one of two used"
        );
    }

    #[tokio::test]
    async fn stats_reads_store_and_semaphore() {
        let resource = MockPool::new();
        let topo = mock_pool(
            Config {
                max_size: 4,
                ..Default::default()
            },
            0,
        );
        let store: InstanceStore<PoolSlot<MockPool>> = InstanceStore::new(None);
        let slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        let epoch = store.stamp_epoch();
        let _ = store.return_slot(slot, epoch).await;
        let stats = topo.stats(&store).await;
        assert_eq!(stats.idle, 1);
        assert_eq!(stats.capacity, 4);
    }

    #[tokio::test]
    async fn topology_metadata_hooks() {
        let topo = mock_pool(
            Config {
                min_size: 3,
                max_size: 5,
                ..Default::default()
            },
            0,
        );
        assert_eq!(topo.tag(), TopologyTag::Pool);
        assert!(topo.pools(), "the pool topology pools released slots");
        assert_eq!(topo.warmup_target(&PoolTestConfig), 3);
        assert!(
            topo.maintenance_schedule().is_some(),
            "the pool runs a maintenance reaper"
        );
    }

    #[tokio::test]
    async fn dispatch_credential_hook_walks_idle_store() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let store: InstanceStore<PoolSlot<MockPool>> = InstanceStore::new(None);

        // Two idle slots.
        for _ in 0..2 {
            let slot = topo
                .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
                .await
                .expect("create");
            let epoch = store.stamp_epoch();
            let _ = store.return_slot(slot, epoch).await;
        }

        topo.dispatch_credential_hook(&resource, &store, "db", false)
            .await
            .expect("rotation dispatch");
        assert_eq!(
            resource.revoke_calls.load(Ordering::SeqCst),
            2,
            "the revoke hook visits every idle slot in the framework store"
        );
    }

    #[test]
    fn try_new_rejects_max_size_zero() {
        let err = match Pooled::<MockPool>::try_new(
            Config {
                max_size: 0,
                ..Config::default()
            },
            1,
        ) {
            Err(e) => e,
            Ok(_) => panic!("max_size == 0 must be a typed registration error, not a pool"),
        };
        assert_eq!(*err.kind(), crate::error::ErrorKind::Permanent);
        assert!(err.to_string().contains("max_size"));
    }

    #[test]
    fn try_new_rejects_min_greater_than_max() {
        let err = match Pooled::<MockPool>::try_new(
            Config {
                min_size: 5,
                max_size: 2,
                ..Config::default()
            },
            1,
        ) {
            Err(e) => e,
            Ok(_) => panic!("min > max must be a typed registration error, not a pool"),
        };
        assert_eq!(*err.kind(), crate::error::ErrorKind::Permanent);
        assert!(err.to_string().contains("min_size") && err.to_string().contains("max_size"));
    }

    /// The revoke fence is the store's, framework-owned: a slot returned at the
    /// pre-bump epoch must be evicted on return after a bump. This exercises the
    /// exact store path the framework `release_slot` uses for the pool.
    #[tokio::test]
    async fn store_return_is_revoke_fenced() {
        let resource = MockPool::new();
        let topo = mock_pool(Config::default(), 0);
        let store: InstanceStore<PoolSlot<MockPool>> = InstanceStore::new(Some(4));

        let slot = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        let epoch = store.stamp_epoch();
        store.bump_revoke_epoch();
        assert!(
            store.return_slot(slot, epoch).await.is_evict(),
            "a slot checked out before a revoke must be evicted by the store fence"
        );

        let slot2 = topo
            .create_pool_slot(&resource, &PoolTestConfig, &test_ctx())
            .await
            .expect("create");
        let fresh = store.stamp_epoch();
        // `ReturnOutcome<PoolSlot<_>>` is not `Debug`/`PartialEq` (the slot is
        // not), so match the recycled arm rather than comparing for equality.
        assert!(
            matches!(
                store.return_slot(slot2, fresh).await,
                ReturnOutcome::Recycled
            ),
            "a slot checked out after the revoke is unaffected"
        );
        assert_eq!(store.len().await, 1, "the post-revoke slot recycled");
    }
}
