//! Central resource manager — registration, acquire dispatch, and shutdown.
//!
//! [`Manager`] is the single entry point for the resource subsystem. It owns
//! the registry and a [`CancellationToken`] for coordinated shutdown.
//!
//! Slot model: the public API carries no `R::Credential` projection. Resources
//! declare credential dependencies as typed slot fields on the struct (via
//! `#[credential]` attributes), and the framework resolves them BEFORE
//! `Resource::create` is invoked. The `acquire_*` family is therefore
//! credential-agnostic at the manager level.
//!
//! # Lifecycle
//!
//! ```text
//! Manager::new()
//!   ├── register()   — store ManagedResource in registry
//!   ├── acquire_*()  — scope-aware lookup + topology dispatch
//!   ├── remove()     — unregister + cleanup
//!   └── shutdown()   — cancel all, drain
//! ```
//!
//! # Submodule layout
//!
//! - `options` — `ManagerConfig`, `RegisterOptions`, `ShutdownConfig`, `DrainTimeoutPolicy`
//! - `gate` — `GateAdmission` + `admit_through_gate` + `settle_gate_admission`
//! - `execute` — resilience pipeline + register-time pool config validation
//! - `shutdown` — `graceful_shutdown` + drain helpers + `set_phase_all*`
//!
//! # The two-phase revoke / drain invariant (canonical)
//!
//! This is the authoritative description of how a credential revoke is made
//! safe against in-flight and future acquires. Every other site that touches
//! the taint flag, the per-resource in-flight counter, the revoke epoch, or
//! the cancellation-safe revoke tail carries only a one-line pointer back
//! here; the rationale lives **only** in this section so the invariant has a
//! single source of truth.
//!
//! ## Goal
//!
//! After a credential is revoked, the resource emits **no further
//! authenticated traffic on that credential**: no new lease is handed out on
//! it, no in-flight lease silently outlives the revoke without being
//! accounted for, and no pooled instance authenticated with it can re-enter
//! the idle queue and be handed onward (a cross-tenant reuse). Revoking
//! resource A must not block on, or be blocked by, in-flight traffic to an
//! unrelated resource B — the drain is **per-resource**, not manager-wide.
//!
//! ## Phase 1 — synchronous taint (before any `.await`)
//!
//! `Manager::revoke_slot` first sets a resource-scoped taint flag on the
//! resolved [`ManagedResource`]'s `taint` and, for the pooled topology,
//! bumps a per-row **revoke epoch** (its `bump_revoke_epoch`). Both run
//! **synchronously, before the first `.await`** of the revoke. The
//! taint reuses the same "stop new leases" mechanism as the per-handle
//! `ResourceGuard::taint` and the manager-wide `shutting_down` flag — one
//! shared mechanism, not a parallel one.
//!
//! **Why the taint must be synchronous-before-the-hook.** The engine rotation
//! fan-out wraps the awaited drain + revoke-hook tail in
//! `tokio::time::timeout`. A Rust `async fn` body is lazy: if the timeout
//! future is dropped before its first poll, the body never runs. Applying the
//! taint (and the epoch bump) in a synchronous phase that completes *before
//! and outside* any per-resource timeout guarantees that a dropped revoke
//! tail still leaves the row tainted and consistent — the credential is never
//! silently un-revoked; only the best-effort drain/hook tail is forgone.
//!
//! ## Phase 2 — cancellation-safe drain + hook tail
//!
//! Phase 1 produces a [`TaintedSlot`] (proof the taint already ran); passing
//! it to [`Manager::drain_and_revoke`] runs the tail: a bounded per-resource
//! in-flight drain followed by the `on_credential_revoke` hook. The tail has
//! exactly one owner of the per-resource time budget — the drain wait is
//! bounded by it (best-effort: a timed-out drain still proceeds to the hook)
//! and the hook is *separately* bounded by it. There is **no** caller-side
//! `tokio::time::timeout` wrapping the whole tail: such a wrapper could drop
//! the future *before the hook ran* when the drain was slow, contradicting
//! the "hook still runs after a timed-out drain" contract. The terminal
//! states are therefore reported explicitly ([`RevokeTail`]) rather than
//! inferred from a dropped outer future, and a hung *hook* is the only thing
//! the budget bounds — never the taint.
//!
//! ## The revoke-vs-acquire TOCTOU close
//!
//! The acquire pipeline pre-counts every acquire on the **per-resource**
//! in-flight counter using `InFlightCounter`, with an `AcqRel`
//! `fetch_add` issued **strictly before** a post-count re-check
//! (`Manager::reject_if_tainted_or_shutting_down_post_count`). The taint
//! gate runs at lookup, but a concurrent `revoke_slot` could taint *after*
//! that gate yet *before* the increment. Re-checking once this acquire is
//! reflected in the exact counter `revoke_slot` drains closes the window:
//! `revoke_slot` taints, then drains this same counter, so either the
//! acquire observes the taint at the re-check, or its increment is visible
//! to the drain and the drain waits for the resulting guard to drop. The
//! increment is held continuously — pre-counted at acquire, handed off to
//! the [`ResourceGuard`](crate::guard::ResourceGuard) on success (RAII
//! decrements and notifies on any failure / cancel / panic), decremented
//! only when the guard drops — so a guard handed out for a row is always
//! reflected in that row's revoke drain. The `AcqRel` ordering is the
//! TOCTOU primitive and is load-bearing: it is preserved verbatim and any
//! ordering tuning is a separate, separately-reviewed change.
//!
//! The **same post-count re-check** also closes the structurally identical
//! `graceful_shutdown` race (an acquire that passed `lookup()`'s
//! `shutdown_guard` while `shutting_down == false` must not complete *after*
//! the drain saw `0` and the registry was cleared). The pre-count alone is
//! *not* sufficient here: an acquire whose `InFlightCounter::new()`
//! increment lands *after* `wait_for_drain` already read `0` is invisible to
//! that drain, so without a re-check it would still hand out a
//! `ResourceGuard` for a drained-and-cleared resource (a logical
//! use-after-drain). `shutdown_guard` is therefore re-run *on the same
//! post-`InFlightCounter::new()` line* as the taint re-check: once the
//! increment is visible to `graceful_shutdown`'s drain, either the acquire
//! observes `shutting_down`/`cancel` and is rejected, or its increment is
//! seen by the drain and the drain waits for the resulting guard — exactly
//! symmetric with the revoke close. The per-resource counter feeds the
//! revoke drain; the manager-wide `drain_tracker` feeds `graceful_shutdown`.
//! An acquire pre-counts on **both**; the guard decrements + notifies
//! **both** on drop.
//!
//! ## Per-resource drain primitive
//!
//! The drain is a hand-rolled `(AtomicU64, Notify)` counter per
//! `ManagedResource` (plus the manager-wide twin for shutdown), not a
//! library tracker: `revoke_slot` drains the same per-resource counter on
//! *every* revoke event and the resource keeps serving acquires afterward
//! (taint stops the old credential's leases, not the resource), so the drain
//! is **repeated and non-terminal** — incompatible with a primitive whose
//! wait completes only on a terminal close, and with a single token that
//! cannot decrement both the manager-wide and per-resource counters. The
//! lost-wakeup-safe wait ordering is written **once** in the `shutdown`
//! submodule's `wait_for_tracker_drain` helper and shared by both the
//! manager-wide and per-resource drains.
//!
//! ## The pooled-topology revoke-epoch fence
//!
//! Only the pooled topology has an idle queue, so only it can re-admit an
//! instance authenticated with a now-revoked credential. The per-row revoke
//! epoch (bumped synchronously in Phase 1, before the hook walks the queue)
//! is snapshotted against each instance's checkout/creation epoch. **Every**
//! path that returns an instance to the idle queue — the release/recycle
//! path, an in-flight create that completes after the revoke, both warmup
//! paths, and the maintenance re-deposit — consults the epoch and
//! `destroy`s (never recycles or admits) an instance whose epoch is stale,
//! *before* `on_credential_revoke` is dispatched. The revoke epoch is
//! distinct from the pool fingerprint / lifetime / idle-timeout checks: an
//! instance can be non-stale and non-timed-out yet still hold a revoked
//! credential, so the existing eviction arms do not cover it. Single-runtime
//! topologies hold one shared runtime and dispatch the hook directly against
//! it under no idle-queue race — there is no return-to-idle site to fence,
//! and the epoch bump is a no-op for them.
//!
//! Note: this fences a revoked instance from being *recycled or created into
//! idle and handed onward*. It does **not** retroactively terminate an
//! already-authenticated in-flight session — that is impossible and a
//! deliberately weaker, different goal.
//!
//! # Architectural rationale (durable record)
//!
//! These decisions have no separate ADR; this section is their durable
//! record.
//!
//! ## Why there are two topology runtimes
//!
//! `Pooled` and `Resident` are distinct runtimes because their semantics
//! are structurally different. `Resident` has a `Lease: Clone` super-bound
//! and a create-vs-rotate epoch reconcile that a shared parameterized
//! runtime cannot express. `Pooled` owns the idle queue and the
//! revoke-epoch fence described above. These differences require distinct
//! runtime implementations rather than a shared parameterization.
//!
//! ## Why RCU was rejected for [`SlotCell`](crate::slot::SlotCell)
//!
//! `slot.rs` keeps its plain `store`/`swap` over a generation stamped
//! *inside* the swapped entry. An `arc-swap` `rcu` was considered and
//! rejected: `rcu`'s closure is `FnMut` and is **retried — called multiple
//! times — under contention**, so a side-effecting generation bump performed
//! inside it would be executed more than once and produce **epoch gaps**.
//! The resident create-vs-rotate reconcile compares a runtime's recorded
//! generation against the live one for *equality of intent*; a gapped
//! generation sequence breaks that reconcile. The current model — a
//! strictly monotonic generation published in the same immutable entry as
//! the value through a single swap — is already torn-read-free (a reader
//! observes the generation and the guard it belongs to as one unit) and does
//! not need RCU. The only residual correctness question is whether the same
//! slot is ever stored by concurrent writers; that is an upstream
//! rotation-driver serialization fact, guarded by a dedicated concurrency
//! test, not an `arc-swap` property.
//!
//! # Deferred follow-up ledger (durable record, no ADR)
//!
//! The topology collapse + cross-tenant-barrier + latent-bug closure that
//! produced this module deliberately did **not** fix every issue it
//! surfaced. This ledger is the durable record of what was consciously left
//! for separate work, so nothing is silently inherited once the originating
//! plan is gone. Every item is also filed as a tracked issue (linked); this
//! ledger is the in-tree index, not the sole record. Severity is the item's
//! own risk, independent of when it is scheduled.
//!
//! ## Latent bugs surfaced but out of scope
//!
//! - **`reload_config` live-runtime application — CLOSED** ([#712]). The new
//!   config is applied to the live runtime lazily on the next acquire across
//!   *every* topology: Pooled evicts stale-fingerprint idle instances and
//!   recreates; Resident rebuilds its shared master on a changed config
//!   fingerprint (`Resident::clone_or_create`); Bounded-Exclusive evicts its
//!   reused instance via a fingerprint-aware `accept` (`Bounded::accept` /
//!   `Bounded::set_fingerprint`); Bounded-Capped/Unbounded create a fresh
//!   instance from the current config on every acquire. The `SwappedImmediately`
//!   outcome stays accurate (config swapped immediately; live runtime rebuilt
//!   lazily) — no eager drain-then-rebuild redesign was needed; see the
//!   **accepted relabel** note below.
//! - **Pool `CreateGuard` cancel-drop — residual saturation-only leak — LOW**
//!   ([#713]). The main cancel-drop path is **closed**: `EntryCreateGuard::drop`
//!   schedules `destroy_within` via the `ReleaseQueue` (see
//!   `runtime/acquire_loop.rs`), proven by
//!   `slot_create_guard_drop_destroys_via_release_queue`. The residual: under
//!   extreme double-full saturation (the primary *and* fallback release-queue
//!   channels both full) a rescue task's timeout can elapse — or the cancel
//!   token fire — before the destroy task is delivered, and the slot leaks,
//!   observable via `ReleaseQueue::dropped_count`. Best-effort by design
//!   (canon §11.4), not a normal-path defect.
//! - **Resident recreate vs dispatch — closed by `create_lock`** ([#714]).
//!   Both `clone_or_create` and `dispatch_resident_hook` take the same
//!   `create_lock` before touching the cell, and the
//!   `take()`→destroy→create→`store()` sequence runs under a continuously held
//!   lock with no yield that releases it. Dispatch therefore either observes
//!   the new runtime (correct delivery) or `None` because create failed
//!   (correct no-op — nothing is bound). No lost-revoke window exists; earlier
//!   ledger text overstated this.
//! - Graceful shutdown owns release workers through cooperative completion
//!   or abort acknowledgement. Cleanup stays open throughout handle drain.
//! - **`RecoveryTicket` Drop counts a panicked probe as an attempt — LOW**
//!   ([#716]). A defensible-but-untested default; recovery internals.
//!
//! ## Separable acquire-path perf micro-folds — LOW ([#717])
//!
//! The collapse took only the perf wins **inseparable** from it (one
//! generic acquire pipeline instead of five byte-identical ones; a single
//! registry resolution instead of a double `DashMap` walk). The separable
//! micro-allocation folds — per-acquire config re-clone hoist,
//! `resilience.clone()` → borrow, `OnceLock`-gated erased no-op accessors,
//! broadcast send gated on `receiver_count() > 0` — were excluded to honor
//! the shape-only scope boundary. The `InFlightCounter` `AcqRel` ordering
//! is the revoke-vs-acquire TOCTOU primitive and is preserved verbatim
//! regardless; any ordering tuning is a separate reviewed change with a
//! re-stated memory-model proof.
//!
//! ## Cross-crate dedup / layer placement — LOW ([#718])
//!
//! Cross-layer type relocation was explicitly out of scope (no ADR in this
//! work). Deferred: `ErrorKind` ≈ `nebula_error::ErrorCategory`
//! reconciliation; hardcoded acquire backoff vs
//! `nebula_resilience::BackoffConfig`; relocating the live `RecoveryGate` +
//! `ReleaseQueue` to `nebula-resilience`; unifying
//! `CreateGuard`/`SessionGuard` into one `DefuseGuard<T>`; revisiting the
//! `register_resolved` JSON/`{{ }}` expression coupling and its engine-ABI
//! positional shape (see the accepted-exception note below). The
//! `events.rs` `broadcast` → `nebula_eventbus::EventBus` migration listed
//! here originally has since **landed** (wired through `Manager`,
//! `ResourceGuard`, and `RecoveryGate`).
//!
//! ## Further `Manager` code-line reduction — LOW ([#719])
//!
//! `crates/resource/src/manager/mod.rs` is large. The structural
//! de-spaghettification root-cause goals **are** met — two topologies, one
//! generic `run_acquire` (no `run_*_acquire` clones), the ~17 register
//! shorthands + 3-deep chain
//! folded into one `register(RegistrationSpec)` funnel, the 8 prose
//! restatements of the revoke invariant collapsed into the single canonical
//! block above, dead surface removed, all type-enforced so the duplication
//! cannot regress. The literal origin "~800 line" target is **not** met:
//! the raw count is inflated by the canonical doc this refactor
//! deliberately centralizes here (it replaces an ADR), and the residual
//! code is the legitimate identity-agnostic-vs-identity-pinned method-pair
//! axis (two real lookup modes), not copy-paste. A generic over that axis
//! could fold the remaining `<op>` / `<op>_for_identity` pairs — a cosmetic
//! tightening, not a correctness fix.
//!
//! ## Accepted carve-outs (recorded, not silently inherited)
//!
//! - **`register_resolved` carries one `// guard-justified:`
//!   `#[allow(clippy::too_many_arguments)]`.** The four register-chain
//!   `too_many_arguments` allows the collapse targeted are gone; this last
//!   one is the irreducible engine ABI — the production engine registrar
//!   dispatches into `register_resolved` positionally with a 9-param
//!   JSON-driven shape, and collapsing it into a struct would re-introduce
//!   the navigation hop the single register funnel removed for the one
//!   erased call site. It is a candidate for the cross-crate-dedup
//!   follow-up ([#718]), not a defect. (The three `too_many_arguments`
//!   allows in `runtime/pool.rs` are pre-existing pool internals untouched
//!   by this work.)
//! - **R15/R16 cross-tenant fixes were latent, not live.** The original
//!   64-bit `DefaultHasher` barrier defect ([#684], **closed** —
//!   structurally fixed here via the collision-free `SlotIdentity`
//!   structural set) and the pooled revoke→recycle TOCTOU were not
//!   reachable in production (this crate is `frontier`; the credential→slot
//!   resolver that landed 2026-09-13 populates credential slots, and the
//!   resource-side caller this defect needs is still absent), which is why
//!   seam-coupled remediation was acceptable over a standalone hotfix.
//!
//! ## Consumer-migration history (honest record)
//!
//! The expand-contract migration of in-tree consumers initially named, but
//! did **not** migrate, the three `m6_*` example binaries
//! (`m6_postgres_pool`, `m6_resident_http`, `m6_telegram_multi_workflow`);
//! they were migrated to `RegistrationSpec` / the structural `SlotIdentity`
//! in a later, separately-committed step before the old surface was
//! deleted. Recorded so the migration history is not misread as
//! single-step.
//!
//! [#684]: https://github.com/vanyastaff/nebula/issues/684
//! [#712]: https://github.com/vanyastaff/nebula/issues/712
//! [#713]: https://github.com/vanyastaff/nebula/issues/713
//! [#714]: https://github.com/vanyastaff/nebula/issues/714
//! [#716]: https://github.com/vanyastaff/nebula/issues/716
//! [#717]: https://github.com/vanyastaff/nebula/issues/717
//! [#718]: https://github.com/vanyastaff/nebula/issues/718
//! [#719]: https://github.com/vanyastaff/nebula/issues/719

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering},
    },
    time::Instant,
};

