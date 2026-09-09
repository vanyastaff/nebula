//! Framework-owned acquire loop and cancel-safety guard for [`ManagedResource`].
//!
//! This module holds the `impl` block that requires the full
//! `R: Provider, R::Topology:
//! Topology<R>` bound — everything that reaches into the topology:
//!
//! - [`run_acquire_loop`](ManagedResource::run_acquire_loop) — the fenced
//!   acquire: reserve → checkout → destroy stale → accept-or-create → prepare
//!   → build guard.
//! - [`checkout_or_create`](ManagedResource::checkout_or_create) — inner loop factored out for future keyed variants.
//! - [`build_guard`](ManagedResource::build_guard) — assembles the [`ResourceGuard`] with its release closure.
//! - [`bump_revoke_epoch`](ManagedResource::bump_revoke_epoch) /
//!   [`submit_slot_hook`](ManagedResource::submit_slot_hook) — credential
//!   rotation hooks that need topology dispatch.
//! - [`warmup`](ManagedResource::warmup) / [`run_maintenance`](ManagedResource::run_maintenance) /
//!   [`probe_idle_entries`](ManagedResource::probe_idle_entries) — lifecycle maintenance driven by the registry reaper.
//! - [`release_entry`] — the async release teardown future the guard's drop schedules.
//! - [`EntryCreateGuard`] — cancel-safety RAII guard for the create-then-prepare window.
//!
//! The weak `impl<R: Provider>` block (status / phase / taint / drain) stays in
//! [`managed`](super::managed) so code that never touches the topology can import
//! a lighter set of bounds.

use std::sync::Arc;

use futures::stream::{self, StreamExt};
use tokio::sync::OwnedSemaphorePermit;

use crate::{
    context::ResourceContext,
    error::Error,
    guard::ResourceGuard,
    metrics::{RecycleOutcome, ResourceOpsMetrics},
    options::AcquireOptions,
    release_queue::{ReleaseQueue, ReleaseSubmission, SubmissionOutcome},
    resource::{Provider, TeardownReason},
    runtime::{managed::ManagedResource, teardown::destroy_within},
    topology::{Topology, store::ReturnOutcome},
};

use super::managed::EntryOf;

mod credential_hook;

pub(crate) use credential_hook::{
    AcceptedSlotHook, RetiredCleanupObservation, SlotHookAdmission, SlotHookDeferral,
    SlotHookObservation, SlotHookSettlement, SlotHookWaitOutcome,
};
#[cfg(test)]
use credential_hook::{RetiredCleanupSettlement, SlotHookReceipt};

pub(crate) type RetiredCleanupObserver = Box<dyn FnOnce(RetiredCleanupObservation) + Send>;

/// Publishes displaced retained owners after every author-held lease is dropped.
///
/// The guard is created before each topology future that receives the retained
/// store. Rust drops that future (and therefore its [`crate::RetainedLease`]s)
/// before this earlier local. Its `Drop` is the guaranteed edge-triggered retry:
/// a last lease release cannot leave the framework-owned retained backlog waiting for an
/// unrelated acquire or maintenance sweep.
struct RetiredEntriesGuard<R: Provider>(Arc<ManagedResource<R>>);

impl<R: Provider> Drop for RetiredEntriesGuard<R> {
    fn drop(&mut self) {
        match self.0.queue_retired_entries() {
            Some(Ok(submission)) => submission.detach(),
            Some(Err(error)) => tracing::warn!(
                error.kind = ?error.kind(),
                resource.key = %R::key(),
                "retired-entry cleanup submission was rejected"
            ),
            None => {},
        }
    }
}

// ── The framework acquire loop + topology-driven lifecycle.
//
// Everything that reaches into the topology — the acquire loop, the revoke
// fence, rotation dispatch, warmup, maintenance — lives here behind the
// `R::Topology: Topology<R>` bound. The weak `R: Provider` block in managed.rs
// stays usable by code that never touches the topology (status / phase / taint /
// drain).

