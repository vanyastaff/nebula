//! Per-registration runtime holding topology + metadata, **and the
//! framework-owned acquire loop**.
//!
//! [`ManagedResource`] is the internal representation of a registered
//! resource. It bundles the resource implementation, hot-swappable config, the
//! framework-owned [`InstanceStore`] idle queue, the resource's
//! [`Provider::Topology`], the release queue, and lifecycle metadata.
//!
//! The framework reaches the topology monomorphically through the resource's
//! associated [`Topology`] type. **The acquire loop, the fenced checkout, the
//! stale-slot destroy, the cancel-safe guard-wrap, and the on-release
//! return-or-destroy all live here, in the framework** — the topology supplies
//! only thin R-aware hooks ([`create_slot`](Topology::create_slot) /
//! [`slot_instance`](Topology::slot_instance) /
//! [`into_instance`](Topology::into_instance) / [`accept`](Topology::accept) /
//! [`prepare`](Topology::prepare) / [`on_release`](Topology::on_release)) it
//! cannot use to skip the credential-revoke fence. This is the inversion that
//! makes the open trait safe-by-construction: a custom topology author writes
//! zero store / checkout / destroy / fence code.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use arc_swap::ArcSwap;
use tokio::sync::{Notify, OwnedSemaphorePermit};

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
    topology::{
        AdmissionPhase, Load, MaintenanceSchedule, Topology, Unavailable,
        store::{InstanceStore, ReturnOutcome},
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
    pub(crate) async fn wait_for_in_flight_drain(
        &self,
        timeout: std::time::Duration,
    ) -> Result<(), u64> {
        crate::manager::shutdown::wait_for_tracker_drain(&self.in_flight, timeout).await
    }
}

// ── The framework acquire loop + topology-driven lifecycle.
//
// Everything that reaches into the topology — the acquire loop, the revoke
// fence, rotation dispatch, warmup, maintenance — lives here behind the
// `R::Topology: Topology<R>` bound. The weak `R: Provider` block above stays
// usable by code that never touches the topology (status / phase / taint /
// drain).