use nebula_core::{LayerLifecycle, ResourceKey, ScopeLevel, context::Context as _};
use nebula_eventbus::EventBus;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::{
    error::Error,
    events::ResourceEvent,
    metrics::{ResourceOpsMetrics, ResourceOpsSnapshot},
    recovery::gate::GateState,
    registry::Registry,
    release_queue::{ReleaseQueue, ReleaseQueueHandle},
    resource::Provider,
    runtime::managed::ManagedResource,
};

pub(crate) mod acquire;
mod gate;
pub(crate) mod options;
mod registration;
mod retirement;
mod rotation;
pub(crate) mod shutdown;
mod shutdown_session;
#[cfg(test)]
mod shutdown_session_tests;

pub use options::{
    DrainTimeoutPolicy, ManagerConfig, RegisterOptions, RegistrationSpec, ShutdownConfig,
};
pub use rotation::{
    EpochRefreshOutcome, EpochRevokeOutcome, RevokeTail, SlotDeferralReason, SlotDispatchOutcome,
    SlotDrainOutcome, TaintedSlot,
};
pub use shutdown::{ShutdownError, ShutdownReport};

/// Snapshot of a resource's health and operational state.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResourceHealthSnapshot {
    /// The resource's unique key.
    pub key: ResourceKey,
    /// Current lifecycle phase.
    pub phase: crate::state::ResourcePhase,
    /// Recovery gate state (if a gate is attached).
    pub gate_state: Option<GateState>,
    /// Aggregate operation counters (present when a metrics registry is configured).
    pub metrics: Option<ResourceOpsSnapshot>,
    /// Config generation counter.
    pub generation: u64,
}