impl<R> ManagedResource<R>
where
    R: Provider,
    R::Topology: Topology<R>,
{
    async fn observe_cleanup_submission(submission: Result<ReleaseSubmission, Error>) {
        match submission {
            Ok(submission) => match submission.wait().await {
                Ok(SubmissionOutcome::Completed) => {},
                Ok(SubmissionOutcome::Deferred) => tracing::debug!(
                    resource.key = %R::key(),
                    "background resource cleanup remains queue-owned"
                ),
                Err(error) => tracing::warn!(
                    error.kind = ?error.kind(),
                    resource.key = %R::key(),
                    "background resource cleanup did not complete successfully"
                ),
            },
            Err(error) => tracing::warn!(
                error.kind = ?error.kind(),
                resource.key = %R::key(),
                "background resource cleanup submission was rejected"
            ),
        }
    }

    fn detach_cleanup_submission(submission: Result<ReleaseSubmission, Error>) {
        match submission {
            Ok(submission) => submission.detach(),
            Err(error) => tracing::warn!(
                error.kind = ?error.kind(),
                resource.key = %R::key(),
                "background resource cleanup submission was rejected"
            ),
        }
    }

    /// **The framework acquire loop.** Runs the full fenced acquire over the
    /// framework-owned [`store`](ManagedResource::store) and the topology's R-aware hooks,
    /// producing a typed [`ResourceGuard<R>`].
    ///
    /// The loop the framework owns (not the topology):
    /// 1. `topology.try_reserve(&store)` — the sync concurrency gate; the
    ///    returned permit is held by the guard for the whole lease.
    /// 2. `store.checkout()` — the **framework** revoke-epoch fence on pop.
    /// 3. destroy every `checkout.stale` entry via
    ///    `destroy(into_owned_instance(stale))` — the **framework** tears down
    ///    since-revoked idle entries; a topology author can never skip this.
    /// 4. `accept(&mut entry)` a fresh idle entry, or `create_entry(…)` on a miss.
    /// 5. wrap the entry in an [`EntryCreateGuard`] (cancel-safety: a drop here
    ///    schedules an async `destroy` via the [`ReleaseQueue`]).
    /// 6. `prepare(&mut entry)` — per-acquire session init; `Err` ⇒ destroy +
    ///    fail.
    /// 7. build the guard whose `Deref` is `topology.entry_instance(&entry)` and
    ///    whose drop runs `on_release(&mut entry)` then either
    ///    `store.return_entry(entry, epoch)` (if `pools()` and kept) or
    ///    `destroy(into_owned_instance(entry))`.
    ///
    /// # Atomicity (revoke fence)
    ///
    /// The fence is the store's: `checkout` pops under the idle lock and
    /// collects stale entries; `return_entry` re-reads the live revoke epoch under
    /// the idle lock before pushing. The on-release closure runs `on_release`
    /// (recycle/reset) *first* and hands the entry to `return_entry` *last*, so a
    /// revoke landing during a parking `on_release` still evicts on return. A
    /// fresh-create that straddles a revoke is fenced by `return_entry`'s
    /// under-lock epoch re-read.
    ///
    /// # Errors
    ///
    /// - [`Unavailable`](crate::topology::Unavailable) from `try_reserve` is mapped to the caller error.
    /// - Propagates `create_entry` / `prepare` failures.
    ///
    /// # Cancel safety
    ///
    /// A drop between checkout/create and the built guard schedules an async
    /// `destroy(into_owned_instance(entry))` via the [`ReleaseQueue`] — see
    /// [`EntryCreateGuard`].
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

        // 2-5. Fenced checkout → destroy stale → accept-or-create. The whole
        //      idle-then-create decision is the framework's; the topology only
        //      validates (`accept`) or makes (`create_entry`). The entry comes
        //      back already armed in its cancel guard — `checkout_or_create`
        //      wraps it the moment it leaves the store / creation call, so a
        //      drop at ANY await from the pop onward (stale destroys, the
        //      `accept` hook, `prepare` below) schedules an async
        //      `destroy(into_owned_instance(entry))` via the ReleaseQueue instead of
        //      leaking the instance through a plain `Drop`.
        let (mut cancel_guard, checkout_epoch) = self.checkout_or_create(ctx, &config).await?;

        // 6. Per-acquire session init. `prepare` borrows the entry mutably from
        //    the cancel guard (a distinct object from `self`), so the topology
        //    `&self` hook and the `&mut entry` borrow do not alias.
        if let Err(e) = self
            .topology
            .prepare(cancel_guard.entry_mut(), &self.resource, ctx)
            .await
        {
            let entry = cancel_guard.defuse();
            Self::observe_cleanup_submission(self.queue_destroy(entry, TeardownReason::Released))
                .await;
            return Err(e);
        }

        if self.store.is_closed() {
            Self::observe_cleanup_submission(
                self.queue_destroy(cancel_guard.defuse(), TeardownReason::Shutdown),
            )
            .await;
            return Err(Error::cancelled().with_resource_key(R::key()));
        }

        // 7. Snapshot author metadata while the entry remains armed, then
        // synchronously transfer ownership into the completed lease guard.
        Ok(self.build_guard(cancel_guard, checkout_epoch, permit, generation, metrics))
    }

    /// Framework checkout-then-create: pop the first fresh idle entry (destroying
    /// every since-revoked stale entry the fence discards), validate it via
    /// `accept`, and on an idle-miss / accept-reject create a fresh entry.
    ///
    /// This is the inner half of the acquire loop; factored out so a future
    /// `checkout_keyed` (affinity) variant entries in beside it without reshaping
    /// the loop. Returns the chosen entry — already armed in its
    /// [`EntryCreateGuard`] — and its checkout epoch (the create path stamps
    /// the current epoch).
    ///
    /// # Cancel safety
    ///
    /// A popped or freshly-created entry is wrapped in an [`EntryCreateGuard`]
    /// **before** any subsequent await, so a caller cancellation — a
    /// `tokio::select!` branch or `tokio::time::timeout` dropping the acquire
    /// future — never discards a live instance through a plain `Drop`.
    /// Rejected and stale idle entries transfer to the [`ReleaseQueue`]
    /// synchronously; the acquire loop never waits for their physical teardown
    /// while it owns the topology permit.
    ///
    /// Complexity: O(stale + 1) idle pops per call (average and worst case);
    /// bounded by the store's idle capacity.
    async fn checkout_or_create(
        self: &Arc<Self>,
        ctx: &ResourceContext,
        config: &R::Config,
    ) -> Result<(EntryCreateGuard<R>, u64), Error> {
        loop {
            if self.store.is_closed() {
                return Err(Error::cancelled().with_resource_key(R::key()));
            }
            let checkout = self.store.checkout().await;
            // Cancel-safety: arm the fresh entry's guard NOW, before the stale
            // destroys and the `accept` hook below get a chance to park this
            // future — a drop while suspended there must schedule an async
            // destroy, not leak the popped instance.
            let fresh = checkout.fresh.map(|co| {
                let (entry, epoch) = co.into_parts();
                (
                    EntryCreateGuard::new(entry, Arc::clone(self), Arc::clone(&self.release_queue)),
                    epoch,
                )
            });
            // FRAMEWORK destroys since-revoked stale entries — the author can
            // never skip this fence.
            if let Some(submission) =
                self.queue_destroy_batch(checkout.stale, TeardownReason::Revoked)
            {
                Self::detach_cleanup_submission(submission);
            }
            let Some((mut cancel_guard, epoch)) = fresh else {
                // Idle-miss — create a fresh entry. Snapshot the revoke epoch
                // BEFORE create so a revoke that lands *during* `create_entry` is
                // detectable (HikariCP #1836): stamping after the await would
                // read the post-revoke counter and silently admit a
                // since-revoked instance.
                let create_epoch = self.store.current_revoke_epoch();
                let _retirement = RetiredEntriesGuard(Arc::clone(self));
                let created = self
                    .topology
                    .create_entry(&self.resource, config, ctx, &self.retained)
                    .await?;
                let (cancel_guard, retirement) = self.arm_created(created);
                if let Some(retirement) = retirement {
                    Self::observe_cleanup_submission(retirement).await;
                }
                if self.store.is_closed() {
                    Self::observe_cleanup_submission(
                        self.queue_destroy(cancel_guard.defuse(), TeardownReason::Shutdown),
                    )
                    .await;
                    return Err(Error::cancelled().with_resource_key(R::key()));
                }
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
                    Self::observe_cleanup_submission(
                        self.queue_destroy(cancel_guard.defuse(), TeardownReason::Revoked),
                    )
                    .await;
                    return Err(Error::revoked(format!(
                        "{}: credential revoked while the instance was being \
                         created — fenced before admission (HikariCP #1836)",
                        R::key()
                    )));
                }
                // No await between `create_entry` returning and this wrap, so
                // the created instance is guarded before the caller's next
                // suspension point.
                return Ok((cancel_guard, create_epoch));
            };
            if self
                .topology
                .accept(cancel_guard.entry_mut(), &self.resource, ctx)
                .await
            {
                return Ok((cancel_guard, epoch));
            }
            // Rejected (stale fingerprint / max-lifetime / broken) — destroy and
            // loop to the next idle entry, then create. Queue admission, not
            // physical teardown, is the ownership-transfer boundary.
            let entry = cancel_guard.defuse();
            Self::detach_cleanup_submission(self.queue_destroy(entry, TeardownReason::Evicted));
        }
    }

    /// Transfers one entry before an acquire's cancellation-sensitive await.
    fn queue_destroy(
        self: &Arc<Self>,
        entry: EntryOf<R>,
        reason: TeardownReason,
    ) -> Result<ReleaseSubmission, Error> {
        let managed = Arc::clone(self);
        self.release_queue.submit_release(move || {
            Box::pin(async move { managed.destroy_entry(entry, reason).await })
        })
    }

    fn queue_destroy_batch(
        self: &Arc<Self>,
        entries: Vec<EntryOf<R>>,
        reason: TeardownReason,
    ) -> Option<Result<ReleaseSubmission, Error>> {
        if entries.is_empty() {
            return None;
        }
        let batch = super::destroy_batch::DestroyBatch::new(Arc::clone(self), entries, reason);
        Some(
            self.release_queue
                .submit_coordinator(move || Box::pin(batch.run())),
        )
    }

    /// Extracts only the final lifecycle owner; shared leases do not destroy a master.
    pub(crate) async fn destroy_entry(
        &self,
        entry: EntryOf<R>,
        reason: TeardownReason,
    ) -> Result<(), Error> {
        match self.topology.into_owned_instance(entry) {
            Some(instance) => destroy_within(&self.resource, instance, reason).await,
            None => Ok(()),
        }
    }

    /// Arms every ownership transfer synchronously before the caller may await.
    fn arm_created(
        self: &Arc<Self>,
        created: crate::topology::CreatedEntry<EntryOf<R>>,
    ) -> (
        EntryCreateGuard<R>,
        Option<Result<ReleaseSubmission, Error>>,
    ) {
        let entry = created.into_entry();
        let active =
            EntryCreateGuard::new(entry, Arc::clone(self), Arc::clone(&self.release_queue));
        let receipt = self.queue_retired_entries();
        (active, receipt)
    }

    fn queue_retired_entries(self: &Arc<Self>) -> Option<Result<ReleaseSubmission, Error>> {
        let (entries, blocked) = self.retained.drain_retired().into_parts();
        if let Some(blocked) = blocked {
            tracing::debug!(
                resource.key = %R::key(),
                reason = ?blocked.reason(),
                "retained cleanup left fenced generations in the backlog"
            );
        }
        if entries.is_empty() {
            return None;
        }
        let mut batch = super::destroy_batch::DestroyBatch::new(
            Arc::clone(self),
            Vec::new(),
            TeardownReason::Evicted,
        );
        batch.extend_retained(entries);
        Some(
            self.release_queue
                .submit_coordinator(move || Box::pin(batch.run())),
        )
    }

    /// Builds a guard owning the exact entry checked out by this acquire.
    fn build_guard(
        self: &Arc<Self>,
        entry: EntryCreateGuard<R>,
        checkout_epoch: u64,
        permit: Option<OwnedSemaphorePermit>,
        generation: u64,
        metrics: Option<ResourceOpsMetrics>,
    ) -> ResourceGuard<R> {
        let identity = crate::guard::GuardIdentity {
            resource_key: R::key(),
            topology_tag: self.topology.tag(),
        };
        ResourceGuard::new(
            Arc::clone(self),
            entry.defuse(),
            checkout_epoch,
            permit,
            generation,
            metrics,
            identity,
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

    /// Creates one fresh entry and cancel-safely deposits it into the
    /// framework store — the shared create→guard→deposit-fence step
    /// [`create_and_deposit_entries`](Self::create_and_deposit_entries) (a
    /// fixed-count batch) and [`refill_min_idle`](Self::refill_min_idle) (a
    /// headroom-rechecking loop) both drive per attempt, so the cancel-safety
    /// and revoke-fence contract is written and tested once.
    ///
    /// The revoke epoch is snapshotted *before* `create_entry` even runs
    /// (`created_epoch`, outside any lock), then compared against the
    /// **live** epoch under the idle lock at deposit time via
    /// `InstanceStore::deposit_fresh_locked` — so a revoke that lands
    /// mid-create (after the snapshot, before the deposit) is detected as a
    /// stale `created_epoch` and the entry is destroyed instead of admitted.
    ///
    /// Returns `Ok(true)` if the entry was admitted to the idle store,
    /// `Ok(false)` if the deposit-time epoch fence evicted it instead (a
    /// revoke raced this create — a legitimate, retry-worthy outcome, not a
    /// failure), or `Err(_)` if `Provider::create` itself failed (the
    /// caller should stop attempting further creates this pass rather than
    /// hammer a backend that just started failing).
    ///
    /// # Cancel safety
    ///
    /// The created-but-not-yet-deposited entry is armed in an
    /// [`EntryCreateGuard`] before the idle-lock await, so a drop of this
    /// future — including the author-hook ceiling timeout
    /// `Manager::warmup_pool` wraps `warmup` in, or the reaper task being
    /// cancelled mid-refill on shutdown — schedules an async
    /// `Provider::destroy` via the [`ReleaseQueue`] instead of leaking the
    /// instance. The guard is defused only once the lock is held and the
    /// fenced deposit runs synchronously to completion.
    async fn create_and_deposit_one(
        self: &Arc<Self>,
        ctx: &ResourceContext,
        config: &R::Config,
    ) -> Result<bool, Error> {
        if self.store.is_closed() {
            return Ok(false);
        }
        let created_epoch = self.store.stamp_epoch();
        let _retirement = RetiredEntriesGuard(Arc::clone(self));
        let created = self
            .topology
            .create_entry(&self.resource, config, ctx, &self.retained)
            .await?;
        // Cancel-safety: arm the guard before the idle-lock await below — a
        // cancellation landing there must destroy the just-created instance,
        // not drop it silently.
        let (cancel_guard, retirement) = self.arm_created(created);
        // Maintenance must never await a job on its own cleanup queue:
        // terminal row cleanup may be waiting for this sweep to finish.
        if let Some(retirement) = retirement {
            Self::detach_cleanup_submission(retirement);
        }
        let mut idle = self.store.lock_idle().await;
        let entry = cancel_guard.defuse();
        let outcome = self
            .store
            .deposit_fresh_locked(&mut idle, entry, created_epoch);
        // Release the idle lock before any teardown await — the evict
        // destroy must not block checkout/return.
        drop(idle);
        match outcome {
            ReturnOutcome::Recycled => Ok(true),
            ReturnOutcome::Evict(entry) => {
                Self::detach_cleanup_submission(self.queue_destroy(entry, TeardownReason::Evicted));
                Ok(false)
            },
        }
    }

    /// Creates and cancel-safely deposits up to `requested` fresh entries
    /// into the framework store via [`create_and_deposit_one`](Self::create_and_deposit_one)
    /// — the fixed-count batch [`warmup`](Self::warmup) drives at
    /// registration. `requested` is a hard attempt cap, not a "keep retrying
    /// until this many succeed" target: a deposit-time eviction (revoke race)
    /// consumes one attempt without incrementing the return count, and
    /// `create_entry` failing stops the whole call early (best effort — a
    /// partially-filled store is better than hammering a backend that just
    /// started failing). Returns the number of entries actually deposited.
    async fn create_and_deposit_entries(
        self: &Arc<Self>,
        ctx: &ResourceContext,
        requested: usize,
    ) -> usize {
        let config = self.config();
        let mut created = 0usize;
        for _ in 0..requested {
            match self.create_and_deposit_one(ctx, &config).await {
                Ok(true) => created += 1,
                Ok(false) => {}, // deposit-time eviction — this attempt is spent
                Err(e) => {
                    tracing::warn!(
                        key = %R::key(),
                        error.kind = ?e.kind(),
                        created,
                        requested,
                        "create_and_deposit_entries: create_entry failed, stopping early"
                    );
                    break;
                },
            }
        }
        created
    }

    /// Pre-warms the store by creating + depositing `warmup_target` entries
    /// (fenced) at registration. Returns the number admitted.
    ///
    /// # Cancel safety
    ///
    /// See [`create_and_deposit_entries`](Self::create_and_deposit_entries).
    pub(crate) async fn warmup(self: &Arc<Self>, ctx: &ResourceContext) -> usize {
        let config = self.config();
        let target = self.topology.warmup_target(&config);
        if target == 0 {
            return 0;
        }
        let created = self.create_and_deposit_entries(ctx, target).await;
        if created > 0 {
            tracing::info!(key = %R::key(), created, target, "resource warmup complete");
        }
        created
    }

    /// Reaper-tick min-idle floor refill (HikariCP `minimumIdle`
    /// topping-off). After [`run_maintenance`](Self::run_maintenance) evicts
    /// TTL/idle-expired/stale-fingerprint entries, the idle queue can sit
    /// below `warmup_target` until the next caller-driven acquire creates
    /// one on demand. This closes that gap proactively from the maintenance
    /// side, reusing [`create_and_deposit_one`](Self::create_and_deposit_one)
    /// — the exact cancel-safe create→deposit step [`warmup`](Self::warmup)
    /// drives in a fixed-count batch — one attempt at a time instead.
    ///
    /// **Bounded by live-instance headroom, not just the idle floor,
    /// rechecked before every attempt.** The naive `warmup_target -
    /// idle_len` deficit ignores currently checked-out leases; under
    /// sustained full load (idle empty, every permit leased) that would
    /// create `min_size` extra instances on top of the `max_size` already
    /// checked out. The refill instead loops **at most `deficit` times**
    /// (`deficit` sampled once at tick start — a fixed attempt cap, never a
    /// retry-until-filled loop that could spin against a revoke-racing
    /// backend), and **before every one of those attempts** re-reads
    /// `idle_len` + `in_flight` and re-derives headroom
    /// (`store.capacity() - (idle_len + in_flight)`) fresh, stopping the
    /// moment headroom hits zero. This shrinks the overshoot window from
    /// "the whole batch's worth of concurrent-acquire races" (checked once,
    /// then blindly creating `bounded_deficit` entries) down to "one
    /// create's worth" (rechecked immediately before each one) — still not a
    /// hard, race-free guarantee (a concurrent acquire can still land in the
    /// gap between *this* attempt's headroom read and its own deposit), but
    /// bounded to a single in-flight create rather than the full batch.
    ///
    /// **Gated on the recovery gate.** When a [`RecoveryGate`](crate::recovery::gate::RecoveryGate)
    /// is attached and its state is anything other than
    /// [`GateState::Idle`](crate::recovery::gate::GateState::Idle) (a
    /// recovery attempt is in progress, backed off, or permanently failed),
    /// this is a no-op for the tick: creating replacement entries against a
    /// backend the gate itself has already flagged unhealthy would recreate
    /// the exact thundering-herd the gate exists to prevent, just from the
    /// maintenance side instead of the acquire side. A resource with no gate
    /// attached always refills. This check is a plain non-blocking read
    /// ([`RecoveryGate::state`](crate::recovery::gate::RecoveryGate::state)
    /// loads an `ArcSwap`) — no ticket is taken, so a healthy refill never
    /// contends with an in-flight acquire's own gate admission.
    ///
    /// # Cancel safety
    ///
    /// Identical to [`warmup`](Self::warmup) — see
    /// [`create_and_deposit_one`](Self::create_and_deposit_one). The reaper
    /// task being cancelled mid-refill (e.g. `graceful_shutdown`) destroys
    /// any in-flight entry via the [`ReleaseQueue`] instead of leaking it;
    /// entries already deposited stay in the store.
    pub(crate) async fn refill_min_idle(self: &Arc<Self>, ctx: &ResourceContext) -> usize {
        if let Some(gate) = &self.recovery_gate
            && !matches!(gate.state(), crate::recovery::gate::GateState::Idle)
        {
            return 0;
        }
        let config = self.config();
        let target = self.topology.warmup_target(&config);
        if target == 0 {
            return 0;
        }
        let idle_before = self.store.len().await;
        let deficit = target.saturating_sub(idle_before);
        if deficit == 0 {
            return 0;
        }

        let mut created = 0usize;
        for _ in 0..deficit {
            // Recompute headroom fresh before every attempt — see the doc
            // above for why this is bounded to `deficit` attempts total
            // rather than looping until `target` is actually reached.
            let idle_now = self.store.len().await;
            let in_flight = self.in_flight_count();
            let headroom = match self.store.capacity() {
                Some(cap) => cap.saturating_sub(idle_now + in_flight),
                // Unbounded topology: the outer `deficit`-attempt cap is the
                // only limit that applies.
                None => usize::MAX,
            };
            if headroom == 0 {
                break;
            }
            match self.create_and_deposit_one(ctx, &config).await {
                Ok(true) => created += 1,
                Ok(false) => {}, // deposit-time eviction (revoke race) — this attempt is spent
                Err(e) => {
                    tracing::warn!(
                        key = %R::key(),
                        error.kind = ?e.kind(),
                        created,
                        target,
                        "refill_min_idle: create_entry failed, stopping early"
                    );
                    break;
                },
            }
        }
        if created > 0 {
            tracing::debug!(
                key = %R::key(),
                created,
                deficit,
                idle_before,
                target,
                "resource maintenance: refilled min-idle floor"
            );
        }
        created
    }

    /// Runs one background maintenance sweep over the framework store.
    ///
    /// Three arms, all under the idle lock (atomic against checkout/return):
    /// - the **revoke** arm — [`crate::topology::store::InstanceStore::evict_stale`] evicts entries whose
    ///   checkout epoch is behind the live counter (framework-owned fence);
    /// - the **non-revoke** arm — [`crate::topology::store::InstanceStore::retain`] over the topology's
    ///   [`idle_evictable`](Topology::idle_evictable) predicate
    ///   (fingerprint / max-lifetime / idle-timeout);
    /// - the **health-probe** arm — [`probe_idle_entries`](Self::probe_idle_entries)
    ///   runs [`Provider::check`] over idle entries, but **only on sweeps where
    ///   the resource's [`CheckCost`](crate::CheckCost) cadence is due**, so an
    ///   expensive check is not run every sweep.
    ///
    /// Transfers evicted entries to one owned teardown batch without awaiting
    /// its receipt. The manager joins maintenance before publishing terminal
    /// queue work. Returns the number evicted, not completed teardowns.
    ///
    /// Complexity: O(n) over the idle queue (average and worst case), bounded
    /// by the store's configured idle capacity; the probe arm adds at most one
    /// `check` per idle entry on a due sweep.
    pub(crate) async fn run_maintenance(self: &Arc<Self>) -> usize {
        use std::sync::atomic::Ordering;

        let stale = self.store.evict_stale().await;
        let mut to_destroy = super::destroy_batch::DestroyBatch::new(
            Arc::clone(self),
            stale,
            TeardownReason::Evicted,
        );
        let nonrevoke = self
            .store
            .retain(|entry, _epoch| {
                if let Ok(evict) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.topology.idle_evictable(entry)
                })) {
                    evict
                } else {
                    crate::hook_guard::HookFault::Panicked.observe(&R::key(), "idle_evictable");
                    true
                }
            })
            .await;
        to_destroy.extend(nonrevoke);

        // Cost-aware health probe: only run `check` over idle entries on sweeps
        // where the resource's check cost says it is due (Cheap every sweep,
        // Expensive every 16th), so a network-round-trip check is not run on
        // every sweep over a pool of idle connections.
        let sweep = self.maintenance_sweeps.fetch_add(1, Ordering::Relaxed) + 1;
        let cadence = self.resource.check_cost().probe_every_n_sweeps();
        let mut probe_evicted = 0;
        if cadence != 0 && sweep.is_multiple_of(cadence) {
            let failed = self.probe_idle_entries().await;
            probe_evicted = failed.len();
            to_destroy.append(failed);
        }

        let evicted = to_destroy.len();
        if evicted > 0 {
            Self::detach_cleanup_submission(
                self.release_queue
                    .submit_coordinator(move || Box::pin(to_destroy.run())),
            );
            tracing::debug!(
                evicted,
                probe_evicted,
                "resource maintenance: evicted idle/expired/unhealthy entries"
            );
        }
        evicted
    }

    /// Health-probes every idle entry via [`Provider::check`], removing and
    /// returning the entries that fail so the caller destroys them.
    ///
    /// # Fence-preserving, non-blocking probe
    ///
    /// The idle lock is taken repeatedly, but only ever briefly, and never
    /// across a `check` await:
    ///
    /// 1. **Drain a batch** — pop at most [`PROBE_CONCURRENCY`] idle entries
    ///    under the lock, then release it. Bounding the drain to one batch
    ///    (rather than the whole idle queue in one shot) bounds the transient
    ///    "outside the idle store" overshoot to [`PROBE_CONCURRENCY`]
    ///    entries: a concurrent acquire during this window may create a
    ///    fresh instance instead of reusing one of the drained ones, so the
    ///    live-instance count can transiently exceed the topology's cap by up
    ///    to [`PROBE_CONCURRENCY`] — never by the whole idle queue (which, at
    ///    a large `max_size`, would otherwise let one sweep drive the pool to
    ///    roughly 2x its configured cap against the backend). Holding the
    ///    lock across every check instead would remove the overshoot but
    ///    reintroduce the head-of-line-blocking bug this probe design avoids:
    ///    a single slow/expensive `check` blocking every concurrent
    ///    checkout/return for the sweep's duration.
    /// 2. **Check the batch outside the lock**, then **return** each entry
    ///    whose check passed through
    ///    [`InstanceStore::return_entry`](crate::topology::store::InstanceStore::return_entry),
    ///    the framework's existing epoch-fenced return path: it re-reads the
    ///    live revoke epoch under the *re-taken* lock and evicts (never
    ///    re-queues) an entry whose checkout epoch has fallen behind — i.e. an
    ///    entry revoked *while the probe was running*. A plain
    ///    `*idle = survivors` write-back is **forbidden**: it would bypass the
    ///    fence and resurrect a since-revoked entry into the idle queue.
    /// 3. **Repeat** for the next batch, until this sweep's target count (the
    ///    idle-queue length sampled once at sweep start — see below) has been
    ///    drained or the queue empties early.
    ///
    /// Checks within a batch run **outside** the lock, with bounded
    /// concurrency ([`PROBE_CONCURRENCY`]) — checkout/return proceed freely
    /// against the (temporarily probe-owned) batch while its author `check`
    /// calls, each individually bound + panic-isolated through
    /// [`hook_guard::guard_author_hook`](crate::hook_guard::guard_author_hook),
    /// are in flight. The cost-aware cadence in
    /// [`run_maintenance`](Self::run_maintenance) is what bounds how often this
    /// runs, so an expensive `check` does not block the pool every sweep.
    ///
    /// The sweep probes exactly the entries present when it started (sampled
    /// once via `store.len()`), not however many keep cycling through the
    /// idle queue while it runs — an entry returned mid-sweep waits for the
    /// next maintenance tick. This bounds the number of batches to
    /// `ceil(initial_len / PROBE_CONCURRENCY)` regardless of concurrent
    /// churn, instead of the loop chasing a moving target.
    ///
    /// Complexity: O(n) checks over the sampled idle-queue length (average
    /// and worst case), bounded by the store's configured idle capacity; at
    /// most [`PROBE_CONCURRENCY`] run concurrently within a batch, and at
    /// most [`PROBE_CONCURRENCY`] entries sit outside the idle store at once
    /// across the whole sweep.
    ///
    /// # Cancel safety
    ///
    /// Every drained entry is armed in an [`EntryCreateGuard`] the instant it
    /// leaves the idle lock — the same guard [`create_and_deposit_entries`](Self::create_and_deposit_entries)
    /// uses for a freshly created entry — and stays armed for the whole
    /// batch of concurrent `check` awaits, defusing only once its outcome is
    /// classified. A reaper task aborted mid-probe (`graceful_shutdown`
    /// racing the background maintenance task) therefore destroys every
    /// still-in-flight entry via the [`ReleaseQueue`] instead of dropping it
    /// silently — this closes the batch-wide exposure the plain-local shape
    /// had before the drain became fenced per batch.
    async fn probe_idle_entries(self: &Arc<Self>) -> super::destroy_batch::DestroyBatch<R> {
        let key = R::key();
        let mut failed = super::destroy_batch::DestroyBatch::new(
            Arc::clone(self),
            Vec::new(),
            TeardownReason::Evicted,
        );

        // Sample the sweep's target count once — see the "Fence-preserving"
        // doc above for why this bounds the loop to a fixed number of
        // batches instead of chasing entries returned mid-sweep.
        let mut remaining = self.store.len().await;

        while remaining > 0 {
            let batch_size = remaining.min(PROBE_CONCURRENCY);

            // 1. Drain at most `batch_size` entries under a brief lock — see
            //    the "Fence-preserving" doc above. Arm each entry in an
            //    `EntryCreateGuard` immediately (see "Cancel safety" above)
            //    — never a plain local across the check awaits below.
            let batch: Vec<(EntryCreateGuard<R>, u64)> = {
                let mut idle = self.store.lock_idle().await;
                std::iter::from_fn(|| idle.pop_front())
                    .take(batch_size)
                    .map(|stored| {
                        let guard = EntryCreateGuard::new(
                            stored.entry,
                            Arc::clone(self),
                            Arc::clone(&self.release_queue),
                        );
                        (guard, stored.checkout_epoch)
                    })
                    .collect()
            };

            let drained = batch.len();
            if drained == 0 {
                // The queue emptied early (concurrent checkouts raced ahead
                // of this sweep) — nothing left to probe this tick.
                break;
            }
            remaining -= drained;

            // 2. Run every check in this batch OUTSIDE the lock, bounded
            //    concurrency.
            let checked = stream::iter(batch)
                .map(|(mut guard, checkout_epoch)| async move {
                    // Route the author's `check` through the bound+isolate
                    // chokepoint like every other author hook: a probe that
                    // hangs is cut at the ceiling and a panicking probe is
                    // caught, never wedging or crashing the reaper.
                    //
                    // SAFETY (unwind): the only state alive across the
                    // guarded await is `guard` (owned, already popped off
                    // the queue, not shared with any other task); a caught
                    // panic leaves it intact and this closure returns it to
                    // the caller for classification, so no partial/torn
                    // state survives.
                    let outcome = crate::hook_guard::guard_author_hook(
                        crate::hook_guard::DEFAULT_AUTHOR_HOOK_CEILING,
                        self.resource
                            .check(self.topology.entry_instance(guard.entry_mut())),
                    )
                    .await;
                    (guard, checkout_epoch, outcome)
                })
                .buffer_unordered(PROBE_CONCURRENCY)
                .collect::<Vec<_>>()
                .await;

            // 3. Classify: a survivor goes back through the epoch-fenced
            //    return path (never a direct write-back); everything else is
            //    collected for the caller to destroy. `defuse` disarms the
            //    cancel-safety guard now that the entry is about to be
            //    handed to one of those two framework-owned paths instead of
            //    sitting in a bare local.
            for (guard, checkout_epoch, outcome) in checked {
                match outcome {
                    // Healthy — return through the fence. `Evict` here means
                    // a revoke landed while this entry was mid-probe (or the
                    // store's capacity was reached by concurrent returns
                    // while the batch sat drained): destroy it, never
                    // re-admit.
                    Ok(Ok(())) => {
                        let mut idle = self.store.lock_idle().await;
                        let entry = guard.defuse();
                        if let ReturnOutcome::Evict(entry) =
                            self.store
                                .return_entry_locked(&mut idle, entry, checkout_epoch)
                        {
                            failed.push(entry);
                        }
                    },
                    // The check ran and reported the instance unhealthy — evict.
                    Ok(Err(_)) => failed.push(guard.defuse()),
                    // The check hung past the ceiling or panicked —
                    // bounded/caught by the framework; treat as unhealthy
                    // and evict.
                    Err(fault) => {
                        fault.observe(&key, "probe");
                        failed.push(guard.defuse());
                    },
                }
            }
        }
        failed
    }
}

