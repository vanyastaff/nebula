//! Per-registration runtime holding topology + metadata.
//!
//! [`ManagedResource`] is the internal representation of a registered
//! resource. It bundles the resource implementation, hot-swappable config,
//! the resource's [`Provider::Topology`],
//! release queue, and lifecycle metadata.
//!
//! The framework reaches the topology monomorphically through the resource's
//! associated [`Topology`] type. The
//! topology-specific operations the manager pipeline needs — produce a
//! [`ResourceGuard`], warm up, run the maintenance reaper, dispatch a
//! credential rotation hook, advance the revoke fence, and report the
//! admission surface — are expressed through the crate-internal
//! [`TopologyDispatch`] bridge, implemented by the built-in
//! [`Pooled`](crate::topology::Pooled) / [`Resident`](crate::topology::Resident)
//! topologies and by any custom topology a resource pins.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use arc_swap::ArcSwap;
use async_trait::async_trait;
use tokio::sync::Notify;

use crate::{
    context::ResourceContext,
    error::Error,
    guard::ResourceGuard,
    metrics::ResourceOpsMetrics,
    options::AcquireOptions,
    recovery::RecoveryGate,
    release_queue::ReleaseQueue,
    resource::{HasCredentialSlots, Provider},
    state::{ResourcePhase, ResourceStatus},
    topology::{AdmissionPhase, Load, Topology, Unavailable},
    topology_tag::TopologyTag,
};

/// Crate-internal bridge from the framework manager pipeline to a concrete
/// topology, monomorphic in the resource type `R`.
///
/// `Provider::Topology` only guarantees [`Topology`] (the open lease/admission
/// contract). The manager pipeline additionally needs operations that produce a
/// typed [`ResourceGuard<R>`], drive warmup / maintenance, and dispatch the
/// per-slot credential rotation hook against the resource handle. Those are
/// expressed here, keyed to `R`, and implemented by the built-in
/// [`Pooled<R>`](crate::topology::Pooled) /
/// [`Resident<R>`](crate::topology::Resident) topologies (and any custom
/// topology a resource pins as its `type Topology`).
///
/// The trait is `#[async_trait]` so the manager can hold a `dyn`-free but
/// `async`-method bridge; it is reached monomorphically (never behind a `dyn`),
/// so the boxed-future cost is negligible next to the I/O each method performs.
#[async_trait]
pub trait TopologyDispatch<R: Provider>: Topology + Send + Sync + 'static {
    /// Runs the full acquire pipeline for this topology and returns a typed
    /// [`ResourceGuard<R>`]. This is the inherent acquire (idle checkout /
    /// create / prepare for pool; clone-or-create for resident), distinct from
    /// the open [`Topology::acquire`] (which only consumes the admission
    /// ticket).
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the inherent pool acquire — resource/config/ctx/queue/gen/options/metrics are distinct concerns"
    )]
    async fn acquire_guard(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        options: &AcquireOptions,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Result<ResourceGuard<R>, Error>;

    /// Pre-warms the topology (no-op for topologies without an idle queue).
    /// Returns the number of instances created.
    async fn warmup(&self, _resource: &R, _config: &R::Config, _ctx: &ResourceContext) -> usize {
        0
    }

    /// Runs one background maintenance sweep (idle-timeout / max-lifetime /
    /// stale-fingerprint / revoke eviction). Returns the number evicted.
    /// No-op for topologies without an idle queue.
    async fn run_maintenance(&self, _resource: &R) -> usize {
        0
    }

    /// `Some((idle_timeout, max_lifetime, maintenance_interval))` if this
    /// topology runs a background maintenance reaper, else `None`.
    ///
    /// The manager only spawns the reaper task when this returns `Some` and at
    /// least one TTL is configured, so topologies with no idle eviction pay
    /// zero background cost.
    fn maintenance_schedule(&self) -> Option<MaintenanceSchedule> {
        None
    }

    /// Advances the credential-revoke fence (no-op for topologies with no idle
    /// queue to fence).
    fn bump_revoke_epoch(&self) {}

    /// Updates the config fingerprint so stale idle instances are evicted on
    /// the next acquire / sweep (no-op for topologies without a fingerprint).
    fn set_fingerprint(&self, _fingerprint: u64) {}

    /// Dispatches the per-slot credential rotation hook against this topology's
    /// live instances. Default no-op: a topology that manages no framework idle
    /// queue has nothing to rotate.
    async fn dispatch_credential_hook(
        &self,
        _resource: &R,
        _slot: &str,
        _refresh: bool,
    ) -> Result<(), Error> {
        Ok(())
    }
}