/// Central registry and lifecycle manager for all resources.
///
/// Owns the [`ReleaseQueue`] internally — callers never need to create,
/// pass, or shut down the queue manually. The queue is drained during
/// [`graceful_shutdown`](Self::graceful_shutdown).
///
/// Thread-safe: all internal state is behind concurrent data structures.
/// Share via `Arc<Manager>` across tasks.
///
/// [`graceful_shutdown`](Self::graceful_shutdown) is the only teardown
/// checkpoint. Dropping an open manager cannot await provider cleanup and
/// therefore classifies every remaining row as an observable
/// [`RetirementOrigin::ManagerDrop`](crate::RetirementOrigin::ManagerDrop)
/// abandonment instead of enqueueing work into workers that are about to be
/// aborted.
///
/// Slot-identity-pinned acquire (the `*_for_identity` entry points —
/// [`acquire_pooled_for_identity`](Self::acquire_pooled_for_identity),
/// [`acquire_resident_for_identity`](Self::acquire_resident_for_identity),
/// [`acquire_bounded_for_identity`](Self::acquire_bounded_for_identity))
/// exists for every built-in topology: it resolves the registry row whose
/// resolved `slot_identity` matches, so a caller that resolved tenant A's
/// credential reaches tenant A's runtime and never tenant B's. The
/// identity-agnostic [`acquire_pooled`](Self::acquire_pooled) /
/// [`acquire_resident`](Self::acquire_resident) /
/// [`acquire_bounded`](Self::acquire_bounded) /
/// [`acquire_any`](Self::acquire_any) methods stay fail-closed for the
/// no-identity caller: under a
/// multi-tenant `(key, scope)` (more than one resolved-credential
/// registration) they return
/// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) rather than
/// aliasing one tenant's runtime to another. Use the `*_for` variant
/// whenever the resolved slot identity is known.
pub struct Manager {
    pub(super) registry: Registry,
    #[cfg(feature = "rotation")]
    rotation_indexes:
        std::sync::Mutex<Vec<std::sync::Weak<crate::credential_fanout::ResourceFanoutIndex>>>,
    /// Serializes registry commits, credential admission/revoke and terminal snapshots.
    /// Never held across await.
    pub(super) admission: std::sync::Mutex<()>,
    pub(super) cancel: CancellationToken,
    pub(super) metrics: Option<ResourceOpsMetrics>,
    /// Shared lifecycle-event sink. Held behind `Arc` so the same
    /// [`EventBus`] can be wired into per-resource
    /// [`RecoveryGate`](crate::recovery::gate::RecoveryGate)s and into each
    /// [`ResourceGuard`](crate::guard::ResourceGuard) (for its
    /// `Released`-on-drop emit) without exposing the bus's internal transport
    /// across module boundaries.
    pub(super) event_bus: Arc<EventBus<ResourceEvent>>,
    pub(super) release_queue: Arc<ReleaseQueue>,
    pub(super) release_queue_handle: Arc<tokio::sync::Mutex<Option<ReleaseQueueHandle>>>,
    /// Tracks active `ResourceGuard`s for drain-aware shutdown.
    pub(super) drain_tracker: Arc<(AtomicU64, Notify)>,
    /// Terminal jobs, including rows already removed or replaced.
    pub(super) retirement_tracker: Arc<(AtomicU64, Notify)>,
    /// Bounded outside-queue retirement owner; dropping aborts its supervisor.
    retirement_supervisor: Arc<retirement::RetirementSupervisor>,
    /// Serializes shutdown drivers and retains the terminal task across caller cancellation.
    shutdown_state: tokio::sync::Mutex<shutdown_session::ShutdownState>,
    /// Fast admission fence flipped before the first shutdown await.
    pub(super) shutting_down: AtomicBool,
    /// Optional lifecycle handle for coordinated cancellation (spec 08).
    pub(super) lifecycle: Option<LayerLifecycle>,
    /// Manager-wide default acquire-slow-log threshold. See
    /// [`ManagerConfig::acquire_slow_threshold`] for the WARN contract;
    /// [`AcquireOptions::acquire_slow_threshold`](crate::options::AcquireOptions::acquire_slow_threshold)
    /// overrides this per call.
    pub(super) acquire_slow_threshold: Option<std::time::Duration>,
}

