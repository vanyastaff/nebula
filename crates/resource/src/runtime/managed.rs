//! Per-registration runtime holding topology + metadata.
//!
//! [`ManagedResource`] is the internal representation of a registered
//! resource. It bundles the resource implementation, hot-swappable config, the
//! framework-owned [`InstanceStore`] idle queue, the resource's
//! [`Provider::Topology`], the release queue, and lifecycle metadata.
//!
//! The framework reaches the topology monomorphically through the resource's
//! associated [`Topology`] type. The acquire loop, fenced checkout, cancel-safe
//! guard-wrap, and on-release return-or-destroy live in the sibling
//! `acquire_loop` module. This module holds only the
//! admission-surface + status/phase/taint/drain impl blocks.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use arc_swap::ArcSwap;
use tokio::sync::Notify;

use crate::{
    recovery::RecoveryGate,
    release_queue::ReleaseQueue,
    resource::{HasCredentialSlots, Provider},
    state::{ResourcePhase, ResourceStatus},
    topology::{
        AdmissionPhase, Load, MaintenanceSchedule, Topology, Unavailable, store::InstanceStore,
    },
    topology_tag::TopologyTag,
};

/// The `Slot` type of a resource's topology — the leasable unit the framework
/// stores and the guard holds for its whole lease.
pub(crate) type SlotOf<R> = <<R as Provider>::Topology as Topology<R>>::Slot;

/// Per-registration runtime holding topology + metadata.
///
/// Created once per `Manager::register()` call and stored for the
/// lifetime of the resource. The `config` and `status` fields are
/// atomically swappable for hot-reload.
pub struct ManagedResource<R: Provider> {
    /// The resource implementation. Held alongside the topology so the
    /// framework's acquire loop, credential-rotation, and maintenance walks can
    /// hand the resource handle to the topology's hooks (the topology drives the
    /// hooks; the resource value is owned here).
    pub(crate) resource: R,
    /// Hot-swappable operational configuration.
    pub(crate) config: ArcSwap<R::Config>,
    /// The resource's lease topology, reached monomorphically.
    pub(crate) topology: R::Topology,
    /// Framework-owned idle store the acquire loop fences on every checkout /
    /// return / sweep.
    ///
    /// This is the **real** idle queue: built-in [`Pooled`](crate::topology::Pooled)
    /// recycles `PoolSlot<R>`s here; [`Resident`](crate::topology::Resident)
    /// (which does not pool) leaves it empty. A custom topology receives a
    /// borrowed `&store` it cannot retain — the structural barrier against a
    /// cross-scope instance cache — and the framework, not the topology, runs
    /// `checkout` / `return_slot` / `evict_stale` against it.
    pub(crate) store: InstanceStore<SlotOf<R>>,
    /// Background worker pool for async cleanup.
    pub(crate) release_queue: Arc<ReleaseQueue>,
    /// Monotonically increasing generation counter (bumped on reload).
    pub(crate) generation: AtomicU64,
    /// Current lifecycle status (phase + last error).
    pub(crate) status: ArcSwap<ResourceStatus>,
    /// Optional recovery gate for thundering-herd prevention.
    ///
    /// When set, acquire calls check the gate before proceeding and
    /// trigger passive recovery on transient failures.
    pub(crate) recovery_gate: Option<Arc<RecoveryGate>>,
    /// Resource-level taint flag set by [`taint`](Self::taint).
    ///
    /// When `true`, the manager's acquire paths reject new acquires for
    /// this resource. Used by `Manager::revoke_slot` to stop handing out
    /// leases on a revoked credential *before* draining in-flight work and
    /// invoking the revoke hook. This is the resource-scoped analogue of
    /// the per-handle taint on [`ResourceGuard`](crate::guard::ResourceGuard)
    /// and the manager-wide `shutting_down` flag — one shared mechanism,
    /// not a parallel one.
    pub(crate) tainted: AtomicBool,
    /// Per-resource in-flight acquire counter `(active, notify)`.
    ///
    /// Every `acquire_*` against *this* row pre-counts here (alongside the
    /// manager-wide `Manager::drain_tracker`) and the resulting
    /// [`ResourceGuard`](crate::guard::ResourceGuard) decrements + notifies
    /// it on drop. `Manager::revoke_slot` drains **only this** counter, so a
    /// revoke on resource A never blocks on in-flight traffic to an unrelated
    /// resource B, and the `AcqRel` taint→increment→post-taint-recheck
    /// ordering against this same counter is what closes the
    /// revoke-vs-acquire TOCTOU. Two-phase-revoke / drain invariant: see the
    /// [`manager`](crate::manager) module documentation.
    pub(crate) in_flight: Arc<(AtomicU64, Notify)>,
    /// Count of background maintenance sweeps run so far.
    ///
    /// Drives the cost-aware health-probe cadence: the reaper probes idle slots
    /// via [`Provider::check`] only on sweeps where
    /// `sweeps % R::check_cost().probe_every_n_sweeps() == 0`, so an
    /// [`Expensive`](crate::CheckCost::Expensive) check runs far less often than
    /// a [`Cheap`](crate::CheckCost::Cheap) one. Bumped once per
    /// [`run_maintenance`](Self::run_maintenance).
    pub(crate) maintenance_sweeps: AtomicU64,
}

