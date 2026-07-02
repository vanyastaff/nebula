//! Framework-owned acquire loop and cancel-safety guard for [`ManagedResource`].
//!
//! This module holds the `impl` block that requires the full
//! `R: Provider, R::Instance: Clone, R::Topology:
//! Topology<R>` bound — everything that reaches into the topology:
//!
//! - [`run_acquire_loop`](ManagedResource::run_acquire_loop) — the fenced
//!   acquire: reserve → checkout → destroy stale → accept-or-create → prepare
//!   → build guard.
//! - [`checkout_or_create`](ManagedResource::checkout_or_create) — inner loop factored out for future keyed variants.
//! - [`build_guard`](ManagedResource::build_guard) — assembles the [`ResourceGuard`] with its release closure.
//! - [`bump_revoke_epoch`](ManagedResource::bump_revoke_epoch) /
//!   [`dispatch_slot_hook`](ManagedResource::dispatch_slot_hook) — credential
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
    release_queue::ReleaseQueue,
    resource::{Provider, TeardownReason},
    runtime::{managed::ManagedResource, teardown::destroy_within},
    topology::{Topology, store::ReturnOutcome},
};

use super::managed::EntryOf;

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
    /// 3. destroy every `checkout.stale` entry via
    ///    `destroy(into_instance(stale))` — the **framework** tears down
    ///    since-revoked idle entries; a topology author can never skip this.
    /// 4. `accept(&mut entry)` a fresh idle entry, or `create_entry(…)` on a miss.
    /// 5. wrap the entry in an [`EntryCreateGuard`] (cancel-safety: a drop here
    ///    schedules an async `destroy` via the [`ReleaseQueue`]).
    /// 6. `prepare(&mut entry)` — per-acquire session init; `Err` ⇒ destroy +
    ///    fail.
    /// 7. build the guard whose `Deref` is `topology.entry_instance(&entry)` and
    ///    whose drop runs `on_release(&mut entry)` then either
    ///    `store.return_entry(entry, epoch)` (if `pools()` and kept) or
    ///    `destroy(into_instance(entry))`.
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
    /// `destroy(into_instance(entry))` via the [`ReleaseQueue`] — see
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
        //      `destroy(into_instance(entry))` via the ReleaseQueue instead of
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
            let _ = destroy_within(
                &self.resource,
                self.topology.into_instance(entry),
                TeardownReason::Released,
            )
            .await;
            return Err(e);
        }

        // 7. Build the guard. Defuse the cancel guard first so its Drop does
        //    not also schedule a destroy.
        let entry = cancel_guard.defuse();
        Ok(self.build_guard(entry, checkout_epoch, permit, generation, metrics))
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
    /// **before** any subsequent await (the stale-destroy loop, the `accept`
    /// hook, the fence destroy), so a caller cancellation — a `tokio::select!`
    /// branch or `tokio::time::timeout` dropping the acquire future — never
    /// discards a live instance through a plain `Drop`: the guard schedules an
    /// async `Provider::destroy` via the [`ReleaseQueue`]. A cancellation that
    /// lands *inside* one of the inline `destroy_within` awaits abandons only
    /// that teardown attempt (teardown is best-effort and deadline-bounded);
    /// the instance is already consumed by the destroy at that point, never
    /// leaked live.
    ///
    /// Complexity: O(stale + 1) idle pops per call (average and worst case);
    /// bounded by the store's idle capacity.
    async fn checkout_or_create(
        self: &Arc<Self>,
        ctx: &ResourceContext,
        config: &R::Config,
    ) -> Result<(EntryCreateGuard<R>, u64), Error> {
        loop {
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
            for stale in checkout.stale {
                let _ = destroy_within(
                    &self.resource,
                    self.topology.into_instance(stale),
                    TeardownReason::Revoked,
                )
                .await;
            }
            let Some((mut cancel_guard, epoch)) = fresh else {
                // Idle-miss — create a fresh entry. Snapshot the revoke epoch
                // BEFORE create so a revoke that lands *during* `create_entry` is
                // detectable (HikariCP #1836): stamping after the await would
                // read the post-revoke counter and silently admit a
                // since-revoked instance.
                let create_epoch = self.store.current_revoke_epoch();
                let entry = self
                    .topology
                    .create_entry(&self.resource, config, ctx)
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
                        self.topology.into_instance(entry),
                        TeardownReason::Revoked,
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
                return Ok((
                    EntryCreateGuard::new(entry, Arc::clone(self), Arc::clone(&self.release_queue)),
                    create_epoch,
                ));
            };
            if self
                .topology
                .accept(cancel_guard.entry_mut(), &self.resource, ctx)
                .await
            {
                return Ok((cancel_guard, epoch));
            }
            // Rejected (stale fingerprint / max-lifetime / broken) — destroy and
            // loop to the next idle entry, then create.
            let entry = cancel_guard.defuse();
            let _ = destroy_within(
                &self.resource,
                self.topology.into_instance(entry),
                TeardownReason::Evicted,
            )
            .await;
        }
    }

    /// Builds the leased [`ResourceGuard<R>`] over a chosen entry.
    ///
    /// The guard's `Deref` is a clone of `entry_instance(&entry)`; the release
    /// closure captures the **whole entry** (metadata intact) + an `Arc<Self>`
    /// (store + topology + resource) and, on guard drop, runs
    /// `on_release(&mut entry)` then either `store.return_entry(entry, epoch)`
    /// (pools + kept) or `destroy(into_instance(entry))`.
    fn build_guard(
        self: &Arc<Self>,
        entry: EntryOf<R>,
        checkout_epoch: u64,
        permit: Option<OwnedSemaphorePermit>,
        generation: u64,
        metrics: Option<ResourceOpsMetrics>,
    ) -> ResourceGuard<R> {
        // The guard's Deref value is a clone of the leasable instance; the
        // release closure owns the real entry.
        let deref_instance = self.topology.entry_instance(&entry).clone();
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
                Box::pin(release_entry(
                    managed,
                    entry,
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

    /// Borrows the live topology and invokes the per-entry credential hook —
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
        let created_epoch = self.store.stamp_epoch();
        let entry = self
            .topology
            .create_entry(&self.resource, config, ctx)
            .await?;
        // Cancel-safety: arm the guard before the idle-lock await below — a
        // cancellation landing there must destroy the just-created instance,
        // not drop it silently.
        let cancel_guard =
            EntryCreateGuard::new(entry, Arc::clone(self), Arc::clone(&self.release_queue));
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
                let _ = destroy_within(
                    &self.resource,
                    self.topology.into_instance(entry),
                    TeardownReason::Evicted,
                )
                .await;
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
                        error = %e,
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
                        error = %e,
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
    /// Each evicted/failed entry is destroyed via `destroy(into_instance(entry))`.
    /// Returns the number evicted.
    ///
    /// Complexity: O(n) over the idle queue (average and worst case), bounded
    /// by the store's configured idle capacity; the probe arm adds at most one
    /// `check` per idle entry on a due sweep.
    pub(crate) async fn run_maintenance(self: &Arc<Self>) -> usize {
        use std::sync::atomic::Ordering;

        let mut to_destroy = self.store.evict_stale().await;
        let nonrevoke = self
            .store
            .retain(|entry, _epoch| self.topology.idle_evictable(entry))
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
            to_destroy.extend(failed);
        }

        let evicted = to_destroy.len();
        for entry in to_destroy {
            let _ = destroy_within(
                &self.resource,
                self.topology.into_instance(entry),
                TeardownReason::Evicted,
            )
            .await;
        }
        if evicted > 0 {
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
    async fn probe_idle_entries(self: &Arc<Self>) -> Vec<EntryOf<R>> {
        let key = R::key();
        let mut failed = Vec::new();

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
                let entry = guard.defuse();
                match outcome {
                    // Healthy — return through the fence. `Evict` here means
                    // a revoke landed while this entry was mid-probe (or the
                    // store's capacity was reached by concurrent returns
                    // while the batch sat drained): destroy it, never
                    // re-admit.
                    Ok(Ok(())) => {
                        if let ReturnOutcome::Evict(entry) =
                            self.store.return_entry(entry, checkout_epoch).await
                        {
                            failed.push(entry);
                        }
                    },
                    // The check ran and reported the instance unhealthy — evict.
                    Ok(Err(_)) => failed.push(entry),
                    // The check hung past the ceiling or panicked —
                    // bounded/caught by the framework; treat as unhealthy
                    // and evict.
                    Err(fault) => {
                        fault.observe(&key, "probe");
                        failed.push(entry);
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
async fn release_entry<R>(
    managed: Arc<ManagedResource<R>>,
    mut entry: EntryOf<R>,
    checkout_epoch: u64,
    tainted: bool,
    metrics: Option<ResourceOpsMetrics>,
) -> Result<(), Error>
where
    R: Provider,
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
            managed.topology.into_instance(entry),
            TeardownReason::Revoked,
        )
        .await;
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
            let destroy = destroy_within(
                &managed.resource,
                managed.topology.into_instance(entry),
                TeardownReason::Released,
            )
            .await;
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
                destroy_within(
                    &managed.resource,
                    managed.topology.into_instance(entry),
                    TeardownReason::Evicted,
                )
                .await
            },
        }
    } else {
        // Non-pooling topology (Resident / permit-only) or a `Drop` decision:
        // the released entry is destroyed, never pooled.
        record(RecycleOutcome::Discarded);
        destroy_within(
            &managed.resource,
            managed.topology.into_instance(entry),
            TeardownReason::Released,
        )
        .await
    }
}

/// Cancel-safety guard for the framework acquire loop's create-then-prepare
/// window, generalized over the topology's [`Entry`](Topology::Entry).
///
/// Wraps a freshly checked-out / created entry from the moment it leaves the
/// store/`create_entry` until the [`ResourceGuard`] is built. If the acquire
/// future is cancelled in that window (`tokio::select!` / timeout), `Drop`
/// schedules an async `destroy(into_instance(entry))` on the [`ReleaseQueue`] —
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
    R::Instance: Clone,
    R::Topology: Topology<R>,
{
    /// `None` after [`defuse`](Self::defuse) took it out; `Some(_)` for any
    /// guard a caller can still observe. `Drop` short-circuits on `None`.
    entry: Option<EntryOf<R>>,
    /// The managed resource (store + topology + resource) so `Drop` can
    /// `destroy(into_instance(entry))` from the [`ReleaseQueue`].
    managed: Arc<ManagedResource<R>>,
    /// The framework release queue so `Drop` submits the async destroy with the
    /// queue's bounded backpressure + shutdown drain (not an orphan spawn).
    release_queue: Arc<ReleaseQueue>,
}

impl<R> EntryCreateGuard<R>
where
    R: Provider,
    R::Instance: Clone,
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
    R::Instance: Clone,
    R::Topology: Topology<R>,
{
    fn drop(&mut self) {
        let Some(entry) = self.entry.take() else {
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
                // An entry cancelled before reaching the built guard was never
                // admitted to the store or handed to a caller; the only correct
                // cleanup is destroy.
                let _ = destroy_within(
                    &managed.resource,
                    managed.topology.into_instance(entry),
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
        recovery::gate::{RecoveryGate, RecoveryGateConfig},
        release_queue::ReleaseQueue,
        resource::{Provider, ResourceConfig, ResourceMetadata, TeardownCx},
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
        /// When `true`, `check` parks forever — the deterministic suspension
        /// point for the accept-await cancellation tests.
        hang_check: Arc<AtomicBool>,
        /// Probe-lock regression fixture: when `true`, `check` notifies
        /// `check_started` the instant it begins, then parks on
        /// `release_check` until the test releases it — a *slow* (it
        /// eventually resolves) check, distinct from `hang_check` (never
        /// resolves). Lets a test observe "the probe is mid-check, still
        /// outside the idle lock" deterministically.
        park_in_check: Arc<AtomicBool>,
        check_started: Arc<Notify>,
        release_check: Arc<Notify>,
        /// Min-idle-refill fixture: when `true`, the *next* `create` call notifies
        /// `create_entered` the instant it begins, then parks on
        /// `release_create` until the test releases it — lets a test observe
        /// "a create is in flight, entry not yet deposited" deterministically
        /// (mirrors `park_in_check`, but for `create` rather than `check`).
        park_create: Arc<AtomicBool>,
        create_entered: Arc<Notify>,
        release_create: Arc<Notify>,
    }

    impl Mock {
        fn new() -> Self {
            Self {
                created: Arc::new(AtomicU64::new(0)),
                destroyed: Arc::new(AtomicU64::new(0)),
                hang_check: Arc::new(AtomicBool::new(false)),
                park_in_check: Arc::new(AtomicBool::new(false)),
                check_started: Arc::new(Notify::new()),
                release_check: Arc::new(Notify::new()),
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
            resource_key!("acquire-loop-guard-mock")
        }

        async fn create(&self, _config: &PoolCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
            let id = self.created.fetch_add(1, Ordering::SeqCst);
            if self.park_create.swap(false, Ordering::SeqCst) {
                self.create_entered.notify_one();
                self.release_create.notified().await;
            }
            Ok(id)
        }

        async fn check(&self, _runtime: &u64) -> Result<(), Error> {
            if self.hang_check.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
                // guard-justified: `std::future::pending()` never resolves,
                // so this line is statically unreachable.
                unreachable!("pending future never resolves")
            }
            if self.park_in_check.load(Ordering::SeqCst) {
                self.check_started.notify_one();
                self.release_check.notified().await;
            }
            Ok(())
        }

        async fn destroy(&self, _runtime: u64, _cx: TeardownCx) -> Result<(), Error> {
            self.destroyed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    crate::no_credential_slots!(Mock);

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

    /// Cancel-safety regression (audit 2026-07-01 bug #1): an acquire future
    /// cancelled while suspended in `Topology::accept` (here: a hanging
    /// `test_on_checkout` health check) must destroy the popped idle entry via
    /// the release queue — before the fix the entry was a plain local across
    /// the `accept().await` and a cancellation dropped the live instance
    /// without ever calling `Provider::destroy` (permanent leak: the entry was
    /// already off the idle queue).
    #[tokio::test]
    async fn cancelled_acquire_during_accept_destroys_the_popped_entry() {
        let resource = Mock::new();
        let destroyed = Arc::clone(&resource.destroyed);
        let hang_check = Arc::clone(&resource.hang_check);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let mr = {
            let topology = Pooled::<Mock>::new(
                PoolConfig {
                    test_on_checkout: true,
                    ..PoolConfig::default()
                },
                0,
            );
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

        // Seed one healthy idle entry, then arm the hang so the NEXT acquire
        // parks inside `accept`'s health check with the entry popped.
        let entry = mr
            .topology
            .create_entry(&mr.resource, &PoolCfg, &test_ctx())
            .await
            .expect("create the seed entry");
        let epoch = mr.store.stamp_epoch();
        assert!(
            !mr.store.deposit_fresh(entry, epoch).await.is_evict(),
            "the seed entry must land in the idle queue"
        );
        hang_check.store(true, Ordering::SeqCst);

        // The cancellation: a timeout drops the acquire future while it is
        // suspended in `accept` → `resource.check`.
        let cancelled = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            mr.run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None),
        )
        .await;
        assert!(
            cancelled.is_err(),
            "the acquire must still be parked in the hanging health check \
             when the timeout fires"
        );
        assert!(
            mr.store.is_empty().await,
            "the popped entry must not have been silently re-queued"
        );

        // Drain the release queue and assert the destroy actually ran.
        rq.close();
        drop(rq);
        drop(mr);
        ReleaseQueue::shutdown(rq_handle).await;
        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            1,
            "a cancellation during `accept` must destroy the popped entry via \
             the ReleaseQueue, never leak it through a plain Drop"
        );
    }

    /// Cancel-safety regression (audit 2026-07-01 bug #2): a warmup future
    /// dropped between `create_entry` succeeding and the fenced deposit
    /// completing (here: parked on the held idle lock; in production the
    /// author-hook ceiling timeout in `Manager::warmup_pool`) must destroy
    /// the created instance via the release queue — before the fix the entry
    /// travelled unguarded into `deposit_fresh`'s future and a cancellation
    /// dropped it without `Provider::destroy`.
    #[tokio::test]
    async fn cancelled_warmup_between_create_and_deposit_destroys_the_entry() {
        let resource = Mock::new();
        let created = Arc::clone(&resource.created);
        let destroyed = Arc::clone(&resource.destroyed);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let mr = {
            let topology = Pooled::<Mock>::new(
                PoolConfig {
                    min_size: 1, // warmup_target = 1
                    ..PoolConfig::default()
                },
                0,
            );
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

        // Hold the idle lock so warmup creates its entry, then parks on the
        // lock acquisition — the exact created-but-undeposited window.
        let idle_lock = mr.store.lock_idle().await;
        let ctx = test_ctx();
        {
            let mut warmup = Box::pin(mr.warmup(&ctx));
            let parked =
                tokio::time::timeout(std::time::Duration::from_millis(100), &mut warmup).await;
            assert!(
                parked.is_err(),
                "warmup must be parked awaiting the idle lock with a created entry in hand"
            );
            drop(warmup); // the cancellation
        }
        drop(idle_lock);

        assert_eq!(
            created.load(Ordering::SeqCst),
            1,
            "exactly one instance was created before the cancellation"
        );
        assert!(
            mr.store.is_empty().await,
            "the cancelled warmup must not have deposited the entry"
        );

        rq.close();
        drop(rq);
        drop(mr);
        ReleaseQueue::shutdown(rq_handle).await;
        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            1,
            "a warmup cancelled between create and deposit must destroy the \
             created instance via the ReleaseQueue, never leak it"
        );
    }

    /// Cancel-safety: an [`EntryCreateGuard`] dropped before `defuse` schedules an
    /// async `destroy` via the release queue.
    #[tokio::test]
    async fn entry_create_guard_drop_destroys_via_release_queue() {
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

        let entry = mr
            .topology
            .create_entry(&mr.resource, &PoolCfg, &test_ctx())
            .await
            .expect("create");
        let guard = EntryCreateGuard::new(entry, Arc::clone(&mr), Arc::clone(&rq));
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
            "EntryCreateGuard::drop must schedule destroy via the ReleaseQueue \
             when the acquire future is cancelled mid-create"
        );
    }

    /// A `EntryCreateGuard` that runs through `defuse` (the success path) must
    /// NOT trigger a stray destroy.
    #[tokio::test]
    async fn entry_create_guard_defuse_skips_destroy() {
        let resource = Mock::new();
        let destroyed = Arc::clone(&resource.destroyed);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let mr = managed(resource, PoolConfig::default());

        let entry = mr
            .topology
            .create_entry(&mr.resource, &PoolCfg, &test_ctx())
            .await
            .expect("create");
        let guard = EntryCreateGuard::new(entry, Arc::clone(&mr), Arc::clone(&rq));
        let _entry = guard.defuse();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;

        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            0,
            "a defused EntryCreateGuard must not schedule a destroy"
        );
    }

    // ── Probe-lock fix regression tests ─────────────────────────────────

    /// A slow (but eventually healthy) `Provider::check` must not block a
    /// concurrent checkout while a maintenance probe is running — the idle
    /// lock is held only to drain the queue, never across the check itself.
    #[tokio::test]
    async fn probe_slow_check_does_not_block_concurrent_checkout() {
        let resource = Mock::new();
        let mr = managed(resource.clone(), PoolConfig::default());

        // Seed one idle entry for the probe to find.
        let entry = mr
            .topology
            .create_entry(&mr.resource, &PoolCfg, &test_ctx())
            .await
            .expect("create the seed entry");
        let epoch = mr.store.stamp_epoch();
        assert!(
            !mr.store.deposit_fresh(entry, epoch).await.is_evict(),
            "the seed entry must land in the idle queue"
        );

        resource.park_in_check.store(true, Ordering::SeqCst);
        let check_started = Arc::clone(&resource.check_started);
        let release_check = Arc::clone(&resource.release_check);

        let mr_probe = Arc::clone(&mr);
        let probe_task = tokio::spawn(async move { mr_probe.probe_idle_entries().await });

        // Deterministic: the probe has drained the store and its (only)
        // `check()` call has started — i.e. it is now suspended OUTSIDE the
        // idle lock (the fix under test). Pre-fix, the lock was held across
        // this exact suspension point.
        check_started.notified().await;

        // Prove the idle lock is free: a concurrent checkout completes
        // promptly instead of blocking on the still-in-flight probe. It
        // correctly observes an empty queue (the probe drained the only
        // entry) — the point is that it does not HANG waiting for a lock.
        let checkout =
            tokio::time::timeout(std::time::Duration::from_millis(200), mr.store.checkout())
                .await
                .expect(
                    "checkout must not block on a slow probe check — the idle \
                 lock must not be held across it",
                );
        assert!(
            checkout.fresh.is_none(),
            "the single idle entry is off-store while the probe holds it mid-check"
        );

        // Release the check and let the probe finish.
        release_check.notify_one();
        let failed = probe_task.await.expect("probe task must not panic");
        assert!(
            failed.is_empty(),
            "the slow-but-healthy check must survive, not be marked failed"
        );
        assert_eq!(
            mr.store.len().await,
            1,
            "the survivor must be returned to the idle queue via the \
             epoch-fenced return path"
        );
    }

    /// A probe sweep must drain the idle store in batches of at most
    /// [`PROBE_CONCURRENCY`], never the whole idle queue in one shot — the
    /// bound on how far live instances can transiently overshoot the
    /// topology's cap while a sweep is in flight (a concurrent acquire can
    /// create a fresh instance for each entry currently drained-but-not-yet-
    /// returned).
    #[tokio::test]
    async fn probe_drains_in_bounded_batches_not_the_whole_idle_queue() {
        let resource = Mock::new();
        let mr = managed(resource.clone(), PoolConfig::default());

        // Seed more idle entries than a single probe batch holds, so the
        // batch boundary is observable.
        let seeded = PROBE_CONCURRENCY + 2;
        for _ in 0..seeded {
            let entry = mr
                .topology
                .create_entry(&mr.resource, &PoolCfg, &test_ctx())
                .await
                .expect("create seed entry");
            let epoch = mr.store.stamp_epoch();
            assert!(
                !mr.store.deposit_fresh(entry, epoch).await.is_evict(),
                "every seed entry must land in the idle queue"
            );
        }
        assert_eq!(mr.store.len().await, seeded);

        resource.park_in_check.store(true, Ordering::SeqCst);
        let check_started = Arc::clone(&resource.check_started);
        let release_check = Arc::clone(&resource.release_check);

        let mr_probe = Arc::clone(&mr);
        let probe_task = tokio::spawn(async move { mr_probe.probe_idle_entries().await });

        // Deterministic: the first batch's checks have started, which only
        // happens after that batch's drain (under the idle lock) already
        // completed.
        check_started.notified().await;

        // The bounded-batch drain must leave the rest of the idle queue
        // alone — never the whole-queue drain a single unbounded `mem::take`
        // would perform.
        assert_eq!(
            mr.store.len().await,
            seeded - PROBE_CONCURRENCY,
            "one probe batch must drain at most PROBE_CONCURRENCY entries, \
             leaving the rest of the idle queue available to concurrent \
             acquires instead of the whole queue at once"
        );

        // Release the first batch and let the remaining batch(es) proceed
        // without parking, so the sweep can finish.
        resource.park_in_check.store(false, Ordering::SeqCst);
        release_check.notify_waiters();

        let failed = probe_task.await.expect("probe task must not panic");
        assert!(
            failed.is_empty(),
            "every seeded entry is healthy and must survive the sweep"
        );
        assert_eq!(
            mr.store.len().await,
            seeded,
            "every entry must be returned to the idle queue across every batch"
        );
    }

    /// A credential revoke that lands WHILE an entry is mid-probe (drained,
    /// health check in flight) must destroy that entry on return — never
    /// re-admit it to the idle queue. This is the fence-preservation half of
    /// the probe-lock fix: a plain `*idle = survivors` write-back would
    /// resurrect a since-revoked entry; routing survivors back through
    /// `InstanceStore::return_entry` re-checks the epoch under the re-taken
    /// lock and evicts instead.
    #[tokio::test]
    async fn probe_revoke_mid_probe_destroys_probed_entries_not_redeposited() {
        let resource = Mock::new();
        let destroyed = Arc::clone(&resource.destroyed);
        let mr = managed(resource.clone(), PoolConfig::default());

        let entry = mr
            .topology
            .create_entry(&mr.resource, &PoolCfg, &test_ctx())
            .await
            .expect("create the seed entry");
        let epoch = mr.store.stamp_epoch();
        assert!(
            !mr.store.deposit_fresh(entry, epoch).await.is_evict(),
            "the seed entry must land in the idle queue"
        );

        resource.park_in_check.store(true, Ordering::SeqCst);
        let check_started = Arc::clone(&resource.check_started);
        let release_check = Arc::clone(&resource.release_check);

        let mr_probe = Arc::clone(&mr);
        let probe_task = tokio::spawn(async move { mr_probe.probe_idle_entries().await });

        check_started.notified().await;

        // The revoke fence bump — exactly what `Manager::revoke_slot`'s
        // synchronous phase 1 does — lands while the entry is drained and
        // mid-check, strictly BEFORE the check resolves.
        mr.store.bump_revoke_epoch();

        // Let the (otherwise healthy) check resolve.
        release_check.notify_one();
        let failed = probe_task.await.expect("probe task must not panic");

        assert_eq!(
            failed.len(),
            1,
            "an entry revoked mid-probe must be reported for the caller to \
             destroy, not silently dropped or kept"
        );
        assert_eq!(
            mr.store.len().await,
            0,
            "an entry revoked mid-probe must NEVER be written back to the \
             idle queue (a plain `*idle = survivors` write-back would \
             resurrect a revoked entry)"
        );

        // Run the destroy the caller (`run_maintenance`, in production)
        // performs on every `failed` entry, and confirm it actually ran —
        // proving this is a real destroy path, not just an accounting
        // artifact.
        for entry in failed {
            let _ = destroy_within(
                &mr.resource,
                mr.topology.into_instance(entry),
                TeardownReason::Revoked,
            )
            .await;
        }
        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            1,
            "the revoked-mid-probe entry must actually be torn down via \
             Provider::destroy"
        );
    }

    // ── Reaper-tick min-idle floor refill ───────────────────────────────

    /// After a maintenance sweep evicts every idle entry, `refill_min_idle`
    /// tops the store back up to `min_size` (warmup_target) — the reaper
    /// closes the gap proactively instead of waiting for the next
    /// caller-driven acquire to create one on demand.
    #[tokio::test]
    async fn refill_min_idle_tops_up_after_maintenance_eviction() {
        let resource = Mock::new();
        let created = Arc::clone(&resource.created);
        let mr = managed(
            resource,
            PoolConfig {
                min_size: 2,
                max_size: 4,
                idle_timeout: None,
                max_lifetime: None,
                ..PoolConfig::default()
            },
        );

        // Two overlapping leases so both entries land in the idle queue on
        // release — a serial acquire-release would just reuse the one entry
        // and only ever accumulate one.
        let g1 = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await
            .expect("acquire 1");
        let g2 = mr
            .run_acquire_loop(&test_ctx(), &AcquireOptions::default(), None)
            .await
            .expect("acquire 2");
        g1.release().await.expect("release 1");
        g2.release().await.expect("release 2");
        assert_eq!(mr.store.len().await, 2);

        // Force eviction deterministically via a fingerprint bump (no
        // wall-clock sleep needed).
        mr.set_fingerprint(99);
        let evicted = mr.run_maintenance().await;
        assert_eq!(evicted, 2, "both stale-fingerprint entries must be evicted");
        assert_eq!(mr.store.len().await, 0);

        let refilled = mr.refill_min_idle(&test_ctx()).await;
        assert_eq!(
            refilled, 2,
            "refill must top the idle queue back up to min_size"
        );
        assert_eq!(mr.store.len().await, 2);
        assert_eq!(
            created.load(Ordering::SeqCst),
            4,
            "2 initial creates + 2 refill creates"
        );
    }

    /// MAJOR regression (final-review item 2): the deficit computation used
    /// to read only the idle-queue floor (`warmup_target - idle_len`),
    /// ignoring currently checked-out leases tracked by
    /// `ManagedResource::in_flight`. Under full load — idle empty, every
    /// permit already leased — the naive deficit equalled `min_size` and the
    /// tick created that many *extra* instances on top of the `max_size`
    /// already checked out, overshooting the pool to `max_size + min_size`
    /// live instances. The fix additionally bounds the refill by
    /// `store.capacity() - (idle_len + in_flight)`; at `max_size` fully
    /// leased that headroom is zero, so the tick must create nothing.
    #[tokio::test]
    async fn refill_min_idle_does_not_overshoot_when_pool_is_fully_leased() {
        let resource = Mock::new();
        let created = Arc::clone(&resource.created);
        let (rq, _handle) = ReleaseQueue::new(1);
        let mr = {
            let config = PoolConfig {
                min_size: 2,
                max_size: 2,
                idle_timeout: None,
                max_lifetime: None,
                ..PoolConfig::default()
            };
            let topology = Pooled::<Mock>::new(config.clone(), 0);
            Arc::new(ManagedResource {
                resource,
                config: ArcSwap::from_pointee(PoolCfg),
                topology,
                store: InstanceStore::new(Some(config.max_size as usize)),
                release_queue: Arc::new(rq),
                generation: AtomicU64::new(0),
                status: ArcSwap::from_pointee(ResourceStatus::new()),
                recovery_gate: None,
                tainted: AtomicBool::new(false),
                in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
                maintenance_sweeps: AtomicU64::new(0),
            })
        };

        // Simulate both `max_size` leases already checked out and in flight
        // — the state `Manager::acquire_pooled`'s `InFlightCounter` puts
        // `ManagedResource::in_flight` in for the duration of a lease.
        // `run_acquire_loop` alone (used directly by this module's unit
        // tests) never touches that counter, so it is set directly here to
        // isolate `refill_min_idle`'s bound math from the manager dispatch
        // layer.
        mr.in_flight.0.store(2, Ordering::SeqCst);
        assert_eq!(
            mr.store.len().await,
            0,
            "idle queue starts empty — both entries are checked out, not idle"
        );

        let refilled = mr.refill_min_idle(&test_ctx()).await;
        assert_eq!(
            refilled, 0,
            "no headroom left under max_size while both leases are in flight \
             — the reaper tick must be a no-op"
        );
        assert_eq!(
            created.load(Ordering::SeqCst),
            0,
            "refill must not create instances beyond max_size"
        );
    }

    /// A `RecoveryGate` in any state other than `Idle` (a recovery attempt
    /// in progress here) must make `refill_min_idle` a complete no-op —
    /// creating replacement entries against a backend the gate has already
    /// flagged unhealthy would recreate the exact stampede the gate exists
    /// to prevent.
    #[tokio::test]
    async fn refill_min_idle_skips_when_gate_not_idle() {
        let resource = Mock::new();
        let created = Arc::clone(&resource.created);
        let gate = RecoveryGate::new(RecoveryGateConfig::default());
        // Holding the ticket moves the gate to `InProgress`.
        let _ticket = gate.try_begin().expect("gate starts idle");

        let mr = {
            let (rq, _handle) = ReleaseQueue::new(1);
            let topology = Pooled::<Mock>::new(
                PoolConfig {
                    min_size: 2,
                    ..PoolConfig::default()
                },
                0,
            );
            Arc::new(ManagedResource {
                resource,
                config: ArcSwap::from_pointee(PoolCfg),
                topology,
                store: InstanceStore::new(None),
                release_queue: Arc::new(rq),
                generation: AtomicU64::new(0),
                status: ArcSwap::from_pointee(ResourceStatus::new()),
                recovery_gate: Some(Arc::new(gate)),
                tainted: AtomicBool::new(false),
                in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
                maintenance_sweeps: AtomicU64::new(0),
            })
        };

        let refilled = mr.refill_min_idle(&test_ctx()).await;
        assert_eq!(
            refilled, 0,
            "a non-Idle gate must skip the refill tick entirely"
        );
        assert_eq!(
            created.load(Ordering::SeqCst),
            0,
            "no create_entry call must happen while the gate is not Idle"
        );
    }

    /// Cancel-safety invariant: shutdown-during-refill race is clean. A refill task
    /// aborted while `create` is in flight (before the entry is deposited —
    /// here: parked in the mock's `create`; in production the reaper task
    /// being cancelled by `graceful_shutdown`) must destroy the created
    /// instance via the release queue, never leak it and never panic —
    /// mirrors `cancelled_warmup_between_create_and_deposit_destroys_the_entry`,
    /// proving `refill_min_idle` inherited the same cancel-safety contract
    /// through the shared `create_and_deposit_entries` helper.
    #[tokio::test]
    async fn refill_min_idle_shutdown_race_destroys_in_flight_entry() {
        let resource = Mock::new();
        resource.park_create.store(true, Ordering::SeqCst);
        let created = Arc::clone(&resource.created);
        let destroyed = Arc::clone(&resource.destroyed);
        let create_entered = Arc::clone(&resource.create_entered);
        let release_create = Arc::clone(&resource.release_create);
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let mr = {
            let topology = Pooled::<Mock>::new(
                PoolConfig {
                    min_size: 1, // refill deficit == 1 against an empty store
                    ..PoolConfig::default()
                },
                0,
            );
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

        let mr_refill = Arc::clone(&mr);
        let ctx = test_ctx();
        let refill_task = tokio::spawn(async move { mr_refill.refill_min_idle(&ctx).await });

        // `len()` (the deficit check) strictly precedes `create_entry` in
        // program order, so by the time `create` has entered and parked, the
        // idle lock is free — take it ourselves *before* letting `create`
        // resume, so the loop's post-create `lock_idle().await` blocks on
        // us. This produces the exact created-but-undeposited window a
        // cancelled reaper task (shutdown) lands in, without racing `len()`
        // for the same lock (holding it from the start would block `len()`
        // itself, never reaching `create_entry` at all).
        create_entered.notified().await;
        let idle_lock = mr.store.lock_idle().await;
        release_create.notify_one();
        // Let the task resume past `create`, build its `EntryCreateGuard`,
        // and block on the lock we hold. Single-threaded test runtime: this
        // only needs to yield long enough for the scheduler to poll the
        // parked task once.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        refill_task.abort(); // the cancellation: shutdown drops the reaper task
        let _ = refill_task.await; // best-effort join (JoinError::is_cancelled())
        drop(idle_lock);

        assert_eq!(
            created.load(Ordering::SeqCst),
            1,
            "exactly one instance was created before the cancellation"
        );
        assert!(
            mr.store.is_empty().await,
            "the cancelled refill must not have deposited the entry"
        );

        rq.close();
        drop(rq);
        drop(mr);
        ReleaseQueue::shutdown(rq_handle).await;
        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            1,
            "a refill cancelled between create and deposit must destroy the \
             created instance via the ReleaseQueue, never leak it — no panic, \
             no leak"
        );
    }

    /// Fence invariant: revoke-during-refill. A `revoke_slot` epoch bump that
    /// lands while a refill-created entry is still mid-`create` (the window
    /// between the epoch snapshot and the fenced deposit) must make the
    /// deposit fence destroy the entry, never admit it to the idle queue as
    /// a since-revoked instance.
    #[tokio::test]
    async fn refill_min_idle_revoke_mid_create_destroys_not_deposits() {
        let resource = Mock::new();
        resource.park_create.store(true, Ordering::SeqCst);
        let created = Arc::clone(&resource.created);
        let destroyed = Arc::clone(&resource.destroyed);
        let create_entered = Arc::clone(&resource.create_entered);
        let release_create = Arc::clone(&resource.release_create);
        let mr = managed(
            resource,
            PoolConfig {
                min_size: 1,
                ..PoolConfig::default()
            },
        );

        let mr_refill = Arc::clone(&mr);
        let ctx = test_ctx();
        let refill_task = tokio::spawn(async move { mr_refill.refill_min_idle(&ctx).await });

        // Deterministic: the entry's pre-revoke epoch is already snapshotted
        // (`create_and_deposit_entries` stamps it before calling
        // `create_entry`) and `create` is now parked mid-flight.
        create_entered.notified().await;

        // The revoke fence bump — exactly what `Manager::revoke_slot`'s
        // synchronous phase 1 does — lands while the entry is still being
        // created, strictly before the fenced deposit.
        mr.bump_revoke_epoch();
        release_create.notify_one();

        // Let the refill actually finish: it must observe the epoch
        // mismatch at deposit and destroy the entry instead of admitting it.
        let refilled = refill_task.await.expect("refill task must not panic");
        assert_eq!(
            refilled, 0,
            "the epoch-fenced entry must not count as a successful refill"
        );
        assert!(
            mr.store.is_empty().await,
            "the deposit fence must never admit a since-revoked entry — no \
             plain write-back that would resurrect it"
        );
        assert_eq!(created.load(Ordering::SeqCst), 1);
        assert_eq!(
            destroyed.load(Ordering::SeqCst),
            1,
            "the fenced entry must be destroyed, not silently dropped"
        );
    }
}