impl std::fmt::Debug for Manager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `registry` holds live `R`-typed resources and `lifecycle`
        // (`LayerLifecycle`) is not `Debug`; print the process-visible
        // state (shutdown flag, tuning knobs) instead of walking either.
        f.debug_struct("Manager")
            .field(
                "shutting_down",
                &self
                    .shutting_down
                    .load(std::sync::atomic::Ordering::Relaxed),
            )
            .field("has_metrics", &self.metrics.is_some())
            .field("acquire_slow_threshold", &self.acquire_slow_threshold)
            .finish_non_exhaustive()
    }
}

impl Manager {
    /// Creates a new empty manager with default configuration.
    pub fn new() -> Self {
        Self::with_config(ManagerConfig::default())
    }

    /// Creates a new empty manager with the given configuration.
    pub fn with_config(config: ManagerConfig) -> Self {
        Self::warn_once_if_panic_abort();
        let event_bus = Arc::new(EventBus::new(256));
        let cancel = CancellationToken::new();
        let (release_queue, release_queue_handle) = ReleaseQueue::new(config.release_queue_workers);
        let release_queue = Arc::new(release_queue);
        let metrics =
            config
                .metrics_registry
                .as_ref()
                .and_then(|reg| match ResourceOpsMetrics::new(reg) {
                    Ok(m) => Some(m),
                    Err(err) => {
                        tracing::warn!(?err, "failed to initialize resource operation metrics");
                        None
                    },
                });
        let acquire_slow_threshold = config.acquire_slow_threshold;
        let retirement_supervisor = Arc::new(retirement::RetirementSupervisor::new(
            Arc::clone(&release_queue),
            config.release_queue_workers,
            config.retirement_queue_capacity,
        ));
        Self {
            registry: Registry::new(),
            #[cfg(feature = "rotation")]
            rotation_indexes: std::sync::Mutex::new(Vec::new()),
            admission: std::sync::Mutex::new(()),
            cancel,
            metrics,
            event_bus,
            release_queue,
            release_queue_handle: Arc::new(tokio::sync::Mutex::new(Some(release_queue_handle))),
            drain_tracker: Arc::new((AtomicU64::new(0), Notify::new())),
            retirement_tracker: Arc::new((AtomicU64::new(0), Notify::new())),
            retirement_supervisor,
            shutdown_state: tokio::sync::Mutex::new(shutdown_session::ShutdownState::Open),
            shutting_down: AtomicBool::new(false),
            lifecycle: None,
            acquire_slow_threshold,
        }
    }

