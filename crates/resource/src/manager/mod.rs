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
//! - **`reload_config` never drains/rebuilds the live runtime — MED-HIGH**
//!   ([#712]). `reload_config` swaps the config `ArcSwap` (and the Pool
//!   fingerprint) but never drains in-flight work or rebuilds the
//!   caller-supplied live `Arc<R::Instance>` for any topology, so a reload
//!   that should rotate the running runtime is silently not applied to it.
//!   Deferred because the reload redesign (drain-then-rebuild + a truthful
//!   outcome contract) is a separate concern; see the **accepted relabel**
//!   note below for why this is a preserved no-op, not a regression.
//! - **Pool `CreateGuard` cancel-drop leaks the instance — MED** ([#713]).
//!   A *cancelled* acquire whose in-flight `create` already built an instance
//!   drops it synchronously without the async `destroy()`, leaking the
//!   server-side handle. (The *other* `CreateGuard` race — an in-flight
//!   create completing *after a revoke* — is the same isolation defect as
//!   the revoke→recycle TOCTOU and **was fixed** by the pooled revoke-epoch
//!   fence; only the cancelled-acquire leak remains.)
//! - **Resident recreate `take()`+destroy-under-lock vs dispatch — MED**
//!   ([#714]). The resident recreate clears the slot then destroys under the
//!   lock; a concurrent revoke/refresh dispatch in that window can run
//!   against the absent/old runtime, losing the revoke for that window.
//!   Resident internals, out of the collapse seam.
//! - **`graceful_shutdown` phase-4 detached workers can outlive
//!   `release_queue_timeout` — LOW** ([#715]). The timeout bounds the wait,
//!   not the detached release work; it eventually drains. shutdown.rs was
//!   not opened by the collapse.
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
//! - **`reload_config` returns `ReloadOutcome::SwappedImmediately` for all
//!   variants.** `reload_config` swaps the config `ArcSwap` (and the Pool
//!   fingerprint) but never drains or rebuilds the live runtime for any
//!   topology — that missing behavior is exactly [#712]. The enum label is
//!   accurate for the current behavior; the missing drain/rebuild is the
//!   deferred work.
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
//!   reachable in production (this crate is `frontier`; there is no
//!   production credential→slot resolver), which is why seam-coupled
//!   remediation was acceptable over a standalone hotfix.
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
//! [#715]: https://github.com/vanyastaff/nebula/issues/715
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

use nebula_core::{LayerLifecycle, ResourceKey, ScopeLevel};
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
mod rotation;
pub(crate) mod shutdown;

pub use options::{
    DrainTimeoutPolicy, ManagerConfig, RegisterOptions, RegistrationSpec, ShutdownConfig,
};
pub use rotation::{RevokeTail, TaintedSlot};
pub use shutdown::{ShutdownError, ShutdownReport};

/// Snapshot of a resource's health and operational state.
#[derive(Debug, Clone)]
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
/// Slot-identity-pinned acquire (the `*_for` entry points —
/// `acquire_{pooled,resident,service,transport,exclusive}_for`) exists for
/// every topology: it resolves the registry row whose resolved
/// `slot_identity` matches, so a caller that resolved tenant A's credential
/// reaches tenant A's runtime and never tenant B's. The identity-agnostic
/// `acquire_*` methods stay fail-closed for the no-identity caller: under a
/// multi-tenant `(key, scope)` (more than one resolved-credential
/// registration) they return
/// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) rather than
/// aliasing one tenant's runtime to another. Use the `*_for` variant
/// whenever the resolved slot identity is known.
pub struct Manager {
    pub(super) registry: Registry,
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
    pub(super) release_queue_handle: tokio::sync::Mutex<Option<ReleaseQueueHandle>>,
    /// Tracks active `ResourceGuard`s for drain-aware shutdown.
    pub(super) drain_tracker: Arc<(AtomicU64, Notify)>,
    /// CAS-guarded idempotency flag for `graceful_shutdown`. Flipped
    /// false → true by the winning caller; losers return
    /// [`ShutdownError::AlreadyShuttingDown`].
    pub(super) shutting_down: AtomicBool,
    /// Optional lifecycle handle for coordinated cancellation (spec 08).
    pub(super) lifecycle: Option<LayerLifecycle>,
}

impl Manager {
    /// Creates a new empty manager with default configuration.
    pub fn new() -> Self {
        Self::with_config(ManagerConfig::default())
    }

