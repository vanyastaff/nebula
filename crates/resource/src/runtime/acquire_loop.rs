//! Framework-owned acquire loop and cancel-safety guard for [`ManagedResource`].
//!
//! This module holds the `impl` block that requires the full
//! `R: Provider + HasCredentialSlots, R::Instance: Clone, R::Topology:
//! Topology<R>` bound — everything that reaches into the topology:
//!
//! - [`run_acquire_loop`](ManagedResource::run_acquire_loop) — the fenced
//!   acquire: reserve → checkout → destroy stale → accept-or-create → prepare
//!   → build guard.
//! - [`checkout_or_create`] — inner loop factored out for future keyed variants.
//! - [`build_guard`] — assembles the [`ResourceGuard`] with its release closure.
//! - [`bump_revoke_epoch`](ManagedResource::bump_revoke_epoch) /
//!   [`dispatch_slot_hook`](ManagedResource::dispatch_slot_hook) — credential
//!   rotation hooks that need topology dispatch.
//! - [`warmup`](ManagedResource::warmup) / [`run_maintenance`](ManagedResource::run_maintenance) /
//!   [`probe_idle_slots`] — lifecycle maintenance driven by the registry reaper.
//! - [`release_slot`] — the async release teardown future the guard's drop schedules.
//! - [`SlotCreateGuard`] — cancel-safety RAII guard for the create-then-prepare window.
//!
//! The weak `impl<R: Provider>` block (status / phase / taint / drain) stays in
//! [`managed`](super::managed) so code that never touches the topology can import
//! a lighter set of bounds.

use std::sync::Arc;

use tokio::sync::OwnedSemaphorePermit;

use crate::{
    context::ResourceContext,
    error::Error,
    guard::ResourceGuard,
    metrics::{RecycleOutcome, ResourceOpsMetrics},
    options::AcquireOptions,
    release_queue::ReleaseQueue,
    resource::{HasCredentialSlots, Provider, TeardownReason},
    runtime::{managed::ManagedResource, teardown::destroy_within},
    topology::{Topology, store::ReturnOutcome},
};

use super::managed::SlotOf;

// ── The framework acquire loop + topology-driven lifecycle.
//
// Everything that reaches into the topology — the acquire loop, the revoke
// fence, rotation dispatch, warmup, maintenance — lives here behind the
// `R::Topology: Topology<R>` bound. The weak `R: Provider` block in managed.rs
// stays usable by code that never touches the topology (status / phase / taint /
// drain).