/// Upper bound on concurrently in-flight [`Provider::check`] calls during a
/// single [`ManagedResource::probe_idle_entries`] sweep, and also the size of
/// each batch [`probe_idle_entries`](ManagedResource::probe_idle_entries)
/// drains from the idle store at a time.
///
/// A fixed, modest cap rather than "all idle entries at once": the idle
/// queue size tracks the topology's capacity (e.g. `PoolConfig::max_size`),
/// which can be large, and an unbounded fan-out would let one maintenance
/// sweep open that many concurrent `check` calls against the backend (a
/// connection-storming health-check burst) *and* pull that many entries out
/// of the idle store at once, letting concurrent acquires create up to that
/// many extra instances against the topology's cap. Probing is a background,
/// off-hot-path sweep, so trading a little probe latency for both a bounded
/// backend load and a bounded live-instance overshoot is the right default.
const PROBE_CONCURRENCY: usize = 8;

/// The release teardown future a guard's drop schedules: run the topology's
/// `on_release` reset, then either return the entry to the framework store
/// (under the revoke-epoch fence) or destroy it.
///
/// # Atomicity (revoke fence)
///
/// `on_release` (reset / recycle) runs **first**; the entry is handed to
/// [`crate::topology::store::InstanceStore::return_entry`] **last**, which re-reads the live revoke epoch
/// under the idle lock before pushing. So a revoke landing during a parking
/// `on_release` still evicts on return — the under-lock compare-then-push is the
/// fence, identical to the historical pool recycle `Keep` arm.
pub(crate) async fn release_entry<R>(
    managed: Arc<ManagedResource<R>>,
    mut entry: EntryOf<R>,
    checkout_epoch: u64,
    tainted: bool,
    metrics: Option<ResourceOpsMetrics>,
) -> Result<(), Error>
where
    R: Provider,
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

    if managed.store.is_closed() {
        record(RecycleOutcome::Discarded);
        return managed.destroy_entry(entry, TeardownReason::Shutdown).await;
    }

    // Tainted lease — destroy immediately, never recycle. Taint is set by the
    // credential-revoke fan-out, so this is the revoke teardown path.
    if tainted {
        record(RecycleOutcome::Discarded);
        return managed.destroy_entry(entry, TeardownReason::Revoked).await;
    }

    // Topology reset / recycle decision (runs before the store fence).
    let keep = match managed
        .topology
        .on_release(&mut entry, &managed.resource)
        .await
    {
        Ok(keep) => keep,
        Err(e) => {
            // Reset failed — destroy. Surface the reset error (so an awaited
            // `release()` sees the failed teardown) once the entry is torn down.
            record(RecycleOutcome::Discarded);
            let destroy = managed.destroy_entry(entry, TeardownReason::Released).await;
            return destroy.and(Err(e));
        },
    };

    if keep && managed.topology.pools() {
        // FENCE: `return_entry` re-reads the revoke epoch under the idle lock.
        match managed.store.return_entry(entry, checkout_epoch).await {
            ReturnOutcome::Recycled => {
                record(RecycleOutcome::Recycled);
                Ok(())
            },
            ReturnOutcome::Evict(entry) => {
                record(RecycleOutcome::Discarded);
                managed.destroy_entry(entry, TeardownReason::Evicted).await
            },
        }
    } else {
        // Non-pooling topology (Resident / permit-only) or a `Drop` decision:
        // the released entry is destroyed, never pooled.
        record(RecycleOutcome::Discarded);
        managed.destroy_entry(entry, TeardownReason::Released).await
    }
}

