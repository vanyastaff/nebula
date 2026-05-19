//! Per-registration runtime holding topology + metadata.
//!
//! [`ManagedResource`] is the internal representation of a registered
//! resource. It bundles the resource implementation, hot-swappable config,
//! topology runtime, release queue, and lifecycle metadata.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use arc_swap::ArcSwap;
use tokio::sync::Notify;

use super::TopologyRuntime;
use crate::{
    error::Error,
    integration::AcquireResilience,
    recovery::RecoveryGate,
    release_queue::ReleaseQueue,
    resource::Resource,
    state::{ResourcePhase, ResourceStatus},
};

/// Per-registration runtime holding topology + metadata.
///
/// Created once per `Manager::register()` call and stored for the
/// lifetime of the resource. The `config` and `status` fields are
/// atomically swappable for hot-reload.
pub struct ManagedResource<R: Resource> {
    /// The resource implementation (topology trait impl).
    pub(crate) resource: R,
    /// Hot-swappable operational configuration.
    pub(crate) config: ArcSwap<R::Config>,
    /// Topology-specific runtime state.
    pub(crate) topology: TopologyRuntime<R>,
    /// Background worker pool for async cleanup.
    pub(crate) release_queue: Arc<ReleaseQueue>,
    /// Monotonically increasing generation counter (bumped on reload).
    pub(crate) generation: AtomicU64,
    /// Current lifecycle status (phase + last error).
    pub(crate) status: ArcSwap<ResourceStatus>,
    /// Optional resilience configuration (timeout + retry) for acquire.
    pub(crate) resilience: Option<AcquireResilience>,
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

impl<R: Resource> ManagedResource<R> {
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
    /// and shutdown (#387).
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
    /// Wired by `Manager::set_phase_all_failed` (R-023): when
    /// `DrainTimeoutPolicy::Abort` fires we transition every registered
    /// resource to `Failed` so callers cannot subsequently acquire a
    /// resource the manager has already declared bankrupt. Per-resource
    /// `HealthChanged{healthy:false}` event emission is owned by the
    /// manager because it holds the broadcast channel.
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

    /// Advances the credential-revoke counter for a pooled topology so
    /// every pool return-to-idle path destroys (never recycles or admits)
    /// an instance authenticated with the now-revoked credential.
    ///
    /// Called synchronously by `Manager::revoke_slot` in phase 1, before the
    /// revoke hook is dispatched — the same pre-`.await` discipline as
    /// [`taint`](Self::taint). Only the [`Pool`](TopologyRuntime::Pool)
    /// topology has an idle queue and the recycle / in-flight-create /
    /// warmup / maintenance return-to-idle paths this counter guards; the
    /// single-runtime topologies hold one shared `Arc<R::Runtime>` and
    /// dispatch the revoke hook directly against it under no idle-queue race,
    /// so there is no return-to-idle site to fence and this is a no-op for
    /// them. See the [`manager`](crate::manager) module docs for the
    /// canonical revoke-epoch-fence rationale.
    pub(crate) fn bump_revoke_epoch(&self) {
        if let TopologyRuntime::Pool(rt) = &self.topology {
            rt.bump_revoke_epoch();
        }
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

    /// Borrows the live runtime(s) for this topology and invokes the
    /// per-slot credential hook — [`Resource::on_credential_refresh`] when
    /// `refresh` is `true`, [`Resource::on_credential_revoke`] otherwise.
    ///
    /// Single-runtime topologies dispatch once against the shared runtime;
    /// Pool dispatches per idle instance (delegating to
    /// [`PoolRuntime::dispatch_slot_hook_over_idle`](super::pool::PoolRuntime::dispatch_slot_hook_over_idle),
    /// which carries the same `refresh` selector). The `Bounded` arm is
    /// the consolidated single-runtime hook: it subsumes the three former
    /// `Service` / `Transport` / `Exclusive` arms (all of which held one
    /// shared `Arc<R::Runtime>` and dispatched the hook against it exactly
    /// once) into one borrow of [`BoundedRuntime::runtime`](super::bounded::BoundedRuntime::runtime).
    /// The `Service` / `Transport` / `Exclusive` arms are kept verbatim
    /// while their construction sites are migrated onto `Bounded`.
    ///
    /// **Topology audit of the `current() == None → Ok(())` stale-skip
    /// (per-resource revoke deferral / #680).** Only **Resident** lazily builds its
    /// runtime internally via `resource.create()` (under its `create_lock`,
    /// with a `None`-cell window), so only Resident had the lost-update
    /// where a rotation racing the first `create` could be recorded as a
    /// success with the hook never delivered. Its dispatch now goes through
    /// [`ResidentRuntime::dispatch_resident_hook`](super::resident::ResidentRuntime::dispatch_resident_hook),
    /// which serialises against `create` on the same lock and reconciles a
    /// runtime built against an older credential epoch instead of silently
    /// succeeding. The other arms do **not** share the defect:
    /// Service / Transport / Exclusive / Bounded take a caller-supplied
    /// runtime at register time (no `None` window — the hook is always
    /// delivered); Pool dispatches over every idle entry and rebuilds
    /// fresh instances against the current (lock-free) slot, so an empty
    /// idle queue masks no stale-bound runtime.
    ///
    /// The `refresh` flag selects the hook exactly once per topology arm
    /// (mirroring the pool selector); both directions share identical
    /// per-topology runtime-borrow semantics.
    pub(crate) async fn dispatch_slot_hook(&self, slot: &str, refresh: bool) -> Result<(), Error> {
        match &self.topology {
            // Reconcile-aware (per-resource revoke deferral / #680): serialises
            // against the resident `create` slow path and re-delivers the
            // hook to a runtime built against an older credential epoch
            // rather than skipping with a false success.
            TopologyRuntime::Resident(rt) => {
                rt.dispatch_resident_hook(&self.resource, slot, refresh)
                    .await
            },
            TopologyRuntime::Service(rt) => {
                self.invoke_slot_hook(slot, refresh, rt.runtime()).await
            },
            TopologyRuntime::Transport(rt) => {
                self.invoke_slot_hook(slot, refresh, rt.runtime()).await
            },
            TopologyRuntime::Exclusive(rt) => {
                self.invoke_slot_hook(slot, refresh, rt.runtime()).await
            },
            // Consolidated single-runtime hook: one shared
            // `Arc<R::Runtime>`, dispatched once. Subsumes the three
            // arms above for runtimes constructed as `Bounded`.
            TopologyRuntime::Bounded(rt) => {
                self.invoke_slot_hook(slot, refresh, rt.runtime()).await
            },
            TopologyRuntime::Pool(rt) => rt
                .dispatch_slot_hook_over_idle(&self.resource, slot, refresh)
                .await
                .map_err(Into::into),
        }
    }

    /// Invokes the selected `&self` credential hook against one borrowed
    /// runtime. Single-runtime topologies call this once; Pool uses its
    /// own per-idle fan-out. The `refresh` selector is applied here so the
    /// per-topology match in [`dispatch_slot_hook`](Self::dispatch_slot_hook)
    /// stays written once.
    async fn invoke_slot_hook(
        &self,
        slot: &str,
        refresh: bool,
        runtime: &R::Runtime,
    ) -> Result<(), Error> {
        let res = if refresh {
            self.resource.on_credential_refresh(slot, runtime).await
        } else {
            self.resource.on_credential_revoke(slot, runtime).await
        };
        res.map_err(Into::into)
    }
}