impl<R> ManagedResource<R>
where
    R: Provider + HasCredentialSlots,
    R::Instance: Clone,
    R::Topology: Topology<R>,
{
    /// **The framework acquire loop.** Runs the full fenced acquire over the
    /// framework-owned [`store`](ManagedResource::store) and the topology's R-aware hooks,
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
    /// - [`Unavailable`](crate::topology::Unavailable) from `try_reserve` is mapped to the caller error.
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
            let _ = destroy_within(
                &self.resource,
                self.topology.into_instance(slot),
                TeardownReason::Released,
            )
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
                let _ = destroy_within(
                    &self.resource,
                    self.topology.into_instance(stale),
                    TeardownReason::Revoked,
                )
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
                    let _ = destroy_within(
                        &self.resource,
                        self.topology.into_instance(slot),
                        TeardownReason::Revoked,
                    )
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
            let _ = destroy_within(
                &self.resource,
                self.topology.into_instance(slot),
                TeardownReason::Evicted,
            )
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
                Box::pin(release_slot(
                    managed,
                    slot,
                    checkout_epoch,
                    tainted,
                    metrics,
                ))
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
    /// [`taint`](ManagedResource::taint). The fence lives on the framework store, so this
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
    /// lock via [`crate::topology::store::InstanceStore::deposit_fresh`], so a revoke that already
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
                        let _ = destroy_within(
                            &self.resource,
                            self.topology.into_instance(slot),
                            TeardownReason::Evicted,
                        )
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
    /// Three arms, all under the idle lock (atomic against checkout/return):
    /// - the **revoke** arm — [`crate::topology::store::InstanceStore::evict_stale`] evicts slots whose
    ///   checkout epoch is behind the live counter (framework-owned fence);
    /// - the **non-revoke** arm — [`crate::topology::store::InstanceStore::retain`] over the topology's
    ///   [`idle_evictable`](Topology::idle_evictable) predicate
    ///   (fingerprint / max-lifetime / idle-timeout);
    /// - the **health-probe** arm — [`probe_idle_slots`](Self::probe_idle_slots)
    ///   runs [`Provider::check`] over idle slots, but **only on sweeps where
    ///   the resource's [`CheckCost`](crate::CheckCost) cadence is due**, so an
    ///   expensive check is not run every sweep.
    ///
    /// Each evicted/failed slot is destroyed via `destroy(into_instance(slot))`.
    /// Returns the number evicted.
    ///
    /// Complexity: O(n) over the idle queue (average and worst case), bounded
    /// by the store's configured idle capacity; the probe arm adds at most one
    /// `check` per idle slot on a due sweep.
    pub(crate) async fn run_maintenance(&self) -> usize {
        use std::sync::atomic::Ordering;

        let mut to_destroy = self.store.evict_stale().await;
        let nonrevoke = self
            .store
            .retain(|slot, _epoch| self.topology.idle_evictable(slot))
            .await;
        to_destroy.extend(nonrevoke);

        // Cost-aware health probe: only run `check` over idle slots on sweeps
        // where the resource's check cost says it is due (Cheap every sweep,
        // Expensive every 16th), so a network-round-trip check is not run on
        // every sweep over a pool of idle connections.
        let sweep = self.maintenance_sweeps.fetch_add(1, Ordering::Relaxed) + 1;
        let cadence = self.resource.check_cost().probe_every_n_sweeps();
        let mut probe_evicted = 0;
        if cadence != 0 && sweep.is_multiple_of(cadence) {
            let failed = self.probe_idle_slots().await;
            probe_evicted = failed.len();
            to_destroy.extend(failed);
        }

        let evicted = to_destroy.len();
        for slot in to_destroy {
            let _ = destroy_within(
                &self.resource,
                self.topology.into_instance(slot),
                TeardownReason::Evicted,
            )
            .await;
        }
        if evicted > 0 {
            tracing::debug!(
                evicted,
                probe_evicted,
                "resource maintenance: evicted idle/expired/unhealthy slots"
            );
        }
        evicted
    }

    /// Health-probes every idle slot via [`Provider::check`], removing and
    /// returning the slots that fail so the caller destroys them.
    ///
    /// Holds the idle lock across the probe awaits (head-of-line-blocking
    /// against checkout/return for the probe's duration — the same discipline
    /// the credential-rotation dispatch uses). The cost-aware cadence in
    /// [`run_maintenance`](Self::run_maintenance) is what bounds how often this
    /// runs, so an expensive `check` does not block the pool every sweep. A slot
    /// that passes is returned to the queue with its checkout epoch intact, so
    /// the revoke fence is unaffected.
    ///
    /// Complexity: O(n) checks over the idle queue (average and worst case),
    /// bounded by the store's configured idle capacity.
    async fn probe_idle_slots(&self) -> Vec<SlotOf<R>> {
        let key = R::key();
        let mut idle = self.store.lock_idle().await;
        let mut kept = std::collections::VecDeque::with_capacity(idle.len());
        let mut failed = Vec::new();
        while let Some(entry) = idle.pop_front() {
            // Route the author's `check` through the bound+isolate chokepoint
            // like every other author hook: a probe that hangs is cut at the
            // ceiling (bounding the idle-lock hold) and a panicking probe is
            // caught, never wedging or crashing the reaper. Either fault marks
            // the slot unhealthy → evict.
            //
            // SAFETY (unwind): the only state alive across the guarded await is
            // `entry` (owned, already popped off the queue); a caught panic
            // leaves it intact and it is moved to `failed` for destruction, so
            // no partial/torn state survives.
            match crate::hook_guard::guard_author_hook(
                crate::hook_guard::DEFAULT_AUTHOR_HOOK_CEILING,
                self.resource
                    .check(self.topology.slot_instance(&entry.slot)),
            )
            .await
            {
                // Healthy — keep the slot idle.
                Ok(Ok(())) => kept.push_back(entry),
                // The check ran and reported the instance unhealthy — evict.
                Ok(Err(_)) => failed.push(entry.slot),
                // The check hung past the ceiling or panicked — bounded/caught
                // by the framework; treat as unhealthy and evict.
                Err(fault) => {
                    fault.observe(&key, "probe");
                    failed.push(entry.slot);
                },
            }
        }
        *idle = kept;
        failed
    }
}