/// Cancel-safety guard for the framework acquire loop's create-then-prepare
/// window, generalized over the topology's [`Entry`](Topology::Entry).
///
/// Wraps a freshly checked-out / created entry from the moment it leaves the
/// store/`create_entry` until the [`ResourceGuard`] is built. If the acquire
/// future is cancelled in that window (`tokio::select!` / timeout), `Drop`
/// schedules an async `destroy(into_owned_instance(entry))` on the [`ReleaseQueue`] —
/// without this, only the instance's *sync* `Drop` runs and the server-side
/// resource (DB session, OS handle) leaks. The `cancel-drop` regression test
/// guards this.
///
/// Call [`defuse`](Self::defuse) once the guard is safely built; it consumes
/// the guard by value, so the borrow checker forbids any use after `defuse` and
/// the `Drop` never runs against a defused entry.
pub(super) struct EntryCreateGuard<R>
where
    R: Provider,
    R::Topology: Topology<R>,
{
    /// `None` after [`defuse`](Self::defuse) took it out; `Some(_)` for any
    /// guard a caller can still observe. `Drop` short-circuits on `None`.
    entry: Option<EntryOf<R>>,
    /// The managed resource (store + topology + resource) so `Drop` can
    /// `destroy(into_owned_instance(entry))` from the [`ReleaseQueue`].
    managed: Arc<ManagedResource<R>>,
    /// The framework release queue so `Drop` submits the async destroy with the
    /// queue's bounded backpressure + shutdown drain (not an orphan spawn).
    release_queue: Arc<ReleaseQueue>,
}