    /// Wires a production credential reverse index into resource retirement.
    ///
    /// Removal visits every distinct live weak reference to delete exact
    /// routing rows while it still holds lifecycle admission. The manager
    /// does not own an index and therefore cannot extend a rotation driver's
    /// lifetime.
    #[cfg(feature = "rotation")]
    pub fn attach_rotation_index(
        &self,
        index: &Arc<crate::credential_fanout::ResourceFanoutIndex>,
    ) {
        let mut indexes = self
            .rotation_indexes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        indexes.retain(|attached| attached.strong_count() != 0);
        if indexes
            .iter()
            .filter_map(std::sync::Weak::upgrade)
            .any(|attached| Arc::ptr_eq(&attached, index))
        {
            return;
        }
        indexes.push(Arc::downgrade(index));
    }

    #[cfg(feature = "rotation")]
    pub(super) fn attached_rotation_indexes(
        &self,
    ) -> Vec<Arc<crate::credential_fanout::ResourceFanoutIndex>> {
        let mut indexes = self
            .rotation_indexes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let live: Vec<_> = indexes
            .iter()
            .filter_map(std::sync::Weak::upgrade)
            .collect();
        indexes.retain(|attached| attached.strong_count() != 0);
        live
    }

    /// One-time, process-wide honesty check for `panic = "abort"` builds.
    /// Every author-hook dispatch in this crate
    /// ([`guard_author_hook`](crate::hook_guard::guard_author_hook)) bounds a
    /// `Provider`/`Topology` hook with a timeout *and* isolates a panic via
    /// `catch_unwind` — but `catch_unwind` cannot catch anything under
    /// `panic = "abort"`: the process aborts immediately on panic, before
    /// unwinding (and therefore `catch_unwind`) ever runs. A workspace built
    /// with the release-profile default (`panic = "abort"`) therefore gets
    /// the timeout bound only; a panicking hook takes the process down with
    /// it. This is a one-time, not a per-call, warning — the cost is fixed
    /// (a build-time profile choice), not per-dispatch.
    ///
    /// `#[cfg(panic = "abort")]` is a compile-time check (stable since Rust
    /// 1.60): the warning is compiled in only for a binary actually built
    /// under that panic strategy, not evaluated at runtime.
    #[cfg(panic = "abort")]
    fn warn_once_if_panic_abort() {
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| {
            tracing::warn!(
                "nebula-resource: this build uses panic = \"abort\" — author-hook \
                 panic isolation (guard_author_hook's catch_unwind) is inert under \
                 abort: a panicking Provider/Topology hook takes the whole process \
                 down instead of being caught and converted to a typed error. The \
                 hook-dispatch TIMEOUT bound still applies. See ADR-0093."
            );
        });
    }

    #[cfg(not(panic = "abort"))]
    fn warn_once_if_panic_abort() {}

    /// Attaches a [`LayerLifecycle`] for coordinated cancellation (spec 08).
    ///
    /// When set, the manager can participate in hierarchical shutdown
    /// orchestrated by a parent layer.
    #[must_use]
    pub fn with_lifecycle(mut self, lifecycle: LayerLifecycle) -> Self {
        self.lifecycle = Some(lifecycle);
        self
    }

    /// Returns a reference to the attached lifecycle, if any.
    pub fn lifecycle(&self) -> Option<&LayerLifecycle> {
        self.lifecycle.as_ref()
    }

    /// Subscribes to resource lifecycle events.
    ///
    /// Returns a [`Subscriber`](crate::Subscriber) that receives
    /// [`ResourceEvent`]s emitted during registration, removal, and
    /// acquisition. The buffer is fixed at 256 events: a slow consumer that
    /// falls behind has the *oldest* unread events skipped (the subscriber
    /// auto-recovers and re-positions to the latest event — it never returns
    /// a lag error). Use
    /// [`Subscriber::lagged_count`](crate::Subscriber::lagged_count) to
    /// observe how many events were skipped.
    pub fn subscribe_events(&self) -> crate::Subscriber<ResourceEvent> {
        self.event_bus.subscribe()
    }

    /// Defense A against the `graceful_shutdown` race: reject any acquire
    /// that arrives after `graceful_shutdown` has flipped the flag, even
    /// if the cancel token has not yet been observed (it is set the line
    /// after on the same task — see `shutdown::graceful_shutdown` Phase 1).
    /// Ordering: `graceful_shutdown` writes `shutting_down` with `AcqRel`,
    /// we read with `Acquire`, so we synchronize-with that write and any
    /// observation here implies the cancel will follow.
    pub(crate) fn shutdown_guard(&self) -> Result<(), Error> {
        if self.shutting_down.load(AtomicOrdering::Acquire) || self.cancel.is_cancelled() {
            return Err(Error::cancelled());
        }
        Ok(())
    }

    /// Maps a [`LookupOutcome`](crate::registry::LookupOutcome) onto the
    /// typed result, downcasting and applying the **fail-closed** rule:
    /// `Ambiguous` becomes a permanent (never-retry)
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) deny —
    /// a caller conflict, not a server error — rather than a
    /// silently-picked row, so two resolved credentials sharing one
    /// `(key, scope)` can never bleed into each other.
    fn resolve_typed<R: Provider>(
        outcome: crate::registry::HandleLookupOutcome,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        use crate::registry::HandleLookupOutcome;
        match outcome {
            HandleLookupOutcome::Found(any) => any
                .as_any_arc()
                .downcast::<ManagedResource<R>>()
                .map_err(|_| Error::not_found(&R::key())),
            HandleLookupOutcome::NotFound => Err(Error::not_found(&R::key())),
            HandleLookupOutcome::Ambiguous { rows } => Err(Error::ambiguous(format!(
                "{}: {rows} resolved-credential registrations exist at this scope; \
                 acquire without a resolved slot identity is refused to prevent \
                 cross-tenant runtime bleed — acquire via the resolved-slot-identity \
                 path",
                R::key()
            ))
            .with_resource_key(R::key())),
        }
    }

    /// Maps a [`PinnedLookup`](crate::registry::PinnedLookup) onto the typed
    /// result.
    ///
    /// There is **no `Ambiguous` arm**: a resolved slot identity pins
    /// exactly one row by construction, so the [`PinnedLookup`](crate::registry::PinnedLookup) type has no
    /// `Ambiguous` variant for this to handle — the cross-tenant-bleed
    /// failure mode the agnostic [`resolve_typed`](Self::resolve_typed)
    /// guards against is type-unrepresentable on the pinned path rather
    /// than a runtime branch.
    fn resolve_typed_pinned<R: Provider>(
        outcome: crate::registry::PinnedHandleLookup,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        use crate::registry::PinnedHandleLookup;
        match outcome {
            PinnedHandleLookup::Found(any) => any
                .as_any_arc()
                .downcast::<ManagedResource<R>>()
                .map_err(|_| Error::not_found(&R::key())),
            PinnedHandleLookup::NotFound => Err(Error::not_found(&R::key())),
        }
    }

    /// Triggers an immediate shutdown of all managed resources.
    ///
    /// Cancels the shared [`CancellationToken`], signaling all in-flight
    /// operations to stop. Cleanup remains available for late guard drops
    /// while the manager lives. Callers should await pending work separately.
    ///
    /// For a shutdown that waits for in-flight work to drain, use
    /// [`graceful_shutdown`](Self::graceful_shutdown).
    pub fn shutdown(&self) {
        let _admission = self
            .admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tracing::info!("resource manager shutting down");
        self.cancel.cancel();
        for managed in self.registry.all_managed() {
            managed.begin_close();
        }
    }

    /// Returns `true` if a resource with the given key is registered.
    pub fn contains(&self, key: &ResourceKey) -> bool {
        self.registry.contains(key)
    }

    /// Returns all registered resource keys.
    pub fn keys(&self) -> Vec<ResourceKey> {
        self.registry.keys()
    }

    /// Returns a reference to the aggregate metrics counters, if a
    /// metrics registry was configured.
    pub fn metrics(&self) -> Option<&ResourceOpsMetrics> {
        self.metrics.as_ref()
    }

    /// Returns the manager's cancellation token.
    ///
    /// Child tokens can be derived from this for per-resource cancellation.
    /// Cancelling it rejects acquires without closing the release queue.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    /// Returns `true` if the manager has been shut down.
    pub fn is_shutdown(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Returns a health snapshot for a registered resource.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound)
    /// if the resource is not registered for the given scope.
    pub fn health_check<R: Provider>(
        &self,
        scope: &ScopeLevel,
    ) -> Result<ResourceHealthSnapshot, Error> {
        let managed = self.lookup::<R>(scope)?;
        Ok(ResourceHealthSnapshot {
            key: R::key(),
            phase: managed.status().phase,
            gate_state: managed.recovery_gate.as_ref().map(|g| g.state()),
            metrics: self.metrics.as_ref().map(ResourceOpsMetrics::snapshot),
            generation: managed.generation(),
        })
    }

    /// Looks up a managed resource by key and scope, returning the
    /// read-only diagnostic view without lifecycle mutation or downcasting.
    ///
    /// Useful for diagnostics and admin APIs that don't need typed access.
    /// Returns `None` both when nothing is registered and when several
    /// resolved-credential rows share `(key, scope)` (ambiguous) — a
    /// diagnostic peek must not arbitrarily pick one tenant's row.
    pub fn get_any(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
    ) -> Option<crate::registry::ManagedResourceView> {
        match self.registry.get(key, scope) {
            crate::registry::LookupOutcome::Found(any) => Some(any),
            crate::registry::LookupOutcome::NotFound
            | crate::registry::LookupOutcome::Ambiguous { .. } => None,
        }
    }

    /// Diagnostic admission snapshot for a registered resource at
    /// `(key, scope)` — its advisory [`AdmissionPhase`](crate::topology::AdmissionPhase)
    /// and optional [`Load`](crate::topology::Load), bundled into an
    /// [`AdmissionStatus`](crate::topology::AdmissionStatus).
    ///
    /// Returns `None` both when nothing is registered and when several
    /// resolved-credential rows share `(key, scope)` (ambiguous) — mirroring
    /// [`get_any`](Self::get_any), a diagnostic peek must not arbitrarily pick
    /// one tenant's row.
    ///
    /// Advisory only: the authoritative admission gate is the acquire path's
    /// `try_reserve`. This surface is for admin APIs, dashboards, and
    /// load-balancer hints.
    pub fn admission_status(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
    ) -> Option<crate::topology::AdmissionStatus> {
        let handle = self.get_any(key, scope)?;
        Some(crate::topology::AdmissionStatus {
            phase: handle.admission_phase(),
            load: handle.admission_load(),
        })
    }

    /// Records acquire success/failure in aggregate metrics, the acquire-wait
    /// histogram, and emits the corresponding [`ResourceEvent`]; also checks
    /// the acquire-slow-log threshold.
    fn record_acquire_result<R: Provider>(
        &self,
        result: &Result<crate::guard::ResourceGuard<R>, Error>,
        started: Instant,
        ctx: &crate::context::ResourceContext,
        options: &crate::options::AcquireOptions,
    ) {
        // Resolve the resource key once: `R::key()` re-validates and re-interns
        // the literal on each call, and the error path emits up to two events.
        let key = R::key();
        let elapsed = started.elapsed();
        match result {
            Ok(_) => {
                if let Some(m) = &self.metrics {
                    m.record_acquire();
                }
                self.emit(ResourceEvent::AcquireSuccess {
                    key: key.clone(),
                    duration: elapsed,
                });
            },
            Err(e) => {
                if let Some(m) = &self.metrics {
                    m.record_acquire_error();
                }
                // `BackpressureDetected` is a topology-pressure signal
                // (semaphore full, max sessions reached). It is a strict
                // subset of `AcquireFailed` — we emit both so subscribers
                // that filter on pressure get a typed event without having
                // to parse error strings, while the unified
                // `AcquireFailed` stream remains the canonical "acquire
                // didn't succeed" feed.
                if matches!(e.kind(), crate::error::ErrorKind::Backpressure) {
                    self.emit(ResourceEvent::BackpressureDetected { key: key.clone() });
                }
                self.emit(ResourceEvent::AcquireFailed {
                    key: key.clone(),
                    kind: e.kind().clone(),
                    error: e.to_string(),
                });
            },
        }

        // Acquire wait-time histogram + waited/timed-out counters. A
        // deadline is "timed out" when it had already elapsed by the time
        // this (failed) acquire completed — mirrors sqlx/bb8's notion of an
        // acquire timeout, independent of which internal error path produced
        // the failure. Reuses the completion instant already captured in
        // `elapsed` (`started + elapsed`) rather than a fresh `Instant::now()`
        // here: the event emission above takes nonzero time, so a fresh read
        // could observe the deadline as elapsed even for a failure that
        // actually completed strictly before it.
        if let Some(m) = &self.metrics {
            let completed_at = started + elapsed;
            let timed_out = result.is_err() && options.deadline.is_some_and(|d| completed_at >= d);
            m.record_acquire_wait(elapsed, timed_out);
        }

        // Acquire-slow-log threshold — at most one WARN per acquire,
        // checked once here at completion. `AcquireOptions` overrides the
        // manager-wide default.
        if let Some(threshold) = options
            .acquire_slow_threshold
            .or(self.acquire_slow_threshold)
            && elapsed > threshold
        {
            tracing::warn!(
                target: "resource",
                %key,
                scope = ?ctx.scope(),
                elapsed = ?elapsed,
                threshold = ?threshold,
                "acquire exceeded the slow-acquire threshold"
            );
        }
    }

    /// Best-effort event emission. The `PublishOutcome` is intentionally
    /// discarded — events are observability aids, not delivery guarantees.
    fn emit(&self, event: ResourceEvent) {
        let _ = self.event_bus.emit(event);
    }
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}