/// Background-maintenance cadence + TTLs for a topology that runs a reaper.
#[derive(Debug, Clone, Copy)]
pub struct MaintenanceSchedule {
    /// Idle-timeout TTL, if configured.
    pub idle_timeout: Option<Duration>,
    /// Max-lifetime TTL, if configured.
    pub max_lifetime: Option<Duration>,
    /// Interval between maintenance sweeps.
    pub maintenance_interval: Duration,
}

/// Per-registration runtime holding topology + metadata.
///
/// Created once per `Manager::register()` call and stored for the
/// lifetime of the resource. The `config` and `status` fields are
/// atomically swappable for hot-reload.
pub struct ManagedResource<R: Provider> {
    /// The resource implementation. Held alongside the topology so the
    /// framework's uniform credential-rotation / maintenance walks can hand
    /// the resource handle to the topology's hooks (the topology drives the
    /// hooks; the resource value is owned here).
    pub(crate) resource: R,
    /// Hot-swappable operational configuration.
    pub(crate) config: ArcSwap<R::Config>,
    /// The resource's lease topology, reached monomorphically.
    pub(crate) topology: R::Topology,
    /// Framework-owned storage borrowed by the open
    /// [`Topology`](crate::topology::Topology) admission methods.
    ///
    /// The built-in [`Pooled`](crate::topology::Pooled) /
    /// [`Resident`](crate::topology::Resident) topologies manage their own
    /// internal storage and ignore this (their `Slot = ()`); it exists so the
    /// open contract's `&InstanceStore<Slot>` argument has a real, borrowable
    /// store without a per-call allocation. A custom topology receives a
    /// borrowed `&store` it cannot retain — the structural barrier against a
    /// cross-scope instance cache.
    pub(crate) store: crate::topology::store::InstanceStore<<R::Topology as Topology>::Slot>,
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

// ── Operations that need the topology bridge (`R::Topology: TopologyDispatch<R>`).
//
// Split into its own `impl` block so the weak `R: Provider` block above stays
// usable by code that never touches the topology (status / phase / taint /
// drain). Everything that reaches into the topology — revoke fence, rotation
// dispatch, the typed acquire pipeline — lives here behind the bridge bound.

impl<R: Provider> ManagedResource<R>
where
    R: HasCredentialSlots,
    R::Topology: TopologyDispatch<R>,
{
    /// Advances the credential-revoke fence so every return-to-idle path
    /// destroys (never recycles or admits) an instance authenticated with the
    /// now-revoked credential.
    ///
    /// Called synchronously by `Manager::revoke_slot` in phase 1, before the
    /// revoke hook is dispatched — the same pre-`.await` discipline as
    /// [`taint`](Self::taint). Delegates to the topology, which owns the fence
    /// (a no-op for topologies with no idle queue).
    pub(crate) fn bump_revoke_epoch(&self) {
        self.topology.bump_revoke_epoch();
    }

    /// Borrows the live topology and invokes the per-slot credential hook —
    /// [`Provider::on_credential_refresh`] when `refresh` is `true`,
    /// [`Provider::on_credential_revoke`] otherwise — against this resource's
    /// instances.
    ///
    /// The dispatch is topology-specific (resident reconcile vs pool idle
    /// fan-out) and lives behind [`TopologyDispatch::dispatch_credential_hook`];
    /// the resource handle the hook needs is supplied from `self.resource`.
    ///
    /// # Cancel Safety
    ///
    /// This method is cancel-safe. The resource taint and revoke-epoch bump
    /// are performed synchronously by the caller before this future is polled.
    /// Dropping the returned future after taint leaves the resource
    /// consistently marked as tainted — no partial-taint state is possible and
    /// new acquires remain rejected.
    pub(crate) async fn dispatch_slot_hook(&self, slot: &str, refresh: bool) -> Result<(), Error> {
        self.topology
            .dispatch_credential_hook(&self.resource, slot, refresh)
            .await
    }

    /// The topology tag for rotation / diagnostic spans.
    pub(crate) fn topology_tag(&self) -> TopologyTag {
        self.topology.tag()
    }

    /// Admission phase snapshot from the topology.
    pub(crate) fn admission_phase(&self) -> AdmissionPhase {
        self.topology.phase(&self.store)
    }

    /// Admission load snapshot from the topology.
    pub(crate) fn admission_load(&self) -> Option<Load> {
        self.topology.load(&self.store)
    }

    /// Sync capacity gate from the topology (the ticket is dropped — this is a
    /// yes/no gate with a typed reason).
    pub(crate) fn try_reserve_gate(&self) -> Result<(), Unavailable> {
        self.topology.try_reserve(&self.store).map(|_ticket| ())
    }
}
