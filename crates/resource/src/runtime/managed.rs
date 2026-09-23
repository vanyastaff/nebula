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
use tokio_util::sync::CancellationToken;

use crate::{
    recovery::RecoveryGate,
    release_queue::ReleaseQueue,
    resource::Provider,
    state::{ResourcePhase, ResourceStatus},
    topology::{
        AdmissionPhase, Load, MaintenanceSchedule, Topology, Unavailable, store::InstanceStore,
    },
    topology_tag::TopologyTag,
};

/// The `Entry` type of a resource's topology — the leasable unit the framework
/// stores and the guard holds for its whole lease.
pub(crate) type EntryOf<R> = <<R as Provider>::Topology as Topology<R>>::Entry;

/// The row owns its maintenance task until terminal cleanup has joined it.
#[derive(Default)]
pub(crate) struct Maintenance {
    cancellation: CancellationToken,
    task: std::sync::Mutex<Option<MaintenanceTask>>,
}

struct MaintenanceTask(tokio::task::JoinHandle<()>);

/// Typed marker carried by terminal errors when retained lease accounting
/// cannot prove final ownership.
///
/// Recovery is deliberately unavailable in-process: extracting a poisoned
/// owner could destroy an instance which is still leased. The owner remains
/// fenced until process restart.
#[derive(Debug)]
pub(crate) struct LeaseAccountingPoisoned;

impl std::fmt::Display for LeaseAccountingPoisoned {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("retained lease accounting poisoned; process restart required")
    }
}

impl std::error::Error for LeaseAccountingPoisoned {}

impl Drop for MaintenanceTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Maintenance {
    pub(crate) fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            task: std::sync::Mutex::new(None),
        }
    }

    pub(crate) fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub(crate) fn set_task(&self, task: tokio::task::JoinHandle<()>) {
        let mut owner = self
            .task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        debug_assert!(
            owner.is_none(),
            "registration installs maintenance only once"
        );
        *owner = Some(MaintenanceTask(task));
    }

    fn abort(&self) {
        // An unpolled/rejected retirement must break a sweep's Arc<row>
        // self-retention even when it cannot await graceful completion.
        let task = self
            .task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(task) = task {
            tracing::debug!("aborting maintenance after terminal cleanup abandonment");
            drop(task);
        }
    }

    async fn join(&self) -> Result<(), crate::Error> {
        let task = self
            .task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(mut task) = task {
            if let Ok(joined) =
                tokio::time::timeout(crate::hook_guard::MAX_TEARDOWN_CEILING, &mut task.0).await
            {
                joined.map_err(|_| {
                    crate::Error::permanent(
                        "resource maintenance worker failed during terminal cleanup",
                    )
                })?;
            } else {
                tracing::warn!(
                    "maintenance join timed out; aborting and awaiting worker acknowledgement"
                );
                task.0.abort();
                let _ = (&mut task.0).await;
                return Err(crate::Error::cancelled());
            }
        }
        Ok(())
    }
}

impl Drop for Maintenance {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

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
    /// recycles `PoolEntry<R>`s here; [`Resident`](crate::topology::Resident)
    /// (which does not pool) leaves it empty. A custom topology receives a
    /// borrowed `&store`, which it may clone; that capability is not tenant
    /// authorization. The framework, not the topology, runs
    /// `checkout` / `return_entry` / `evict_stale` against it.
    pub(crate) store: InstanceStore<EntryOf<R>>,
    /// Retained roots remain owned and loss-accounted independently of policy hooks.
    pub(crate) retained: crate::RetainedStore<EntryOf<R>>,
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
    /// Optional rate limit consumed on every acquire and exposed per call
    /// through the guard.
    pub(crate) rate_limiter: Option<Arc<crate::rate_limit::RateLimiter>>,
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
    /// Drives the cost-aware health-probe cadence: the reaper probes idle entries
    /// via [`Provider::check`] only on sweeps where
    /// `sweeps % R::check_cost().probe_every_n_sweeps() == 0`, so an
    /// [`Expensive`](crate::CheckCost::Expensive) check runs far less often than
    /// a [`Cheap`](crate::CheckCost::Cheap) one. Bumped once per
    /// [`run_maintenance`](Self::run_maintenance).
    pub(crate) maintenance_sweeps: AtomicU64,
    pub(crate) maintenance: Maintenance,
}