// RAII guard that pre-counts an in-flight `acquire_*` call against both the
// manager-wide and per-resource drain trackers, from the moment `lookup()`
// succeeds until either (a) the acquire completes and the slot is handed off
// to the resulting `ResourceGuard`, or (b) the acquire fails / panics / is
// cancelled and the slot is decremented + waiters notified on drop. The
// `AcqRel` pre-increment ordered strictly before the post-taint re-check is
// the revoke-vs-acquire TOCTOU primitive, and the manager-wide pre-count is
// Defense B of the `graceful_shutdown` race (Defense A is the
// `shutting_down` check inside `Manager::lookup`). Two-phase-revoke / drain
// invariant: see the `manager` module documentation.

pub(crate) struct InFlightCounter {
    /// Manager-wide drain tracker — the `graceful_shutdown` drain primitive.
    manager: crate::guard::DrainTracker,
    /// Per-`ManagedResource` in-flight tracker — the *only* counter
    /// `revoke_slot` drains, so a revoke on one resource never blocks on a
    /// sibling's in-flight work. See the [`manager`](crate::manager) module
    /// docs for the canonical invariant.
    per_resource: crate::guard::DrainTracker,
    armed: bool,
}

impl InFlightCounter {
    /// Pre-counts an in-flight acquire against **both** the manager-wide
    /// drain tracker (shutdown) and the per-resource tracker (the revoke
    /// drain + the `AcqRel` taint→increment→re-check TOCTOU close — see the
    /// [`manager`](crate::manager) module docs).
    pub(crate) fn new(
        manager: crate::guard::DrainTracker,
        per_resource: crate::guard::DrainTracker,
    ) -> Self {
        manager.0.fetch_add(1, AtomicOrdering::AcqRel);
        per_resource.0.fetch_add(1, AtomicOrdering::AcqRel);
        Self {
            manager,
            per_resource,
            armed: true,
        }
    }