impl<R: Provider> ManagedResource<R> {
    /// Returns the current generation counter.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Returns a snapshot of the current lifecycle status.
    pub fn status(&self) -> Arc<ResourceStatus> {
        self.status.load_full()
    }

    /// Returns a snapshot of the current configuration.
    pub fn config(&self) -> Arc<R::Config> {
        self.config.load_full()
    }

    /// Atomically replace the lifecycle status with a new phase.
    ///
    /// Rebuilds a fresh [`ResourceStatus`] from the latest snapshot,
    /// copying the current generation across and preserving `last_error`.
    /// Used by the manager to drive phase transitions on register, reload
    /// and shutdown.
    pub(crate) fn set_phase(&self, phase: ResourcePhase) {
        let prev = self.status.load_full();
        let next = ResourceStatus {
            phase,
            generation: self.generation(),
            last_error: prev.last_error.clone(),
        };
        self.status.store(Arc::new(next));
    }

    /// Replace the lifecycle status with `Failed` and record a reason.
    ///
    /// Wired by `Manager::set_phase_all_failed`: when
    /// `DrainTimeoutPolicy::Abort` fires we transition every registered
    /// resource to `Failed` so callers cannot subsequently acquire a
    /// resource the manager has already declared bankrupt. Per-resource
    /// `HealthChanged{healthy:false}` event emission is owned by the
    /// manager because it holds the event bus.
    pub(crate) fn set_failed(&self, error: impl Into<String>) {
        let next = ResourceStatus {
            phase: ResourcePhase::Failed,
            generation: self.generation(),
            last_error: Some(error.into()),
        };
        self.status.store(Arc::new(next));
    }

    /// Marks the resource tainted so the manager rejects new acquires.
    ///
    /// Phase 1 of the two-phase revoke: `Manager::revoke_slot` calls this
    /// synchronously, before draining, reusing the same "stop new leases"
    /// mechanism as the per-handle `ResourceGuard::taint` and the
    /// manager-wide `shutting_down` flag. See the [`manager`](crate::manager)
    /// module docs for the canonical invariant.
    pub(crate) fn taint(&self) {
        self.tainted.store(true, Ordering::Release);
    }

    /// Returns `true` if [`taint`](Self::taint) has been called.
    pub(crate) fn is_tainted(&self) -> bool {
        self.tainted.load(Ordering::Acquire)
    }

    /// Returns a clone of this resource's per-resource in-flight tracker so
    /// an acquire pipeline can pre-count against it (and hand it to the
    /// resulting guard). Distinct from the manager-wide `drain_tracker`:
    /// `Manager::revoke_slot` drains *this* counter only. See the
    /// [`manager`](crate::manager) module docs for the canonical invariant.
    pub(crate) fn in_flight_tracker(&self) -> Arc<(AtomicU64, Notify)> {
        Arc::clone(&self.in_flight)
    }