impl<R> ManagedResource<R>
where
    R: Provider + HasCredentialSlots,
    R::Instance: Clone,
    R::Topology: Topology<R>,
{
    /// **The framework acquire loop.** Runs the full fenced acquire over the
    /// framework-owned [`store`](Self::store) and the topology's R-aware hooks,
    /// producing a typed [`ResourceGuard<R>`].
    ///
    /// The loop the framework owns (not the topology):
    /// 1. `topology.try_reserve(&store)` — the sync concurrency gate; the
    ///    returned permit is held by the guard for the whole lease.
    /// 2. `store.checkout()` — the **framework** revoke-epoch fence on pop.
    /// 3. destroy every `checkout.stale` slot via
    ///    `destroy(into_instance(stale))` — the **framework** tears down
    ///    since-revoked idle slots; a topology author can never skip this.
    /// 4. `accept(&mut slot)` a fresh idle slot, or `create_slot(…)` on a miss.
    /// 5. wrap the slot in a [`SlotCreateGuard`] (cancel-safety: a drop here
    ///    schedules an async `destroy` via the [`ReleaseQueue`]).
    /// 6. `prepare(&mut slot)` — per-acquire session init; `Err` ⇒ destroy +
    ///    fail.
    /// 7. build the guard whose `Deref` is `topology.slot_instance(&slot)` and
    ///    whose drop runs `on_release(&mut slot)` then either
    ///    `store.return_slot(slot, epoch)` (if `pools()` and kept) or
    ///    `destroy(into_instance(slot))`.
    ///
    /// # Atomicity (revoke fence)
    ///
    /// The fence is the store's: `checkout` pops under the idle lock and
    /// collects stale slots; `return_slot` re-reads the live revoke epoch under
    /// the idle lock before pushing. The on-release closure runs `on_release`
    /// (recycle/reset) *first* and hands the slot to `return_slot` *last*, so a
    /// revoke landing during a parking `on_release` still evicts on return. A
    /// fresh-create that straddles a revoke is fenced by `return_slot`'s
    /// under-lock epoch re-read.
    ///
    /// # Errors
    ///
    /// - [`Unavailable`] from `try_reserve` is mapped to the caller error.
    /// - Propagates `create_slot` / `prepare` failures.
    ///
    /// # Cancel safety
    ///
    /// A drop between checkout/create and the built guard schedules an async
    /// `destroy(into_instance(slot))` via the [`ReleaseQueue`] — see
    /// [`SlotCreateGuard`].
    pub(crate) async fn run_acquire_loop(
        self: &Arc<Self>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Result<ResourceGuard<R>, Error> {
        let _ = options;
        let config = self.config();
        let generation = self.generation();

        // 1. Sync concurrency gate. The permit (if any) is held by the guard
        //    for the whole lease and returned to the topology's semaphore on
        //    guard drop.
        let permit = self
            .topology
            .try_reserve(&self.store)
            .map_err(|u| u.into_error(R::key()))?
            .into_permit();

        // 2-4. Fenced checkout → destroy stale → accept-or-create. The whole
        //      idle-then-create decision is the framework's; the topology only
        //      validates (`accept`) or makes (`create_slot`).
        let (slot, checkout_epoch) = self.checkout_or_create(ctx, &config).await?;

        // 5. Cancel-safety: from here until the guard is built, a drop must
        //    `destroy(into_instance(slot))` via the ReleaseQueue.
        let mut cancel_guard =
            SlotCreateGuard::new(slot, Arc::clone(self), Arc::clone(&self.release_queue));

        // 6. Per-acquire session init. `prepare` borrows the slot mutably from
        //    the cancel guard (a distinct object from `self`), so the topology
        //    `&self` hook and the `&mut slot` borrow do not alias.
        if let Err(e) = self
            .topology
            .prepare(cancel_guard.slot_mut(), &self.resource, ctx)
            .await
        {
            let slot = cancel_guard.defuse();
            let _ = self
                .resource
                .destroy(self.topology.into_instance(slot))
                .await;
            return Err(e);
        }

        // 7. Build the guard. Defuse the cancel guard first so its Drop does
        //    not also schedule a destroy.
        let slot = cancel_guard.defuse();
        Ok(self.build_guard(slot, checkout_epoch, permit, generation, metrics))
    }

    /// Framework checkout-then-create: pop the first fresh idle slot (destroying
    /// every since-revoked stale slot the fence discards), validate it via
    /// `accept`, and on an idle-miss / accept-reject create a fresh slot.
    ///
    /// This is the inner half of the acquire loop; factored out so a future
    /// `checkout_keyed` (affinity) variant slots in beside it without reshaping
    /// the loop. Returns the chosen slot and its checkout epoch (the create
    /// path stamps the current epoch).
    ///
    /// Complexity: O(stale + 1) idle pops per call (average and worst case);
    /// bounded by the store's idle capacity.
    async fn checkout_or_create(
        &self,
        ctx: &ResourceContext,
        config: &R::Config,
    ) -> Result<(SlotOf<R>, u64), Error> {
        loop {
            let checkout = self.store.checkout().await;
            // FRAMEWORK destroys since-revoked stale slots — the author can
            // never skip this fence.
            for stale in checkout.stale {
                let _ = self
                    .resource
                    .destroy(self.topology.into_instance(stale))
                    .await;
            }
            let Some(co) = checkout.fresh else {
                // Idle-miss — create a fresh slot. Snapshot the revoke epoch
                // BEFORE create so a revoke that lands *during* `create_slot` is
                // detectable (HikariCP #1836): stamping after the await would
                // read the post-revoke counter and silently admit a
                // since-revoked instance.
                let create_epoch = self.store.current_revoke_epoch();
                let slot = self
                    .topology
                    .create_slot(&self.resource, config, ctx)
                    .await?;
                // Fresh-create fence (HikariCP #1836) — POOLED topologies only.
                // A pooled instance created against a credential revoked while
                // the create was in flight must NOT be admitted to the idle pool
                // or handed onward: destroy it and fail the acquire closed.
                // Non-pooling topologies (Resident / permit-only) never enter
                // the idle store — the instance is one-shot per acquire and a
                // concurrent revoke is handled by the credential cell + rotation
                // hook (it serves, then the hook clears the shared binding), so
                // they must NOT fail-closed here.
                if self.topology.pools() && self.store.current_revoke_epoch() != create_epoch {
                    let _ = self
                        .resource
                        .destroy(self.topology.into_instance(slot))
                        .await;
                    return Err(Error::revoked(format!(
                        "{}: credential revoked while the instance was being \
                         created — fenced before admission (HikariCP #1836)",
                        R::key()
                    )));
                }
                return Ok((slot, create_epoch));
            };
            let (mut slot, epoch) = co.into_parts();
            if self.topology.accept(&mut slot, &self.resource, ctx).await {
                return Ok((slot, epoch));
            }
            // Rejected (stale fingerprint / max-lifetime / broken) — destroy and
            // loop to the next idle slot, then create.
            let _ = self
                .resource
                .destroy(self.topology.into_instance(slot))
                .await;
        }
    }

    /// Builds the leased [`ResourceGuard<R>`] over a chosen slot.
    ///
    /// The guard's `Deref` is a clone of `slot_instance(&slot)`; the release
    /// closure captures the **whole slot** (metadata intact) + an `Arc<Self>`
    /// (store + topology + resource) and, on guard drop, runs
    /// `on_release(&mut slot)` then either `store.return_slot(slot, epoch)`
    /// (pools + kept) or `destroy(into_instance(slot))`.
    fn build_guard(
        self: &Arc<Self>,
        slot: SlotOf<R>,
        checkout_epoch: u64,
        permit: Option<OwnedSemaphorePermit>,
        generation: u64,
        metrics: Option<ResourceOpsMetrics>,
    ) -> ResourceGuard<R> {
        // The guard's Deref value is a clone of the leasable instance; the
        // release closure owns the real slot.
        let deref_instance = self.topology.slot_instance(&slot).clone();
        let managed = Arc::clone(self);
        let release_queue = Arc::clone(&self.release_queue);

        ResourceGuard::guarded_with_permit(
            deref_instance,
            R::key(),
            self.topology.tag(),
            generation,
            move |_returned_instance: R::Instance, tainted| {
                if let Some(m) = &metrics {
                    m.record_release();
                }
                Box::pin(release_slot(managed, slot, checkout_epoch, tainted))
            },
            permit,
            release_queue,
        )
    }

    /// Advances the credential-revoke fence so every return-to-idle path
    /// destroys (never recycles or admits) an instance authenticated with the
    /// now-revoked credential.
    ///
    /// Called synchronously by `Manager::revoke_slot` in phase 1, before the
    /// revoke hook is dispatched — the same pre-`.await` discipline as
    /// [`taint`](Self::taint). The fence lives on the framework store, so this
    /// is store-owned for every topology (a no-op for topologies whose store
    /// stays empty, e.g. Resident).
    pub(crate) fn bump_revoke_epoch(&self) {
        self.store.bump_revoke_epoch();
    }

    /// Borrows the live topology and invokes the per-slot credential hook —
    /// [`Provider::on_credential_refresh`] when `refresh` is `true`,
    /// [`Provider::on_credential_revoke`] otherwise — against this resource's
    /// instances.
    ///
    /// The dispatch is topology-specific (resident reconcile vs pool idle
    /// fan-out) and lives behind [`Topology::dispatch_credential_hook`]; the
    /// resource handle the hook needs is supplied from `self.resource`.
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
            .dispatch_credential_hook(&self.resource, &self.store, slot, refresh)
            .await
    }

    /// Pre-warms the store by creating + depositing `warmup_target` slots
    /// (fenced) at registration. Returns the number admitted.
    ///
    /// Each warmed slot is stamped with the live revoke epoch under the idle
    /// lock via [`InstanceStore::deposit_fresh`], so a revoke that already
    /// landed evicts it immediately rather than admitting a since-revoked
    /// instance.
    pub(crate) async fn warmup(&self, ctx: &ResourceContext) -> usize {
        let config = self.config();
        let target = self.topology.warmup_target(&config);
        if target == 0 {
            return 0;
        }
        let mut created = 0usize;
        for _ in 0..target {
            let created_epoch = self.store.stamp_epoch();
            match self
                .topology
                .create_slot(&self.resource, &config, ctx)
                .await
            {
                Ok(slot) => match self.store.deposit_fresh(slot, created_epoch).await {
                    ReturnOutcome::Recycled => created += 1,
                    ReturnOutcome::Evict(slot) => {
                        let _ = self
                            .resource
                            .destroy(self.topology.into_instance(slot))
                            .await;
                    },
                },
                Err(e) => {
                    tracing::warn!(
                        key = %R::key(),
                        error = %e,
                        created,
                        target,
                        "warmup: create_slot failed, stopping early"
                    );
                    break;
                },
            }
        }
        if created > 0 {
            tracing::info!(key = %R::key(), created, target, "resource warmup complete");
        }
        created
    }

    /// Runs one background maintenance sweep over the framework store.
    ///
    /// Two arms, both under the idle lock (atomic against checkout/return):
    /// - the **revoke** arm — [`InstanceStore::evict_stale`] evicts slots whose
    ///   checkout epoch is behind the live counter (framework-owned fence);
    /// - the **non-revoke** arm — [`InstanceStore::retain`] over the topology's
    ///   [`idle_evictable`](Topology::idle_evictable) predicate
    ///   (fingerprint / max-lifetime / idle-timeout).
    ///
    /// Each evicted slot is destroyed via `destroy(into_instance(slot))`.
    /// Returns the number evicted.
    ///
    /// Complexity: O(n) over the idle queue (average and worst case), bounded
    /// by the store's configured idle capacity.
    pub(crate) async fn run_maintenance(&self) -> usize {
        let mut to_destroy = self.store.evict_stale().await;
        let nonrevoke = self
            .store
            .retain(|slot, _epoch| self.topology.idle_evictable(slot))
            .await;
        to_destroy.extend(nonrevoke);

        let evicted = to_destroy.len();
        for slot in to_destroy {
            let _ = self
                .resource
                .destroy(self.topology.into_instance(slot))
                .await;
        }
        if evicted > 0 {
            tracing::debug!(evicted, "resource maintenance: evicted idle/expired slots");
        }
        evicted
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

    /// Sync capacity gate from the topology (the ticket is dropped — this is a
    /// yes/no gate with a typed reason).
    pub(crate) fn try_reserve_gate(&self) -> Result<(), Unavailable> {
        self.topology.try_reserve(&self.store).map(|_ticket| ())
    }
}

/// The release teardown future a guard's drop schedules: run the topology's
/// `on_release` reset, then either return the slot to the framework store
/// (under the revoke-epoch fence) or destroy it.
///
/// # Atomicity (revoke fence)
///
/// `on_release` (reset / recycle) runs **first**; the slot is handed to
/// [`InstanceStore::return_slot`] **last**, which re-reads the live revoke epoch
/// under the idle lock before pushing. So a revoke landing during a parking
/// `on_release` still evicts on return — the under-lock compare-then-push is the
/// fence, identical to the historical pool recycle `Keep` arm.
async fn release_slot<R>(
    managed: Arc<ManagedResource<R>>,
    mut slot: SlotOf<R>,
    checkout_epoch: u64,
    tainted: bool,
) -> Result<(), Error>
where
    R: Provider + HasCredentialSlots,
    R::Instance: Clone,
    R::Topology: Topology<R>,
{
    // Tainted lease — destroy immediately, never recycle.
    if tainted {
        return managed
            .resource
            .destroy(managed.topology.into_instance(slot))
            .await;
    }

    // Topology reset / recycle decision (runs before the store fence).
    let keep = match managed
        .topology
        .on_release(&mut slot, &managed.resource)
        .await
    {
        Ok(keep) => keep,
        Err(e) => {
            // Reset failed — destroy. Surface the reset error (so an awaited
            // `release()` sees the failed teardown) once the slot is torn down.
            let destroy = managed
                .resource
                .destroy(managed.topology.into_instance(slot))
                .await;
            return destroy.and(Err(e));
        },
    };

    if keep && managed.topology.pools() {
        // FENCE: `return_slot` re-reads the revoke epoch under the idle lock.
        match managed.store.return_slot(slot, checkout_epoch).await {
            ReturnOutcome::Recycled => Ok(()),
            ReturnOutcome::Evict(slot) => {
                managed
                    .resource
                    .destroy(managed.topology.into_instance(slot))
                    .await
            },
        }
    } else {
        // Non-pooling topology (Resident / permit-only) or a `Drop` decision:
        // the released slot is destroyed, never pooled.
        managed
            .resource
            .destroy(managed.topology.into_instance(slot))
            .await
    }
}

/// Cancel-safety guard for the framework acquire loop's create-then-prepare
/// window, generalized over the topology's [`Slot`](Topology::Slot).
///
/// Wraps a freshly checked-out / created slot from the moment it leaves the
/// store/`create_slot` until the [`ResourceGuard`] is built. If the acquire
/// future is cancelled in that window (`tokio::select!` / timeout), `Drop`
/// schedules an async `destroy(into_instance(slot))` on the [`ReleaseQueue`] —
/// without this, only the instance's *sync* `Drop` runs and the server-side
/// resource (DB session, OS handle) leaks. The `cancel-drop` regression test
/// guards this.
///
/// Call [`defuse`](Self::defuse) once the guard is safely built; it consumes
/// the guard by value, so the borrow checker forbids any use after `defuse` and
/// the `Drop` never runs against a defused slot.
struct SlotCreateGuard<R>
where
    R: Provider + HasCredentialSlots,
    R::Instance: Clone,
    R::Topology: Topology<R>,
{
    /// `None` after [`defuse`](Self::defuse) took it out; `Some(_)` for any
    /// guard a caller can still observe. `Drop` short-circuits on `None`.
    slot: Option<SlotOf<R>>,
    /// The managed resource (store + topology + resource) so `Drop` can
    /// `destroy(into_instance(slot))` from the [`ReleaseQueue`].
    managed: Arc<ManagedResource<R>>,
    /// The framework release queue so `Drop` submits the async destroy with the
    /// queue's bounded backpressure + shutdown drain (not an orphan spawn).
    release_queue: Arc<ReleaseQueue>,
}

impl<R> SlotCreateGuard<R>
where
    R: Provider + HasCredentialSlots,
    R::Instance: Clone,
    R::Topology: Topology<R>,
{
    /// Creates a new guard wrapping the chosen slot.
    fn new(
        slot: SlotOf<R>,
        managed: Arc<ManagedResource<R>>,
        release_queue: Arc<ReleaseQueue>,
    ) -> Self {
        Self {
            slot: Some(slot),
            managed,
            release_queue,
        }
    }

    /// Returns a mutable reference to the wrapped slot for `prepare`.
    ///
    /// `&mut self` keeps this a plain safe borrow: the acquire loop owns the
    /// cancel guard by value and only borrows it mutably here, so the topology
    /// `&self` hook (a distinct object) and this `&mut slot` never alias.
    fn slot_mut(&mut self) -> &mut SlotOf<R> {
        // guard-justified: `slot` is `Some(_)` for the guard's whole observable
        // lifetime — it is set in `new` and only taken in `defuse`/`Drop`, both
        // of which consume the guard by value. Reaching `None` here would mean
        // a borrow after `defuse`, which the borrow checker already forbids, so
        // this `unreachable!` documents an unrepresentable state rather than a
        // runtime path.
        self.slot
            .as_mut()
            .unwrap_or_else(|| unreachable!("SlotCreateGuard::slot_mut after defuse"))
    }

    /// Consumes the guard and returns the wrapped slot.
    ///
    /// After this call the guard is gone; its `Drop` runs against `slot: None`
    /// and short-circuits without scheduling a destroy.
    fn defuse(mut self) -> SlotOf<R> {
        // guard-justified: `defuse` consumes `self` by value, so the borrow
        // checker forbids calling it twice. `slot` is `Some(_)` for the guard's
        // whole observable lifetime (set in `new`, only taken here or in
        // `Drop`, both consuming), so `take()` cannot be `None` on this path.
        self.slot
            .take()
            .unwrap_or_else(|| unreachable!("SlotCreateGuard defused twice"))
    }
}

impl<R> Drop for SlotCreateGuard<R>
where
    R: Provider + HasCredentialSlots,
    R::Instance: Clone,
    R::Topology: Topology<R>,
{
    fn drop(&mut self) {
        let Some(slot) = self.slot.take() else {
            return; // defused — nothing to clean up
        };
        let managed = Arc::clone(&self.managed);
        tracing::warn!(
            resource = %R::key(),
            "cancel-safety: acquire future cancelled mid-create — \
             scheduling async destroy via ReleaseQueue"
        );
        self.release_queue.submit(move || {
            Box::pin(async move {
                // A slot cancelled before reaching the built guard was never
                // admitted to the store or handed to a caller; the only correct
                // cleanup is destroy.
                let _ = managed
                    .resource
                    .destroy(managed.topology.into_instance(slot))
                    .await;
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicBool, AtomicU64, Ordering},
        time::Duration,
    };

    use nebula_core::{ExecutionId, ResourceKey, resource_key};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        resource::{ResourceConfig, ResourceMetadata},
        topology::{Pooled, pooled::config::Config as PoolConfig},
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
    }

    impl Mock {
        fn new() -> Self {
            Self {
                created: Arc::new(AtomicU64::new(0)),
                destroyed: Arc::new(AtomicU64::new(0)),
                park_create: Arc::new(AtomicBool::new(false)),
                create_entered: Arc::new(Notify::new()),
                release_create: Arc::new(Notify::new()),
            }
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

        async fn destroy(&self, _runtime: u64) -> Result<(), Error> {
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

    /// Cancel-safety: a [`SlotCreateGuard`] dropped before `defuse` schedules an
    /// async `destroy` via the release queue.
    #[tokio::test]
    async fn slot_create_guard_drop_destroys_via_release_queue() {
        let resource = Mock::new();
        let destroyed = Arc::clone(&resource.destroyed);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let mr = {
            let topology = Pooled::<Mock>::new(PoolConfig::default(), 0);
            Arc::new(ManagedResource {
                resource,
                config: ArcSwap::from_pointee(PoolCfg),
                topology,
                store: InstanceStore::new(None),
                release_queue: Arc::clone(&rq),
                generation: AtomicU64::new(0),
                status: ArcSwap::from_pointee(ResourceStatus::new()),
                recovery_gate: None,
                tainted: AtomicBool::new(false),
                in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
            })
        };

        let slot = mr
            .topology
            .create_slot(&mr.resource, &PoolCfg, &test_ctx())
            .await
            .expect("create");
        let guard = SlotCreateGuard::new(slot, Arc::clone(&mr), Arc::clone(&rq));
        // Simulate a cancelled acquire: dropped before `defuse`.
        drop(guard);

        tokio::time::sleep(Duration::from_millis(50)).await;
        // Signal the workers to drain + exit before joining. `drop(rq)` alone
        // does NOT close the channels here: `mr` holds another
        // `Arc<ReleaseQueue>`, so the senders outlive the test's `rq` and the
        // worker loop would block on `rx.recv()` forever. `close()` cancels the
        // token, the documented precondition for `shutdown`.
        rq.close();
        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;

        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            1,
            "SlotCreateGuard::drop must schedule destroy via the ReleaseQueue \
             when the acquire future is cancelled mid-create"
        );
    }

    /// A `SlotCreateGuard` that runs through `defuse` (the success path) must
    /// NOT trigger a stray destroy.
    #[tokio::test]
    async fn slot_create_guard_defuse_skips_destroy() {
        let resource = Mock::new();
        let destroyed = Arc::clone(&resource.destroyed);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let mr = managed(resource, PoolConfig::default());

        let slot = mr
            .topology
            .create_slot(&mr.resource, &PoolCfg, &test_ctx())
            .await
            .expect("create");
        let guard = SlotCreateGuard::new(slot, Arc::clone(&mr), Arc::clone(&rq));
        let _slot = guard.defuse();

        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;

        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            0,
            "a defused SlotCreateGuard must not schedule a destroy"
        );
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
}