impl<R: Provider> std::fmt::Debug for ManagedResource<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `resource: R`, `topology: R::Topology`, and `store` hold live
        // `R::Instance`s with no `Debug` bound — print the process-visible
        // bookkeeping counters instead of the instance payload.
        f.debug_struct("ManagedResource")
            .field("key", &R::key())
            .field("generation", &self.generation.load(Ordering::Relaxed))
            .field("tainted", &self.tainted.load(Ordering::Relaxed))
            .field(
                "maintenance_sweeps",
                &self.maintenance_sweeps.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

impl<R: Provider> ManagedResource<R> {
    /// Publishes the terminal admission fence before any cleanup suspension.
    pub(crate) fn begin_close(&self) {
        self.store.begin_close();
        self.retained.begin_close();
        self.maintenance.cancellation.cancel();
    }

    /// Breaks maintenance ownership when a terminal job is abandoned.
    /// Graceful cleanup instead takes and joins the handle, making this a no-op.
    pub(crate) fn abort_maintenance(&self) {
        self.maintenance.abort();
    }

    /// Joins maintenance outside the release queue so author hooks may release dependencies.
    #[tracing::instrument(skip_all, fields(resource.key = %R::key()))]
    pub(crate) async fn join_maintenance(&self) -> Result<(), crate::Error> {
        self.maintenance.join().await
    }

    /// Destroys idle and retained owners after maintenance has been joined externally.
    ///
    /// Runs inside a queue-owned coordinator; each entry receives its own
    /// fault boundary and budget, not one timeout for the entire collection.
    #[tracing::instrument(skip_all, fields(resource.key = %R::key()))]
    pub(crate) async fn close_retained(self: &Arc<Self>) -> Result<(), crate::Error> {
        self.begin_close();
        let entries = self.store.close_and_drain().await;
        let mut batch = super::destroy_batch::DestroyBatch::new(
            Arc::clone(self),
            entries,
            crate::TeardownReason::Shutdown,
        );
        let quiesce =
            crate::hook_guard::guard_author_hook(crate::hook_guard::MAX_TEARDOWN_CEILING, async {
                self.topology.quiesce().await
            })
            .await;
        let quiesce = match quiesce {
            Ok(result) => result,
            Err(fault) => {
                fault.observe(&R::key(), "quiesce");
                Err(match fault {
                    crate::hook_guard::HookFault::Panicked => {
                        crate::Error::permanent("topology quiesce hook panicked")
                    },
                    crate::hook_guard::HookFault::TimedOut => crate::Error::cancelled(),
                })
            },
        };
        let wait_for_leases = tokio::time::timeout(
            crate::hook_guard::MAX_TEARDOWN_CEILING,
            self.retained.wait_terminal_quiescent(),
        )
        .await;
        let (ready, blocked_after_wait) = self.retained.drain_all().into_parts();
        batch.extend_retained(ready);
        let leases = match wait_for_leases {
            Ok(Ok(())) if blocked_after_wait.is_none() => Ok(()),
            Ok(Ok(())) => {
                tracing::error!(
                    reason = ?blocked_after_wait.map(super::retained_store::DrainBlocked::reason),
                    "closed retained store changed after quiescence; blocked roots remain armed"
                );
                Err(crate::Error::cancelled())
            },
            Ok(Err(blocked)) => {
                tracing::error!(
                    reason = ?blocked.reason(),
                    ready_owner_count = batch.len(),
                    "retained lease accounting is poisoned; healthy roots continue to teardown"
                );
                Err(crate::Error::permanent(
                    "retained lease accounting poisoned; process restart required",
                )
                .with_source(LeaseAccountingPoisoned))
            },
            Err(_) => {
                tracing::warn!(
                    reason = ?blocked_after_wait.map(super::retained_store::DrainBlocked::reason),
                    ready_owner_count = batch.len(),
                    "retained lease quiescence timed out; healthy roots continue to teardown"
                );
                Err(crate::Error::cancelled())
            },
        };
        let cleanup = batch.run().await;
        quiesce.and(leases).and(cleanup)
    }

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
    pub(crate) fn set_failed(&self, kind: crate::error::ErrorKind, message: impl Into<String>) {
        let next = ResourceStatus {
            phase: ResourcePhase::Failed,
            generation: self.generation(),
            last_error: Some(crate::state::ResourceErrorSummary {
                kind,
                message: message.into(),
            }),
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

    /// Current in-flight-acquire count for *this* resource row — a
    /// point-in-time read of the counter [`in_flight_tracker`](Self::in_flight_tracker)
    /// hands out, without exposing the tracker's tuple shape at call sites.
    pub(crate) fn in_flight_count(&self) -> usize {
        self.in_flight.0.load(Ordering::Acquire) as usize
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
    R: Provider,
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
    /// track none) so stale idle entries evict on the next sweep / acquire.
    pub(crate) fn set_fingerprint(&self, fingerprint: u64) {
        self.topology.set_fingerprint(fingerprint);
    }

    /// Admission phase snapshot from the topology.
    pub(crate) fn admission_phase(&self) -> AdmissionPhase {
        self.topology
            .phase(crate::topology::store::StoreView::new(&self.store))
    }

    /// Admission load snapshot from the topology.
    pub(crate) fn admission_load(&self) -> Option<Load> {
        self.topology
            .load(crate::topology::store::StoreView::new(&self.store))
    }

    /// Sync capacity gate from the topology — an **advisory** yes/no pre-check
    /// with a typed reason, NOT a held reservation.
    ///
    /// The [`Ticket`](crate::topology::contract::Ticket) (and any semaphore permit it carries) is dropped
    /// immediately, so the permit is released the moment this returns. This is a
    /// deliberate pre-flight probe (e.g. for `Manager::admission_status`): under
    /// contention the permit it momentarily held can be taken by another
    /// acquirer before the real acquire runs, so a gate `Ok` does not guarantee
    /// the subsequent acquire admits — the authoritative reservation is the
    /// `try_reserve` inside [`run_acquire_loop`](Self::run_acquire_loop), whose
    /// `Ticket` IS held for the lease. A gate `Err(Saturated)` likewise releases
    /// its permit; it reports the rejection, it does not hold it.
    pub(crate) fn try_reserve_gate(&self) -> Result<(), Unavailable> {
        self.topology
            .try_reserve(crate::topology::store::StoreView::new(&self.store))
            .map(|_ticket| ())
    }
}

#[cfg(test)]
#[path = "managed_tests.rs"]
mod tests;