    /// Hand off the in-flight slot to a `ResourceGuard`. Both trackers stay
    /// incremented; the guard's drop decrements + notifies both.
    ///
    /// Disarms this counter so the slot is NOT decremented on drop. Returns
    /// `(manager_wide, per_resource)` for
    /// [`ResourceGuard::with_drain_tracker`](crate::guard::ResourceGuard::with_drain_tracker).
    pub(crate) fn release_to_guard(mut self) -> crate::guard::DrainTrackers {
        self.armed = false;
        (self.manager.clone(), self.per_resource.clone())
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.cancel.cancel();
        if matches!(
            self.shutdown_state.get_mut(),
            shutdown_session::ShutdownState::Open
        ) {
            for managed in self.registry.clear() {
                managed.set_phase(crate::state::ResourcePhase::ShuttingDown);
                self.prepare_retirement(managed, crate::events::RetirementOrigin::ManagerDrop)
                    .abandon("manager dropped before graceful terminal cleanup");
            }
        } else {
            // The session already owns this snapshot; the registry is only an index.
            self.registry.clear();
        }
        self.shutdown_state.get_mut().abort();
        *self.shutdown_state.get_mut() = shutdown_session::ShutdownState::Finished;
        self.release_queue.close();
    }
}

impl Drop for InFlightCounter {
    fn drop(&mut self) {
        if self.armed {
            for tracker in [&self.manager, &self.per_resource] {
                if tracker.0.fetch_sub(1, AtomicOrdering::AcqRel) == 1 {
                    tracker.1.notify_waiters();
                }
            }
        }
    }
}

#[cfg(test)]
mod shutdown_post_count_race_tests;

#[cfg(test)]
mod projection_admission_tests;