impl<R> EntryCreateGuard<R>
where
    R: Provider,
    R::Topology: Topology<R>,
{
    /// Creates a new guard wrapping the chosen entry.
    pub(super) fn new(
        entry: EntryOf<R>,
        managed: Arc<ManagedResource<R>>,
        release_queue: Arc<ReleaseQueue>,
    ) -> Self {
        Self {
            entry: Some(entry),
            managed,
            release_queue,
        }
    }

    /// Returns a mutable reference to the wrapped entry for `prepare`.
    ///
    /// `&mut self` keeps this a plain safe borrow: the acquire loop owns the
    /// cancel guard by value and only borrows it mutably here, so the topology
    /// `&self` hook (a distinct object) and this `&mut entry` never alias.
    pub(super) fn entry_mut(&mut self) -> &mut EntryOf<R> {
        // guard-justified: `entry` is `Some(_)` for the guard's whole observable
        // lifetime — it is set in `new` and only taken in `defuse`/`Drop`, both
        // of which consume the guard by value. Reaching `None` here would mean
        // a borrow after `defuse`, which the borrow checker already forbids, so
        // this `unreachable!` documents an unrepresentable state rather than a
        // runtime path.
        self.entry
            .as_mut()
            .unwrap_or_else(|| unreachable!("EntryCreateGuard::entry_mut after defuse"))
    }

    /// Consumes the guard and returns the wrapped entry.
    ///
    /// After this call the guard is gone; its `Drop` runs against `entry: None`
    /// and short-circuits without scheduling a destroy.
    pub(super) fn defuse(mut self) -> EntryOf<R> {
        // guard-justified: `defuse` consumes `self` by value, so the borrow
        // checker forbids calling it twice. `entry` is `Some(_)` for the guard's
        // whole observable lifetime (set in `new`, only taken here or in
        // `Drop`, both consuming), so `take()` cannot be `None` on this path.
        self.entry
            .take()
            .unwrap_or_else(|| unreachable!("EntryCreateGuard defused twice"))
    }
}

impl<R> Drop for EntryCreateGuard<R>
where
    R: Provider,
    R::Topology: Topology<R>,
{
    fn drop(&mut self) {
        let Some(entry) = self.entry.take() else {
            return; // defused — nothing to clean up
        };
        let managed = Arc::clone(&self.managed);
        tracing::warn!(
            resource_type = std::any::type_name::<R>(),
            "cancel-safety: acquire future cancelled mid-create — \
             scheduling async destroy via ReleaseQueue"
        );
        let submission = self.release_queue.submit_release(move || {
            Box::pin(async move {
                // An entry cancelled before reaching the built guard was never
                // admitted to the store or handed to a caller; the only correct
                // cleanup is destroy.
                managed.destroy_entry(entry, TeardownReason::Evicted).await
            })
        });
        match submission {
            Ok(submission) => submission.detach(),
            Err(error) => tracing::warn!(
                error.kind = ?error.kind(),
                resource.key = %R::key(),
                "cancel-safety destroy submission was rejected after ownership settlement"
            ),
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