    /// Creates a new empty manager with the given configuration.
    pub fn with_config(config: ManagerConfig) -> Self {
        let event_bus = Arc::new(EventBus::new(256));
        let cancel = CancellationToken::new();
        let (release_queue, release_queue_handle) =
            ReleaseQueue::with_cancel(config.release_queue_workers, cancel.clone());
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
        Self {
            registry: Registry::new(),
            cancel,
            metrics,
            event_bus,
            release_queue: Arc::new(release_queue),
            release_queue_handle: tokio::sync::Mutex::new(Some(release_queue_handle)),
            drain_tracker: Arc::new((AtomicU64::new(0), Notify::new())),
            shutting_down: AtomicBool::new(false),
            lifecycle: None,
        }
    }

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
        outcome: crate::registry::LookupOutcome,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        use crate::registry::LookupOutcome;
        match outcome {
            LookupOutcome::Found(any) => any
                .as_any_arc()
                .downcast::<ManagedResource<R>>()
                .map_err(|_| Error::not_found(&R::key())),
            LookupOutcome::NotFound => Err(Error::not_found(&R::key())),
            LookupOutcome::Ambiguous { rows } => Err(Error::ambiguous(format!(
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
    /// exactly one row by construction, so the [`PinnedLookup`] type has no
    /// `Ambiguous` variant for this to handle — the cross-tenant-bleed
    /// failure mode the agnostic [`resolve_typed`](Self::resolve_typed)
    /// guards against is type-unrepresentable on the pinned path rather
    /// than a runtime branch.
    fn resolve_typed_pinned<R: Provider>(
        outcome: crate::registry::PinnedLookup,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        use crate::registry::PinnedLookup;
        match outcome {
            PinnedLookup::Found(any) => any
                .as_any_arc()
                .downcast::<ManagedResource<R>>()
                .map_err(|_| Error::not_found(&R::key())),
            PinnedLookup::NotFound => Err(Error::not_found(&R::key())),
        }
    }

    /// Triggers an immediate shutdown of all managed resources.
    ///
    /// Cancels the shared [`CancellationToken`], signaling all in-flight
    /// operations to stop. Callers should await pending work separately.
    ///
    /// For a shutdown that waits for in-flight work to drain, use
    /// [`graceful_shutdown`](Self::graceful_shutdown).
    pub fn shutdown(&self) {
        tracing::info!("resource manager shutting down");
        self.cancel.cancel();
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
    /// type-erased `Arc<dyn ManagedHandle>`.
    ///
    /// Useful for diagnostics and admin APIs that don't need typed access.
    /// Returns `None` both when nothing is registered and when several
    /// resolved-credential rows share `(key, scope)` (ambiguous) — a
    /// diagnostic peek must not arbitrarily pick one tenant's row.
    pub fn get_any(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
    ) -> Option<Arc<dyn crate::registry::ManagedHandle>> {
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

    /// Records acquire success/failure in aggregate metrics and emits
    /// the corresponding [`ResourceEvent`].
    fn record_acquire_result<R: Provider>(
        &self,
        result: &Result<crate::guard::ResourceGuard<R>, Error>,
        started: Instant,
    ) {
        // Resolve the resource key once: `R::key()` re-validates and re-interns
        // the literal on each call, and the error path emits up to two events.
        let key = R::key();
        match result {
            Ok(_) => {
                if let Some(m) = &self.metrics {
                    m.record_acquire();
                }
                self.emit(ResourceEvent::AcquireSuccess {
                    key,
                    duration: started.elapsed(),
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
                    key,
                    error: e.to_string(),
                });
            },
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

/// Recursively resolve `{{ … }}` expression templates inside a JSON tree.
///
/// Strings that contain template markers are routed through
/// [`ExpressionEngine::parse_template`] +
/// [`render_template`](nebula_expression::ExpressionEngine::render_template); strings without
/// markers, and all non-string scalars, pass through untouched. Object and array containers are
/// walked recursively.
///
/// Used by [`Manager::register_resolved`] to evaluate dynamic config values before serde
/// deserialization. This is the resource-side mirror of the engine's `ParamResolver` — it resolves
/// at register time rather than at node dispatch time.
fn resolve_json_templates(
    value: serde_json::Value,
    engine: &nebula_expression::ExpressionEngine,
    ctx: &nebula_expression::EvaluationContext,
) -> Result<serde_json::Value, Error> {
    use serde_json::Value;
    match value {
        Value::String(s) => {
            if !s.contains("{{") {
                return Ok(Value::String(s));
            }
            let template = engine.parse_template(&s).map_err(|e| {
                Error::permanent(format!(
                    "register_resolved: template parse failed for `{s}`: {e}"
                ))
            })?;
            let rendered = engine.render_template(&template, ctx).map_err(|e| {
                Error::permanent(format!(
                    "register_resolved: template render failed for `{s}`: {e}"
                ))
            })?;
            Ok(Value::String(rendered))
        },
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve_json_templates(item, engine, ctx)?);
            }
            Ok(Value::Array(out))
        },
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                out.insert(k, resolve_json_templates(v, engine, ctx)?);
            }
            Ok(Value::Object(out))
        },
        other => Ok(other),
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
mod shutdown_post_count_race_tests {
    //! Finding #2 — `graceful_shutdown`-vs-acquire use-after-drain.
    //!
    //! `lookup()` runs `shutdown_guard()` (Defense A) *before* the
    //! `InFlightCounter::new()` increment. An acquire that passes `lookup()`
    //! while `shutting_down == false`, then has its increment land *after*
    //! `wait_for_drain` already observed `0` and `registry.clear()` ran, is
    //! a logical use-after-drain: the post-`InFlightCounter::new()` re-check
    //! (`reject_if_tainted_post_count`) only observed taint, never
    //! `shutting_down`, so the acquire completed and a `ResourceGuard` was
    //! handed out for a resource the manager had already drained and cleared.
    //!
    //! This is structurally identical to the revoke path, which *is* closed
    //! by a symmetric taint pre-check + post-count re-check. The shutdown
    //! path had the pre-check (`lookup`'s `shutdown_guard`) but no symmetric
    //! post-count re-check.
    //!
    //! The race window (`lookup` → `InFlightCounter::new`) has no `.await`,
    //! so this test reproduces the interleave deterministically by splitting
    //! it at exactly that seam: resolve the managed row via the same private
    //! lookup `acquire_resident` uses (while `shutting_down == false`), then
    //! run `graceful_shutdown`'s Phase 1–3 (signal + drain-sees-`0` because
    //! the counter increment has not happened yet + `registry.clear()`),
    //! then drive the private post-lookup tail (`run_acquire`) with that
    //! resolved row. Pre-fix the tail succeeds and hands out a guard for a
    //! cleared registry; post-fix it must reject with `Cancelled`.

    use std::{sync::Arc, time::Duration};

    use nebula_core::{ExecutionId, ResourceKey, resource_key, scope::Scope};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::{
        TopologyTag,
        context::ResourceContext,
        error::ErrorKind,
        options::AcquireOptions,
        resource::{HasCredentialSlots, ResourceConfig, ResourceMetadata},
        topology::{Resident, resident::config::Config as ResidentConfig},
    };

    #[derive(Clone, Default)]
    struct RaceCfg;

    nebula_schema::impl_empty_has_schema!(RaceCfg);

    impl ResourceConfig for RaceCfg {
        fn fingerprint(&self) -> u64 {
            // Unit struct: all instances identical — constant 0 is correct.
            0
        }
    }

    #[derive(Clone)]
    struct ShutdownRaceResident;

    #[async_trait::async_trait]
    impl Provider for ShutdownRaceResident {
        type Config = RaceCfg;
        type Instance = ();
        type Topology = Resident<Self>;

        fn key() -> ResourceKey {
            resource_key!("test.shutdown_post_count_race.resident")
        }

        async fn create(&self, _config: &RaceCfg, _ctx: &ResourceContext) -> Result<(), Error> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl HasCredentialSlots for ShutdownRaceResident {
        fn credential_slot_epoch(&self) -> u64 {
            0
        }
    }

    impl crate::topology::ResidentProvider for ShutdownRaceResident {
        fn is_alive_sync(&self, _runtime: &()) -> bool {
            true
        }
    }

    fn ctx() -> ResourceContext {
        let scope = Scope {
            execution_id: Some(ExecutionId::new()),
            ..Default::default()
        };
        ResourceContext::minimal(scope, CancellationToken::new())
    }

    fn register_race_resident(manager: &Manager, topology: Resident<ShutdownRaceResident>) {
        let spec = RegistrationSpec {
            resource: ShutdownRaceResident,
            config: RaceCfg,
            scope: ScopeLevel::Global,
            slot_identity: crate::dedup::SlotIdentity::Unbound,
            topology,
            recovery_gate: None,
        };
        assert!(manager.register(spec).is_ok(), "register succeeds");
    }

    /// Runs the resident acquire through the framework loop — the same
    /// monomorphic dispatch `run_acquire_dispatch` performs.
    async fn race_resident_acquire(
        managed: &Arc<ManagedResource<ShutdownRaceResident>>,
        ctx: &ResourceContext,
    ) -> Result<crate::guard::ResourceGuard<ShutdownRaceResident>, Error> {
        managed
            .run_acquire_loop(ctx, &AcquireOptions::default(), None)
            .await
    }

    /// Deterministic reproduction of the use-after-drain. The acquire
    /// resolves its row *before* shutdown (Defense A passes), shutdown then
    /// drains (sees `0` because the acquire has not yet hit
    /// `InFlightCounter::new()`) and clears the registry, and only *then*
    /// does the post-lookup acquire tail run. The tail must reject — the
    /// caller must NOT receive a `ResourceGuard` for a drained-and-cleared
    /// resource.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_acquire_rejects_when_drain_completed_after_lookup_passed() {
        let manager = Manager::new();
        let resident_rt = Resident::<ShutdownRaceResident>::new(ResidentConfig::default());
        register_race_resident(&manager, resident_rt);

        let acquire_ctx = ctx();

        // Step 1: the acquire passes `lookup()` (Defense A) while
        // `shutting_down == false`. This is the same private resolution
        // `acquire_resident` performs before `run_acquire`.
        let managed = manager
            .lookup_for_acquire_scope::<ShutdownRaceResident>(&acquire_ctx)
            .expect("lookup must succeed before shutdown starts");

        // Step 2: `graceful_shutdown` Phase 1–3 run *now*, while the
        // resolved-but-not-yet-counted acquire is parked between `lookup()`
        // and `InFlightCounter::new()`. The drain observes `0` (the
        // increment has not happened) and the registry is cleared.
        manager.shutting_down.store(true, AtomicOrdering::Release);
        manager.cancel.cancel();
        manager
            .wait_for_drain(Duration::from_secs(5))
            .await
            .expect("drain sees 0 — the racing acquire has not incremented yet");
        manager.registry.clear();

        // Step 3: only now does the post-lookup acquire tail run. Its
        // `InFlightCounter::new()` increment lands *after* the drain saw
        // `0` and the registry was cleared. The post-count re-check is the
        // last line of defense; it must reject.
        let result = manager
            .run_acquire(Arc::clone(&managed), || {
                let managed = Arc::clone(&managed);
                let ctx = &acquire_ctx;
                async move { race_resident_acquire(&managed, ctx).await }
            })
            .await;

        match result {
            Err(e) if matches!(e.kind(), ErrorKind::Cancelled) => {
                // Correct: the acquire whose counter landed after the drain
                // completed is rejected — no guard for a cleared registry.
            },
            Ok(guard) => panic!(
                "use-after-drain: run_acquire handed out a {:?} guard for a \
                 resource whose drain completed and registry was cleared \
                 before the in-flight increment landed",
                guard.topology_tag()
            ),
            Err(other) => {
                panic!("expected Cancelled (post-count shutdown re-check), got {other:?}")
            },
        }

        // The drained guard must not leave a leaked in-flight count behind.
        assert_eq!(
            manager.drain_tracker.0.load(AtomicOrdering::Acquire),
            0,
            "rejected acquire must not leak a manager-wide in-flight count"
        );
    }

    /// Sanity twin: when shutdown has *not* started, the identical
    /// post-lookup tail succeeds and hands out a real resident guard. This
    /// pins that the fix rejects *only* the drained-after-lookup race, not
    /// every acquire (no false-positive regression of the happy path).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_acquire_still_succeeds_when_not_shutting_down() {
        let manager = Manager::new();
        let resident_rt = Resident::<ShutdownRaceResident>::new(ResidentConfig::default());
        register_race_resident(&manager, resident_rt);

        let acquire_ctx = ctx();
        let managed = manager
            .lookup_for_acquire_scope::<ShutdownRaceResident>(&acquire_ctx)
            .expect("lookup succeeds");

        let result = manager
            .run_acquire(Arc::clone(&managed), || {
                let managed = Arc::clone(&managed);
                let ctx = &acquire_ctx;
                async move { race_resident_acquire(&managed, ctx).await }
            })
            .await;

        let guard = result.expect("acquire must succeed when not shutting down");
        assert_eq!(guard.topology_tag(), TopologyTag::Resident);
    }
}