/// The release teardown future a guard's drop schedules: run the topology's
/// `on_release` reset, then either return the slot to the framework store
/// (under the revoke-epoch fence) or destroy it.
///
/// # Atomicity (revoke fence)
///
/// `on_release` (reset / recycle) runs **first**; the slot is handed to
/// [`crate::topology::store::InstanceStore::return_slot`] **last**, which re-reads the live revoke epoch
/// under the idle lock before pushing. So a revoke landing during a parking
/// `on_release` still evicts on return — the under-lock compare-then-push is the
/// fence, identical to the historical pool recycle `Keep` arm.
async fn release_slot<R>(
    managed: Arc<ManagedResource<R>>,
    mut slot: SlotOf<R>,
    checkout_epoch: u64,
    tainted: bool,
    metrics: Option<ResourceOpsMetrics>,
) -> Result<(), Error>
where
    R: Provider + HasCredentialSlots,
    R::Instance: Clone,
    R::Topology: Topology<R>,
{
    // Recycle-vs-discard observability (ADR-0093 Tier-4): exactly one
    // outcome is recorded per release — `Recycled` only on the clean
    // return-to-store arm, `Discarded` on every teardown path (tainted,
    // reset error, evict-on-return, non-pooling / `Drop` decision). The
    // `record` helper makes the `Option<metrics>` no-op explicit and keeps
    // the no-double-count discipline local to one call per arm.
    let record = |outcome: RecycleOutcome| {
        if let Some(m) = &metrics {
            m.record_recycle_outcome(outcome);
        }
    };

    // Tainted lease — destroy immediately, never recycle. Taint is set by the
    // credential-revoke fan-out, so this is the revoke teardown path.
    if tainted {
        record(RecycleOutcome::Discarded);
        return destroy_within(
            &managed.resource,
            managed.topology.into_instance(slot),
            TeardownReason::Revoked,
        )
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
            record(RecycleOutcome::Discarded);
            let destroy = destroy_within(
                &managed.resource,
                managed.topology.into_instance(slot),
                TeardownReason::Released,
            )
            .await;
            return destroy.and(Err(e));
        },
    };

    if keep && managed.topology.pools() {
        // FENCE: `return_slot` re-reads the revoke epoch under the idle lock.
        match managed.store.return_slot(slot, checkout_epoch).await {
            ReturnOutcome::Recycled => {
                record(RecycleOutcome::Recycled);
                Ok(())
            },
            ReturnOutcome::Evict(slot) => {
                record(RecycleOutcome::Discarded);
                destroy_within(
                    &managed.resource,
                    managed.topology.into_instance(slot),
                    TeardownReason::Evicted,
                )
                .await
            },
        }
    } else {
        // Non-pooling topology (Resident / permit-only) or a `Drop` decision:
        // the released slot is destroyed, never pooled.
        record(RecycleOutcome::Discarded);
        destroy_within(
            &managed.resource,
            managed.topology.into_instance(slot),
            TeardownReason::Released,
        )
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
pub(super) struct SlotCreateGuard<R>
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
    pub(super) fn new(
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
    pub(super) fn slot_mut(&mut self) -> &mut SlotOf<R> {
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
    pub(super) fn defuse(mut self) -> SlotOf<R> {
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
                let _ = destroy_within(
                    &managed.resource,
                    managed.topology.into_instance(slot),
                    TeardownReason::Evicted,
                )
                .await;
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };

    use arc_swap::ArcSwap;
    use nebula_core::{ExecutionId, ResourceKey, resource_key};
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        context::ResourceContext,
        error::Error,
        release_queue::ReleaseQueue,
        resource::{HasCredentialSlots, Provider, ResourceConfig, ResourceMetadata, TeardownCx},
        runtime::managed::ManagedResource,
        state::ResourceStatus,
        topology::{Pooled, pooled::config::Config as PoolConfig, store::InstanceStore},
    };

    // Minimal pooled resource config used by both test helpers.
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
    }

    impl Mock {
        fn new() -> Self {
            Self {
                created: Arc::new(AtomicU64::new(0)),
                destroyed: Arc::new(AtomicU64::new(0)),
            }
        }
    }

    #[async_trait::async_trait]
    impl Provider for Mock {
        type Config = PoolCfg;
        type Instance = u64;
        type Topology = Pooled<Self>;

        fn key() -> ResourceKey {
            resource_key!("acquire-loop-guard-mock")
        }

        async fn create(&self, _config: &PoolCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
            let id = self.created.fetch_add(1, Ordering::SeqCst);
            Ok(id)
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
                maintenance_sweeps: AtomicU64::new(0),
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

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;

        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            0,
            "a defused SlotCreateGuard must not schedule a destroy"
        );
    }
}