    /// Drains *this* resource's in-flight acquires (bounded by `timeout`).
    ///
    /// The per-resource analogue of `Manager::wait_for_drain`: it waits on
    /// this row's own counter, not the manager-wide one, and reuses the exact
    /// lost-wakeup-safe ordering of the shared shutdown drain helper. Returns
    /// `Ok(())` once drained, or `Err(outstanding)` with the counter snapshot
    /// at the moment the timer fired (the caller — `revoke_resolved` — keeps
    /// the taint and proceeds to the revoke hook regardless; the timeout is
    /// best-effort because the taint already stops *new* leases). See the
    /// [`manager`](crate::manager) module docs for the canonical invariant.
    pub(crate) async fn wait_for_in_flight_drain(&self, timeout: Duration) -> Result<(), u64> {
        crate::manager::shutdown::wait_for_tracker_drain(&self.in_flight, timeout).await
    }
}

// Admission surface + diagnostics that the type-erased handle forwards. Needs
// only `R::Topology: Topology<R>` (no `Clone` / `R::Instance: Clone`), so it is
// a separate block usable by the erased admission probes.
impl<R> ManagedResource<R>
where
    R: Provider + HasCredentialSlots,
    R::Topology: Topology<R>,
{
    /// The topology tag for rotation / diagnostic spans.
    pub(crate) fn topology_tag(&self) -> TopologyTag {
        self.topology.tag()
    }

    /// `Some(schedule)` if the topology runs a background maintenance reaper.
    pub(crate) fn maintenance_schedule(&self) -> Option<MaintenanceSchedule> {
        self.topology.maintenance_schedule()
    }

    /// Updates the topology's config fingerprint (no-op for topologies that
    /// track none) so stale idle slots evict on the next sweep / acquire.
    pub(crate) fn set_fingerprint(&self, fingerprint: u64) {
        self.topology.set_fingerprint(fingerprint);
    }

    /// Admission phase snapshot from the topology.
    pub(crate) fn admission_phase(&self) -> AdmissionPhase {
        self.topology.phase(&self.store)
    }

    /// Admission load snapshot from the topology.
    pub(crate) fn admission_load(&self) -> Option<Load> {
        self.topology.load(&self.store)
    }

    /// Sync capacity gate from the topology — an **advisory** yes/no pre-check
    /// with a typed reason, NOT a held reservation.
    ///
    /// The [`Ticket`] (and any semaphore permit it carries) is dropped
    /// immediately, so the permit is released the moment this returns. This is a
    /// deliberate pre-flight probe (e.g. for `Manager::admission_status`): under
    /// contention the permit it momentarily held can be taken by another
    /// acquirer before the real acquire runs, so a gate `Ok` does not guarantee
    /// the subsequent acquire admits — the authoritative reservation is the
    /// `try_reserve` inside [`run_acquire_loop`](Self::run_acquire_loop), whose
    /// `Ticket` IS held for the lease. A gate `Err(Saturated)` likewise releases
    /// its permit; it reports the rejection, it does not hold it.
    pub(crate) fn try_reserve_gate(&self) -> Result<(), Unavailable> {
        self.topology.try_reserve(&self.store).map(|_ticket| ())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicBool, AtomicU64, Ordering},
        time::{Duration, Instant},
    };

    use arc_swap::ArcSwap;
    use nebula_core::{ExecutionId, ResourceKey, resource_key};
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        context::ResourceContext,
        error::Error,
        options::AcquireOptions,
        release_queue::ReleaseQueue,
        resource::{ResourceConfig, ResourceMetadata, TeardownCx, TeardownReason},
        runtime::teardown::{destroy_within, teardown_deadline},
        topology::{Pooled, pooled::config::Config as PoolConfig, store::InstanceStore},
    };

    // A minimal pooled resource over which the framework acquire loop runs.
    #[derive(Clone)]
    struct PoolCfg;
    crate::impl_empty_has_schema!(PoolCfg);
    impl ResourceConfig for PoolCfg {
        fn fingerprint(&self) -> u64 {
            0
        }
    }

    #[derive(Clone)]
    struct Mock {
        created: Arc<AtomicU64>,
        destroyed: Arc<AtomicU64>,
        park_create: Arc<AtomicBool>,
        create_entered: Arc<Notify>,
        release_create: Arc<Notify>,
        checks: Arc<AtomicU64>,
        check_cost: crate::CheckCost,
        check_fails: Arc<AtomicBool>,
        check_panics: Arc<AtomicBool>,
    }

    impl Mock {
        fn new() -> Self {
            Self {
                created: Arc::new(AtomicU64::new(0)),
                destroyed: Arc::new(AtomicU64::new(0)),
                park_create: Arc::new(AtomicBool::new(false)),
                create_entered: Arc::new(Notify::new()),
                release_create: Arc::new(Notify::new()),
                checks: Arc::new(AtomicU64::new(0)),
                check_cost: crate::CheckCost::Cheap,
                check_fails: Arc::new(AtomicBool::new(false)),
                check_panics: Arc::new(AtomicBool::new(false)),
            }
        }

        fn with_check_cost(mut self, cost: crate::CheckCost) -> Self {
            self.check_cost = cost;
            self
        }
    }

    #[async_trait::async_trait]
    impl Provider for Mock {
        type Config = PoolCfg;
        type Instance = u64;
        type Topology = Pooled<Self>;

        fn key() -> ResourceKey {
            resource_key!("managed-loop-mock")
        }

        async fn create(&self, _config: &PoolCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
            let id = self.created.fetch_add(1, Ordering::SeqCst);
            if self.park_create.swap(false, Ordering::SeqCst) {
                self.create_entered.notify_one();
                self.release_create.notified().await;
            }
            Ok(id)
        }

        async fn check(&self, _instance: &u64) -> Result<(), Error> {
            self.checks.fetch_add(1, Ordering::SeqCst);
            assert!(
                !self.check_panics.load(Ordering::SeqCst),
                "mock health check panics (probe-isolation test)"
            );
            if self.check_fails.load(Ordering::SeqCst) {
                Err(Error::transient("mock health check failed"))
            } else {
                Ok(())
            }
        }

        fn check_cost(&self) -> crate::CheckCost {
            self.check_cost
        }

        async fn destroy(&self, _runtime: u64, _cx: TeardownCx) -> Result<(), Error> {
            self.destroyed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl HasCredentialSlots for Mock {
        fn credential_slot_epoch(&self) -> u64 {
            0
        }
    }

    impl crate::topology::pooled::PoolProvider for Mock {}

    fn test_ctx() -> ResourceContext {
        use nebula_core::scope::Scope;
        let scope = Scope {
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        };
        ResourceContext::minimal(scope, CancellationToken::new())
    }

    fn managed(resource: Mock, config: PoolConfig) -> Arc<ManagedResource<Mock>> {
        let (rq, _handle) = ReleaseQueue::new(1);
        let topology = Pooled::<Mock>::new(config, 0);
        Arc::new(ManagedResource {
            resource,
            config: ArcSwap::from_pointee(PoolCfg),
            topology,
            store: InstanceStore::new(None),
            release_queue: Arc::new(rq),
            generation: AtomicU64::new(0),
            status: ArcSwap::from_pointee(ResourceStatus::new()),
            recovery_gate: None,
            tainted: AtomicBool::new(false),
            in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
            maintenance_sweeps: AtomicU64::new(0),
        })
    }

    #[tokio::test]
    async fn loop_creates_then_recycles_then_reuses() {
        let resource = Mock::new();
        let created = Arc::clone(&resource.created);
        let mr = managed(
            resource,
            PoolConfig {
                max_size: 2,
                ..Default::default()
            },
        );

        // First acquire creates one slot.
        let g = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await
            .expect("first acquire");
        assert_eq!(*g, 0);
        // Release inline so the slot recycles into the framework store.
        g.release().await.expect("release recycles");
        assert_eq!(mr.store.len().await, 1, "the slot recycled into the store");

        // Second acquire reuses the idle slot — no new create.
        let g2 = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await
            .expect("second acquire");
        assert_eq!(*g2, 0, "reused the recycled slot");
        assert_eq!(
            created.load(Ordering::SeqCst),
            1,
            "the second acquire reused the idle slot — no extra create"
        );
        g2.release().await.expect("release");
    }

    /// The framework loop's revoke fence: a slot idle before a bump is evicted
    /// (and destroyed by the framework) on the next acquire — the author writes
    /// no fence code.
    #[tokio::test]
    async fn loop_evicts_revoke_stale_idle_slot_on_acquire() {
        let resource = Mock::new();
        let destroyed = Arc::clone(&resource.destroyed);
        let created = Arc::clone(&resource.created);
        let mr = managed(
            resource,
            PoolConfig {
                max_size: 2,
                ..Default::default()
            },
        );

        // Acquire + release so a clean slot sits idle.
        let g = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await
            .expect("acquire");
        g.release().await.expect("release");
        assert_eq!(mr.store.len().await, 1);

        // Revoke (the manager phase-1 synchronous bump).
        mr.bump_revoke_epoch();

        // Next acquire: the FRAMEWORK loop checks out, sees the stale slot,
        // destroys it, and creates a fresh one. The author wrote no fence code.
        let g2 = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await
            .expect("acquire after revoke");
        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            1,
            "the framework destroyed the since-revoked idle slot on checkout"
        );
        assert_eq!(
            created.load(Ordering::SeqCst),
            2,
            "a fresh slot was created after the stale one was fenced"
        );
        // The fresh lease is the post-revoke instance, not the stale one.
        assert_eq!(*g2, 1);
        g2.release().await.expect("release");
    }

    /// Max-lifetime eviction keeps firing because the slot's `created_at`
    /// survives the round-trip (slot-centric). A slot older than max_lifetime is
    /// not re-handed-out: the loop's `accept` rejects it and the framework
    /// creates a fresh one.
    #[tokio::test]
    async fn loop_max_lifetime_rejects_aged_idle_slot() {
        let resource = Mock::new();
        let created = Arc::clone(&resource.created);
        let mr = managed(
            resource,
            PoolConfig {
                max_size: 2,
                max_lifetime: Some(Duration::from_millis(20)),
                ..Default::default()
            },
        );

        let g = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await
            .expect("acquire");
        g.release().await.expect("release");
        assert_eq!(mr.store.len().await, 1);

        // Age the idle slot past max_lifetime.
        tokio::time::sleep(Duration::from_millis(40)).await;

        let g2 = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await
            .expect("acquire after aging");
        assert_eq!(
            created.load(Ordering::SeqCst),
            2,
            "the aged idle slot was rejected by `accept` (created_at survived \
             the round-trip) and a fresh slot was created"
        );
        g2.release().await.expect("release");
    }

    /// Maintenance over the framework store evicts both revoke-stale and
    /// non-revoke (fingerprint) idle slots, destroying each.
    #[tokio::test]
    async fn maintenance_evicts_stale_and_revoked() -> Result<(), Error> {
        let resource = Mock::new();
        let destroyed = Arc::clone(&resource.destroyed);
        let mr = managed(
            resource,
            PoolConfig {
                max_size: 4,
                idle_timeout: None,
                max_lifetime: None,
                ..Default::default()
            },
        );

        // Two clean idle slots: hold BOTH guards live, then release both. A
        // serial acquire-release reuses the single idle slot (correct pooling),
        // which would deposit only one — so the two leases must overlap to
        // accumulate two distinct slots for the maintenance sweep to evict.
        let g1 = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await?;
        let g2 = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await?;
        g1.release().await?;
        g2.release().await?;
        assert_eq!(mr.store.len().await, 2);

        // No change yet → nothing evicted.
        assert_eq!(mr.run_maintenance().await, 0);

        // Bump fingerprint → both become non-revoke-evictable.
        mr.set_fingerprint(99);
        assert_eq!(mr.run_maintenance().await, 2);
        assert_eq!(destroyed.load(Ordering::SeqCst), 2);
        assert_eq!(mr.store.len().await, 0);
        Ok(())
    }

    /// A11: the background health probe fires at a cadence set by
    /// [`CheckCost`](crate::CheckCost) — a `Cheap` check is probed every sweep, an
    /// `Expensive` one once per 16 sweeps, so an expensive probe does not hammer
    /// an idle pool.
    #[tokio::test]
    async fn health_probe_cadence_scales_with_check_cost() -> Result<(), Error> {
        async fn one_idle(
            cost: crate::CheckCost,
        ) -> Result<(Arc<AtomicU64>, Arc<ManagedResource<Mock>>), Error> {
            let resource = Mock::new().with_check_cost(cost);
            let checks = Arc::clone(&resource.checks);
            let mr = managed(
                resource,
                PoolConfig {
                    max_size: 2,
                    ..Default::default()
                },
            );
            let g = mr
                .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
                .await?;
            g.release().await?;
            assert_eq!(mr.store.len().await, 1, "one slot recycled into the store");
            Ok((checks, mr))
        }

        let (cheap_checks, cheap) = one_idle(crate::CheckCost::Cheap).await?;
        let (expensive_checks, expensive) = one_idle(crate::CheckCost::Expensive).await?;

        for _ in 0..16 {
            cheap.run_maintenance().await;
            expensive.run_maintenance().await;
        }

        assert_eq!(
            cheap_checks.load(Ordering::SeqCst),
            16,
            "a Cheap check is probed on every one of the 16 sweeps"
        );
        assert_eq!(
            expensive_checks.load(Ordering::SeqCst),
            1,
            "an Expensive check is probed once in 16 sweeps (every 16th)"
        );
        Ok(())
    }

    /// A11: a probe whose `check` fails evicts and destroys the unhealthy idle
    /// slot, so the next acquire rebuilds a fresh one.
    #[tokio::test]
    async fn health_probe_evicts_unhealthy_idle_slot() -> Result<(), Error> {
        let resource = Mock::new();
        let destroyed = Arc::clone(&resource.destroyed);
        let check_fails = Arc::clone(&resource.check_fails);
        let mr = managed(
            resource,
            PoolConfig {
                max_size: 2,
                ..Default::default()
            },
        );

        let g = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await?;
        g.release().await?;
        assert_eq!(mr.store.len().await, 1);

        // The slot's health check now fails — the probe must evict + destroy it.
        check_fails.store(true, Ordering::SeqCst);
        let evicted = mr.run_maintenance().await;

        assert_eq!(evicted, 1, "the failing probe evicted the unhealthy slot");
        assert_eq!(mr.store.len().await, 0, "the unhealthy slot left the store");
        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            1,
            "the probed-out slot was destroyed"
        );
        Ok(())
    }

    /// A11 foolproofing: a probe whose `check` PANICS is caught by the framework
    /// (routed through `guard_author_hook`) — the reaper is not crashed, and the
    /// slot is treated as unhealthy and evicted/destroyed.
    #[tokio::test]
    async fn health_probe_isolates_panicking_check() -> Result<(), Error> {
        let resource = Mock::new();
        let destroyed = Arc::clone(&resource.destroyed);
        let check_panics = Arc::clone(&resource.check_panics);
        let mr = managed(
            resource,
            PoolConfig {
                max_size: 2,
                ..Default::default()
            },
        );

        let g = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await?;
        g.release().await?;
        assert_eq!(mr.store.len().await, 1);

        // The probe's `check` now panics — the chokepoint must catch it (not
        // crash the reaper) and evict the slot.
        check_panics.store(true, Ordering::SeqCst);
        let evicted = mr.run_maintenance().await;

        assert_eq!(
            evicted, 1,
            "a panicking probe is isolated and the slot evicted"
        );
        assert_eq!(mr.store.len().await, 0);
        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            1,
            "the panicked-on slot was destroyed"
        );
        Ok(())
    }

    /// Warmup pre-creates `warmup_target` slots into the framework store.
    #[tokio::test]
    async fn warmup_fills_store() {
        let resource = Mock::new();
        let mr = managed(
            resource,
            PoolConfig {
                min_size: 3,
                max_size: 5,
                ..Default::default()
            },
        );
        let created = mr.warmup(&test_ctx()).await;
        assert_eq!(created, 3, "warmup creates `min_size` slots");
        assert_eq!(mr.store.len().await, 3, "warmed slots land in the store");
    }

    // ----- ADR-0093 per-resource teardown deadline -----

    /// A resource that declares a short `teardown_budget` and whose `destroy`
    /// hangs forever. Drives the per-resource deadline tests.
    #[derive(Clone)]
    struct SlowTeardown {
        budget: Duration,
        last_reason: Arc<std::sync::Mutex<Option<TeardownReason>>>,
    }

    impl SlowTeardown {
        fn new(budget: Duration) -> Self {
            Self {
                budget,
                last_reason: Arc::new(std::sync::Mutex::new(None)),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for SlowTeardown {
        type Config = PoolCfg;
        type Instance = u64;
        type Topology = Pooled<Self>;

        fn key() -> ResourceKey {
            resource_key!("slow-teardown-mock")
        }

        async fn create(&self, _config: &PoolCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
            Ok(0)
        }

        fn teardown_budget(&self) -> Duration {
            self.budget
        }

        async fn destroy(&self, _runtime: u64, cx: TeardownCx) -> Result<(), Error> {
            if let Ok(mut slot) = self.last_reason.lock() {
                *slot = Some(cx.reason);
            }
            // Hang forever: the framework's per-resource deadline must abandon
            // this — the test proves the bound bites, not the body.
            std::future::pending::<()>().await;
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl HasCredentialSlots for SlowTeardown {
        fn credential_slot_epoch(&self) -> u64 {
            0
        }
    }

    impl crate::topology::pooled::PoolProvider for SlowTeardown {}

    /// A sub-30s per-resource `teardown_budget` bounds a hanging `destroy`: the
    /// framework abandons it at the deadline and returns a typed error rather
    /// than blocking. `start_paused` fires the deadline deterministically with
    /// no wall-clock wait. This is the deferred per-resource-deadline landing —
    /// the previous release-hang test relied on the global 30s ceiling.
    #[tokio::test(start_paused = true)]
    async fn destroy_within_abandons_hanging_destroy_at_short_budget() {
        let resource = SlowTeardown::new(Duration::from_millis(50));
        let started = Instant::now();
        let outcome = destroy_within(&resource, 0u64, TeardownReason::Released).await;
        assert!(
            outcome.is_err(),
            "a hanging destroy must be abandoned at the per-resource deadline"
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the 50ms budget — not the 30s global ceiling — bounded the teardown"
        );
    }

    /// A `Revoked` teardown caps at 5s even when the resource declares a 60s
    /// budget: the composed deadline must sit at or below now+5s.
    #[tokio::test(start_paused = true)]
    async fn revoked_teardown_caps_at_five_seconds() {
        let resource = SlowTeardown::new(Duration::from_mins(1));

        // `teardown_deadline` adds the budget to a fresh `Instant::now()`. The
        // monotonic clock keeps ticking under `start_paused` (only tokio's timer
        // is paused), so bracket the cap with a small tolerance rather than an
        // exact equality: the revoke deadline must land close to now+5s and far
        // below the declared 60s.
        let before = Instant::now();
        let revoke_deadline = teardown_deadline(&resource, TeardownReason::Revoked);
        assert!(
            revoke_deadline <= before + Duration::from_secs(6),
            "a revoke teardown is capped at ~5s regardless of the declared 60s budget"
        );
        assert!(
            revoke_deadline >= before + Duration::from_secs(4),
            "the revoke cap is the 5s budget, not an over-aggressive clamp"
        );

        // The non-revoke path honors the full declared budget (sanity: the cap
        // is revoke-specific, not a blanket clamp).
        let release_deadline = teardown_deadline(&resource, TeardownReason::Released);
        assert!(
            release_deadline >= before + Duration::from_secs(59),
            "a non-revoke teardown keeps the full 60s budget"
        );

        // And a 60s-hanging destroy under revoke is abandoned ~5s in.
        let started = Instant::now();
        let outcome = destroy_within(&resource, 0u64, TeardownReason::Revoked).await;
        assert!(outcome.is_err(), "revoke teardown abandoned at the 5s cap");
        assert!(
            started.elapsed() <= Duration::from_secs(6),
            "the revoke cap (5s), not the 60s budget, bounded the teardown"
        );
    }

    /// A `destroy` impl observes `cx.reason`: the framework hands the reason it
    /// composed the teardown for, so an author can adapt graceful behavior.
    #[tokio::test(start_paused = true)]
    async fn destroy_observes_teardown_reason() {
        let resource = SlowTeardown::new(Duration::from_millis(10));
        let recorder = Arc::clone(&resource.last_reason);

        // Hanging destroy is abandoned at the 10ms budget, but the reason is
        // recorded synchronously on entry before the hang.
        let _ = destroy_within(&resource, 0u64, TeardownReason::Shutdown).await;

        let observed = recorder.lock().ok().and_then(|g| *g);
        assert_eq!(
            observed,
            Some(TeardownReason::Shutdown),
            "the destroy impl saw the reason the framework tore it down for"
        );
    }
}
