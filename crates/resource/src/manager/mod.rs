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
//! `fetch_add` issued **strictly before** a post-taint re-check
//! (`Manager::reject_if_tainted_post_count`). The taint gate runs
//! at lookup, but a concurrent `revoke_slot` could taint *after* that gate
//! yet *before* the increment. Re-checking once this acquire is reflected in
//! the exact counter `revoke_slot` drains closes the window: `revoke_slot`
//! taints, then drains this same counter, so either the acquire observes the
//! taint at the re-check, or its increment is visible to the drain and the
//! drain waits for the resulting guard to drop. The increment is held
//! continuously — pre-counted at acquire, handed off to the
//! [`ResourceGuard`](crate::guard::ResourceGuard) on success (RAII
//! decrements and notifies on any failure / cancel / panic), decremented
//! only when the guard drops — so a guard handed out for a row is always
//! reflected in that row's revoke drain. The `AcqRel` ordering is the
//! TOCTOU primitive and is load-bearing: it is preserved verbatim and any
//! ordering tuning is a separate, separately-reviewed change.
//!
//! This same pre-count also closes a second race — the `graceful_shutdown`
//! race (an acquire that passed `lookup()` before `cancel.cancel()` must not
//! complete *after* the drain saw `0` and the registry was cleared). The
//! per-resource counter feeds the revoke drain; the manager-wide
//! `drain_tracker` feeds `graceful_shutdown`. An acquire pre-counts on
//! **both**; the guard decrements + notifies **both** on drop.
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
//! ## Why the topology taxonomy is three runtimes, not five
//!
//! The resource topologies were collapsed from five to three. Two axes carry
//! all real variation: the **concurrency cap** and the **per-acquire hook
//! pair** (acquire / release-shape). `Pooled` and `Resident` stay distinct
//! runtimes — `Resident` has a `Lease: Clone` super-bound and a
//! create-vs-rotate epoch reconcile that the folded runtime cannot express,
//! and `Pooled` owns the idle queue and the revoke-epoch fence above. The
//! former `Service` / `Transport` / `Exclusive` topologies differed only in
//! cap and release-shape, so they fold into one parameterized `Bounded`
//! runtime whose cap and release-shape are **type-enforced** via a sealed
//! `Cap` typestate marker (`Unbounded` / `Capped<N>` / `Exclusive`). A
//! sealed typestate makes "a tracked service that never releases" or "an
//! exclusive runtime without reset ordering" a compile error instead of a
//! runtime `==` branch that could silently no-op — invalid states are
//! unrepresentable rather than discipline-checked.
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

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering},
    },
    time::Instant,
};

use nebula_core::{Context, LayerLifecycle, ResourceKey, ScopeLevel};
use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;

use crate::{
    context::ResourceContext,
    error::Error,
    events::ResourceEvent,
    integration::AcquireResilience,
    metrics::{ResourceOpsMetrics, ResourceOpsSnapshot},
    options::AcquireOptions,
    recovery::gate::{GateState, RecoveryGate},
    registry::Registry,
    release_queue::{ReleaseQueue, ReleaseQueueHandle},
    reload::ReloadOutcome,
    resource::Resource,
    runtime::{TopologyRuntime, managed::ManagedResource},
};

pub(crate) mod acquire_dispatch;
mod execute;
mod gate;
pub(crate) mod options;
pub(crate) mod shutdown;

pub use crate::registry::ErasedAcquireFn;
use execute::execute_with_resilience;
use gate::{admit_through_gate, settle_gate_admission};
pub use options::{
    DrainTimeoutPolicy, ManagerConfig, RegisterOptions, RegistrationSpec, ShutdownConfig,
};
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

/// A resource registry row whose credential slot has been **synchronously
/// tainted** by [`Manager::taint_slot`](Manager::taint_slot) /
/// [`Manager::taint_slot_for_identity`](Manager::taint_slot_for_identity) —
/// phase 1 of the
/// two-phase revoke (see the [`manager`](crate::manager) module docs for the
/// canonical invariant and why the taint is synchronous-before-the-tail).
///
/// Holding one is proof the taint already ran to completion: new acquires on
/// this row's credential are already rejected. It is consumed by
/// [`Manager::drain_and_revoke`](Manager::drain_and_revoke) to run the
/// cancellation-safe drain + revoke-hook tail.
///
/// Opaque by design: the only valid use is to pass it to
/// [`drain_and_revoke`](Manager::drain_and_revoke). It is **not** `Clone` —
/// one taint maps to exactly one drain/revoke tail.
#[must_use = "a TaintedSlot only completes the revoke when passed to Manager::drain_and_revoke"]
pub struct TaintedSlot {
    /// Structural key of the tainted resource registry row (span/event
    /// label only — no credential material).
    key: ResourceKey,
    /// The credential slot on that row that was revoked.
    slot: String,
    /// The resolved row whose taint flag was already set synchronously.
    managed: Arc<dyn crate::registry::AnyManagedResource>,
    /// When the synchronous taint was applied — the drain/revoke duration
    /// metric spans from here so it covers the whole revoke, not just the
    /// awaited tail.
    tainted_at: Instant,
}

impl std::fmt::Debug for TaintedSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omits `managed` (not `Debug`, and an internal
        // erased handle); only the credential-free routing labels.
        f.debug_struct("TaintedSlot")
            .field("key", &self.key)
            .field("slot", &self.slot)
            .finish_non_exhaustive()
    }
}

/// Outcome of the cancellation-safe revoke tail
/// ([`Manager::drain_and_revoke`]).
///
/// The tail has exactly one owner of the per-resource time budget (the
/// `drain_timeout` argument): the drain wait is bounded by it
/// (best-effort — a timed-out drain still proceeds to the hook), and the
/// revoke hook is *separately* bounded by it. There is **no** caller-side
/// `tokio::time::timeout` wrapping the whole tail; the three terminal states
/// are reported here rather than inferred from a dropped outer future. See
/// the [`manager`](crate::manager) module docs for why an outer timeout
/// wrapper would be unsafe (it could drop the future before the hook ran):
///
/// - [`Done`](Self::Done) — the revoke hook completed `Ok`.
/// - [`HookFailed`](Self::HookFailed) — the hook returned `Err` (carried
///   verbatim).
/// - [`HookTimedOut`](Self::HookTimedOut) — the hook itself did not
///   complete within the budget. The row stays tainted (the taint ran in
///   the synchronous phase-1); only a *hung hook* is bounded, never the
///   taint, and never at the cost of skipping a hook after a slow drain.
#[derive(Debug)]
#[must_use = "the revoke tail outcome must be recorded (it is not a silent success)"]
pub enum RevokeTail {
    /// Drain + revoke hook completed; the hook returned `Ok`. (A
    /// best-effort drain timeout that still reached a successful hook is
    /// still `Done` — the drain timeout is non-fatal.)
    Done,
    /// The revoke hook returned an error. The row stays tainted; the
    /// inner error is preserved for the caller's outcome accounting.
    HookFailed(Error),
    /// The revoke hook did not complete within the per-resource budget
    /// (a wedged `on_credential_revoke`). The row stays tainted; this is
    /// the only thing the budget bounds.
    HookTimedOut,
}

impl RevokeTail {
    /// Adapts the tail outcome to `Result<(), Error>` for the back-compat
    /// convenience callers ([`Manager::revoke_slot`] /
    /// [`Manager::revoke_slot_for_identity`]) that run taint+tail
    /// back-to-back and
    /// only need pass/fail. A hook timeout becomes a retryable transient
    /// error (the row is tainted; a later retry is meaningful), distinct
    /// from a hook failure which carries the hook's own error.
    fn into_result(self) -> Result<(), Error> {
        match self {
            RevokeTail::Done => Ok(()),
            RevokeTail::HookFailed(e) => Err(e),
            RevokeTail::HookTimedOut => Err(Error::transient(
                "revoke hook timed out — row stays tainted, no new leases",
            )),
        }
    }
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
    pub(super) event_tx: broadcast::Sender<ResourceEvent>,
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
        let (event_tx, _) = broadcast::channel(256);
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
            event_tx,
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
    /// Returns a [`broadcast::Receiver`] that receives [`ResourceEvent`]s
    /// emitted during registration, removal, and acquisition. Slow consumers
    /// that fall behind the 256-event buffer will receive a
    /// [`RecvError::Lagged`](broadcast::error::RecvError::Lagged) on the
    /// next recv.
    pub fn subscribe_events(&self) -> broadcast::Receiver<ResourceEvent> {
        self.event_tx.subscribe()
    }

    /// Erased acquire hook for a resident row.
    ///
    /// Takes **no** slot-identity argument: the single-walk acquire
    /// resolution pins the row by the *caller's* runtime slot identity, so
    /// the registration-time identity never parameterised the hook. The
    /// structural register path ([`register_resolved`](Self::register_resolved))
    /// hands this hook in by value with no identity threading.
    #[must_use]
    pub fn erased_acquire_resident_for<R>() -> ErasedAcquireFn
    where
        R: crate::topology::resident::Resident + Resource + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        acquire_dispatch::erased_acquire_resident::<R>()
    }

    /// Erased acquire hook for a pooled row, structural-identity form.
    ///
    /// See [`erased_acquire_resident_for`](Self::erased_acquire_resident_for)
    /// — no slot-identity argument; the single-walk resolution pins the row.
    #[must_use]
    pub fn erased_acquire_pooled_for<R>() -> ErasedAcquireFn
    where
        R: crate::topology::pooled::Pooled + Clone + Resource + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        acquire_dispatch::erased_acquire_pooled::<R>()
    }

    /// Erased acquire hook for a [`Bounded`](crate::topology::bounded::Bounded)
    /// row.
    ///
    /// The registration-time hook for a `TopologyRuntime::Bounded` row. No
    /// slot-identity argument — the single-walk acquire resolution pins
    /// the row by the caller's runtime slot identity, and the release
    /// shape is the resource's [`Cap`](crate::topology::bounded::Bounded::Cap)
    /// typestate (resolved inside the pipeline), not a registration
    /// parameter.
    #[must_use]
    pub fn erased_acquire_bounded_for<R>() -> ErasedAcquireFn
    where
        R: crate::topology::bounded::BoundedRelease + Clone + Resource + Send + Sync + 'static,
        R::Runtime: Clone + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        acquire_dispatch::erased_acquire_bounded::<R>()
    }

    /// Registers a resource from a fully-specified [`RegistrationSpec`].
    ///
    /// This is the **single registration funnel**: the former 3-deep
    /// `register` → `register_with_identity` → `register_with_slot_identity`
    /// → internal-row-builder chain and the ~17 per-topology
    /// `register_<topo>[_with]` shorthands all collapse onto this one
    /// method fed by one struct. Callers that only need the historical
    /// single-row-per-`(key, scope)` behaviour pass
    /// [`RegistrationSpec::slot_identity`] =
    /// [`SlotIdentity::Unbound`](crate::dedup::SlotIdentity).
    ///
    /// Per slot model the `spec.resource` value is expected to have **all
    /// `#[credential]` slot fields already resolved and populated**.
    /// `Manager::register` does not itself resolve credential bindings —
    /// that is the responsibility of the caller (typically the engine
    /// dispatch layer that assembles `R` via the `FromConfig` trait emitted
    /// by `#[derive(Resource)]`).
    ///
    /// `spec.slot_identity` is the structural anti-bleed seam: two
    /// registrations of the same resource type at the same `spec.scope`
    /// whose resolved `(slot, credential)` bindings differ occupy
    /// **distinct** registry rows with **distinct** topology runtimes, so
    /// one tenant's runtime can never serve another tenant's resolved
    /// credential. Equality is exact and structural (no digest), so two
    /// distinct resolved binding sets can never alias.
    ///
    /// The resource is wrapped in a [`ManagedResource`] and stored in the
    /// registry under `R::key()`. If a resource with the same key, scope,
    /// and slot identity is already registered, it is silently replaced.
    /// The manager's internal [`ReleaseQueue`] is automatically shared with
    /// the managed resource — callers never need to create or manage it.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails on the provided config.
    pub fn register<R: Resource>(&self, spec: RegistrationSpec<R>) -> Result<(), Error> {
        use crate::resource::ResourceConfig as _;

        let RegistrationSpec {
            resource,
            config,
            scope,
            slot_identity,
            topology,
            acquire,
            resilience,
            recovery_gate,
        } = spec;

        config.validate()?;

        // #390 (pool min/max sanity) is enforced structurally by
        // `PoolRuntime::new`, which the caller has already invoked to build
        // the `TopologyRuntime::Pool` handed in here — an invalid
        // `(min_size, max_size)` cannot reach this funnel because the
        // `PoolRuntime` could not have been constructed. No separate
        // register-time pool-config check is needed (the deleted
        // `register_pooled[_with]` shorthands re-validated the raw config
        // only because they took it *before* building the runtime).

        let key = R::key();
        let managed = Arc::new(ManagedResource {
            resource,
            config: arc_swap::ArcSwap::from_pointee(config),
            topology,
            release_queue: Arc::clone(&self.release_queue),
            generation: AtomicU64::new(0),
            status: arc_swap::ArcSwap::from_pointee(crate::state::ResourceStatus::new()),
            resilience,
            recovery_gate,
            tainted: AtomicBool::new(false),
            in_flight: Arc::new((AtomicU64::new(0), Notify::new())),
        });

        let type_id = std::any::TypeId::of::<ManagedResource<R>>();
        self.registry.register(
            key.clone(),
            type_id,
            scope,
            slot_identity,
            managed.clone(),
            acquire,
        );

        // #387: everything below this point is a single funnel — the
        // resource is installed, so advance its phase from `Initializing`
        // to `Ready`. Failures are surfaced by `config.validate()` above,
        // which aborts before we reach this line.
        managed.set_phase(crate::state::ResourcePhase::Ready);

        if let Some(m) = &self.metrics {
            m.record_create();
        }
        let _ = self
            .event_tx
            .send(ResourceEvent::Registered { key: key.clone() });

        tracing::debug!(%key, "resource registered");
        Ok(())
    }

    /// Schema-validate an **already-resolved** config JSON tree against
    /// `<R::Config as HasSchema>::schema()` *without* registering anything.
    ///
    /// This is the pure validation core shared with
    /// [`register_resolved`](Self::register_resolved): it runs exactly
    /// the schema pass, the closed-set guard, and the `R::Config`
    /// deserialize step that the live path runs *after* template
    /// resolution — but performs **no** `{{ … }}` resolution, **no**
    /// `Manager` mutation, and constructs **no** `resource: R` /
    /// `TopologyRuntime<R>`. It is the seam a config-CRUD writer uses to
    /// reject a bad `ResourceEntry.config` *before* persistence, keeping
    /// config validation strictly separate from engine-activation live
    /// registration (INTEGRATION_MODEL integration seam.1 — live registration happens
    /// at engine activation, never at config-create time).
    ///
    /// Template resolution is deliberately excluded: `{{ … }}` is resolved
    /// against the engine's expression context at activation, which does
    /// not exist at config-create time. A stored config may legitimately
    /// still carry unresolved templates; validating the *post-resolution*
    /// shape is an activation-time concern.
    ///
    /// On success returns the validated, deserialized `R::Config`: the
    /// closed-set guard and `serde_json::from_value::<R::Config>` already
    /// run here, so the live `register_resolved` path consumes this
    /// owned value directly instead of deserializing the same JSON twice.
    ///
    /// # Errors
    ///
    /// - [`Error::permanent`] when the JSON is not a field tree, fails the
    ///   `R::Config` schema (missing/invalid declared fields, `#[validate]`
    ///   rules), or fails to deserialize into `R::Config`.
    /// - [`Error::permanent`] when the config carries a top-level field the
    ///   `R::Config` schema does not declare (closed-set guard):
    ///   `ResourceConfig` must carry no secrets, so an inlined
    ///   secret-shaped field is rejected here rather than silently ignored
    ///   (product credential boundary). The error names only the offending key,
    ///   never its value.
    pub fn validate_config_value<R>(config_json: serde_json::Value) -> Result<R::Config, Error>
    where
        R: Resource,
        R::Config: serde::de::DeserializeOwned,
    {
        // Schema-validate against <R::Config as HasSchema>::schema(). This is
        // independent of serde::Deserialize: it surfaces missing/invalid fields a
        // serde default impl would silently accept, and runs the schema's
        // `#[validate(...)]` rules (length, pattern, …). Schema check runs FIRST so
        // structural errors are reported as schema violations rather than
        // confusingly re-routed through serde.
        let schema = <R::Config as nebula_schema::HasSchema>::schema();
        let field_values =
            nebula_schema::FieldValues::from_json(config_json.clone()).map_err(|e| {
                Error::permanent(format!("validate_config_value: invalid field tree: {e}"))
            })?;
        if let Err(report) = schema.validate(&field_values) {
            return Err(Error::permanent(format!(
                "validate_config_value: schema validation failed: {report:?}"
            )));
        }

        // Closed-set guard: reject any config key the typed `R::Config` schema does
        // not declare. `nebula_schema::Schema::validate` only checks *declared*
        // fields and silently ignores unknown ones, so without this an operator
        // could inline a secret-shaped field (e.g. `password`) into
        // `ResourceConfig` and get no signal — `ResourceConfig` must carry no
        // secrets; secrets reach a resource ONLY via typed credential slots
        // (product credential boundary; slot model; engine credential orchestration redaction; credential isolation
        // isolation). The error names only the offending KEY, never its value, so
        // a mis-wired secret can never leak through the rejection message.
        //
        // Skipped when the schema declares no fields: an empty `ValidSchema` is
        // the "schema not yet declared" sentinel (`impl_empty_has_schema!`), and a
        // closed set over zero fields would reject every config — that gate
        // belongs to types that have opted into a real schema.
        let declared = schema.fields();
        if !declared.is_empty()
            && let Some((unknown, _)) = field_values
                .iter()
                .find(|(k, _)| !declared.iter().any(|f| f.key() == *k))
        {
            return Err(Error::permanent(format!(
                "validate_config_value: config field `{unknown}` is not declared by \
                 the `{ty}` schema; secrets must not be inlined into ResourceConfig \
                 — bind them through a typed credential slot instead \
                 (product credential boundary)",
                unknown = unknown.as_str(),
                ty = std::any::type_name::<R::Config>(),
            )));
        }

        // Deserialize R::Config from the JSON to surface any residual
        // type-shape mismatch the structural schema pass did not, and
        // return the parsed value: the live `register_resolved` path
        // consumes this owned `R::Config` directly, so the JSON is
        // deserialized exactly once across validation + typed dispatch.
        serde_json::from_value::<R::Config>(config_json).map_err(|e| {
            Error::permanent(format!(
                "validate_config_value: failed to deserialize {ty} config from JSON: {e}",
                ty = std::any::type_name::<R::Config>()
            ))
        })
    }

    /// JSON-driven registration keyed by the **collision-free structural**
    /// resolved-credential identity.
    ///
    /// The JSON-driven registration entry: it resolves `{{ … }}` templates,
    /// schema-validates, and dispatches into the single
    /// [`register`](Self::register) funnel. Phase order: slot-binding
    /// validation → `{{ … }}` template resolution → schema + closed-set
    /// guard + `R::Config` deserialize → dispatch into the single funnel.
    /// The registry row is keyed by the structural
    /// [`SlotIdentity`](crate::dedup::SlotIdentity) derived from the
    /// resolved `(slot, credential)` bindings via
    /// [`SlotIdentity::from_bindings`](crate::dedup::SlotIdentity::from_bindings)
    /// — collision-free by exact string equality (no digest). Two
    /// registrations whose resolved bindings differ are distinct rows by
    /// construction, eliminating the cross-tenant-bleed failure mode a
    /// digest exposes rather than shrinking it.
    ///
    /// The derived structural identity is **returned** so the caller (the
    /// engine activation loop) records it for the acquire path and the
    /// rotation fan-out reverse index, addressing the *same* registry row
    /// this method created. The erased `acquire` hook is passed by value
    /// (not a `Fn(slot_id)` factory): the single-walk acquire resolution
    /// pins the row by the *caller's* runtime slot identity, so the
    /// registration-time identity no longer parameterises the hook.
    ///
    /// `nebula-resource → nebula-expression` is allowed under deny.toml's
    /// `[[bans]]` `nebula-resource` wrapper allowlist (Business → Core layer
    /// edge per typed ref fields / Phase 9, R-040 R8).
    ///
    /// # Errors
    ///
    /// - [`Error::permanent`] when expression resolution, JSON
    ///   deserialization, or schema validation fails.
    /// - [`Error::permanent`] when the config carries a top-level field the
    ///   `R::Config` schema does not declare (closed-set guard):
    ///   `ResourceConfig` must carry no secrets, so an inlined secret-shaped
    ///   field is rejected here rather than silently ignored (product
    ///   credential boundary). The error names only the offending key, never
    ///   its value.
    /// - [`Error::permanent`] when a `slot_bindings` key does not correspond
    ///   to a declared credential slot on `R`.
    /// - Any [`Error`](Error) returned by the underlying typed
    ///   [`register`](Self::register).
    #[tracing::instrument(
        level = "debug",
        target = "nebula_resource::register_resolved",
        skip_all,
        fields(
            resource_key = %R::key(),
            slot_count = slot_bindings.len(),
        )
    )]
    // guard-justified: the production engine registrar dispatches into this positionally (config_json + expr_engine + slot_bindings + resource + scope + topology + acquire + the two optional policies), so the 9-param JSON-driven shape is the engine ABI — collapsing it into a struct would re-introduce the navigation hop the single funnel removed and is not warranted for the one erased call site.
    #[allow(
        clippy::too_many_arguments,
        reason = "engine-facing JSON-driven structural-identity entry: the production engine registrar calls register_resolved positionally; collapsing the 9-param shape into a struct would re-introduce a navigation hop for the one erased call site, and the body itself builds one RegistrationSpec and delegates to the single register() funnel"
    )]
    pub async fn register_resolved<R>(
        &self,
        config_json: serde_json::Value,
        expr_engine: &nebula_expression::ExpressionEngine,
        slot_bindings: std::collections::HashMap<String, nebula_core::CredentialKey>,
        resource: R,
        scope: ScopeLevel,
        topology: TopologyRuntime<R>,
        acquire: ErasedAcquireFn,
        resilience: Option<AcquireResilience>,
        recovery_gate: Option<Arc<RecoveryGate>>,
    ) -> Result<crate::dedup::SlotIdentity, Error>
    where
        R: Resource + nebula_core::DeclaresDependencies,
        R::Config: serde::de::DeserializeOwned,
    {
        // 0. Validate that every binding matches a declared credential slot.
        //    Hard error on unknown slot — refuses to register a resource
        //    whose credential surface diverged from the one the workflow
        //    JSON specified, so misconfiguration surfaces at register time
        //    rather than as a confusing rotation no-op later.
        let deps = R::dependencies();
        for slot_name in slot_bindings.keys() {
            let known = deps.slot_fields().iter().any(|sf| {
                sf.slot_key == slot_name.as_str()
                    && matches!(
                        sf.kind,
                        nebula_core::dependencies::SlotKind::Credential { .. }
                    )
            });
            if !known {
                return Err(Error::permanent(format!(
                    "register_resolved: slot binding `{slot_name}` does not match any declared credential slot on `{}`",
                    std::any::type_name::<R>()
                )));
            }
        }

        // 1. Resolve `{{ … }}` templates inside the JSON tree.
        let ctx = nebula_expression::EvaluationContext::new();
        let resolved = resolve_json_templates(config_json, expr_engine, &ctx)?;

        // 2/2b/3. Schema pass + closed-set guard + `R::Config` deserialize.
        //    Shared verbatim with the config-CRUD validate seam via
        //    [`validate_config_value`](Self::validate_config_value) so the
        //    two paths cannot drift.
        let config: R::Config = Self::validate_config_value::<R>(resolved)?;

        // 4. Derive the **collision-free structural** slot identity from the
        //    resolved slot bindings. Equality is exact string equality over
        //    the canonical-sorted `(slot, credential)` pairs, so two
        //    registrations whose resolved credentials differ are distinct
        //    rows by construction (no digest, no collidable space). This is
        //    the structural barrier against cross-tenant runtime bleed
        //    (credential isolation, slot model). It carries no secret bytes
        //    — only a stable identity over the resolved binding *names*.
        let slot_identity = crate::dedup::SlotIdentity::from_bindings(
            slot_bindings
                .iter()
                .map(|(slot, cred)| (slot.as_str(), cred.as_str())),
        );

        // 5. Dispatch into the single typed register funnel via a
        //    `RegistrationSpec`. ResourceConfig::validate() runs inside
        //    `register`, so domain-level rules (PoolConfig sanity, host
        //    non-empty) are still enforced.
        tracing::debug!(
            target: "nebula_resource::register_resolved",
            ?slot_identity,
            "all pre-register checks passed; dispatching into typed register"
        );
        self.register(RegistrationSpec {
            resource,
            config,
            scope,
            slot_identity: slot_identity.clone(),
            topology,
            acquire,
            resilience,
            recovery_gate,
        })?;
        Ok(slot_identity)
    }

    /// Looks up a registered `ManagedResource<R>` by type and scope.
    ///
    /// This is the building block for acquire: callers retrieve the managed
    /// resource and then call the topology-specific acquire method directly.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered for the given scope.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    pub fn lookup<R: Resource>(
        &self,
        scope: &ScopeLevel,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        self.shutdown_guard()?;
        Self::resolve_typed::<R>(self.registry.get_typed::<R>(scope))
    }

    /// [`lookup`](Self::lookup) pinned to the **collision-free structural**
    /// resolved per-slot credential identity.
    ///
    /// Selects the registry row whose `slot_identity` matches, so a caller
    /// that resolved tenant A's credential can only ever reach tenant A's
    /// runtime. This is the read-side counterpart of
    /// [`register_resolved`](Self::register_resolved); use it whenever the
    /// resolved slot identity is known so the lookup is never ambiguous.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    pub fn lookup_for_identity<R: Resource>(
        &self,
        scope: &ScopeLevel,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        self.shutdown_guard()?;
        Self::resolve_typed_pinned::<R>(self.registry.get_typed_for::<R>(scope, slot_identity))
    }

    /// Defense A against the `graceful_shutdown` race: reject any acquire
    /// that arrives after `graceful_shutdown` has flipped the flag, even
    /// if the cancel token has not yet been observed (it is set the line
    /// after on the same task — see `shutdown::graceful_shutdown` Phase 1).
    /// Ordering: `graceful_shutdown` writes `shutting_down` with `AcqRel`,
    /// we read with `Acquire`, so we synchronize-with that write and any
    /// observation here implies the cancel will follow.
    fn shutdown_guard(&self) -> Result<(), Error> {
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
    fn resolve_typed<R: Resource>(
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
    fn resolve_typed_pinned<R: Resource>(
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

    /// Typed acquire lookup walking [`scope_levels_for_acquire`](crate::context::scope_levels_for_acquire)
    /// on the context scope bag, then [`taint_gate`](Self::taint_gate).
    fn lookup_for_acquire_scope<R: Resource>(
        &self,
        ctx: &ResourceContext,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        self.shutdown_guard()?;
        let managed =
            Self::resolve_typed::<R>(self.registry.get_typed_for_acquire_scope::<R>(ctx.scope()))?;
        Self::taint_gate::<R>(managed)
    }

    /// [`lookup_for_acquire_scope`](Self::lookup_for_acquire_scope) pinned to
    /// the **collision-free structural** resolved per-slot credential
    /// identity. The pinned lookup is 2-variant (no `Ambiguous`).
    fn lookup_for_acquire_with_identity<R: Resource>(
        &self,
        ctx: &ResourceContext,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        self.shutdown_guard()?;
        let managed = Self::resolve_typed_pinned::<R>(
            self.registry
                .get_typed_for_acquire::<R>(ctx.scope(), slot_identity),
        )?;
        Self::taint_gate::<R>(managed)
    }

    /// Downcasts the row already resolved by
    /// [`Registry::get_acquire_for`](crate::registry::Registry::get_acquire_for)'s
    /// single scope walk, then applies the shared shutdown + taint tail.
    ///
    /// The erased-acquire path threads the resolved
    /// `Arc<dyn AnyManagedResource>` out of that one walk (via
    /// [`AcquireLookupOutcome::Found`](crate::registry::AcquireLookupOutcome::Found)),
    /// so the typed handle is recovered by a **downcast of that exact
    /// row** — not a second `DashMap` walk at the matched scope. The
    /// resolved row is, by construction, the `ManagedResource<R>` the
    /// `erased_acquire_*::<R>` hook was registered alongside, so the
    /// downcast yields the identical handle the prior pinned re-walk
    /// would have. Failure mapping (`NotFound` on a type mismatch) and
    /// the [`taint_gate`](Self::taint_gate) tail are byte-identical to
    /// the replaced pinned-lookup path.
    fn downcast_resolved_row<R: Resource>(
        &self,
        managed: Arc<dyn crate::registry::AnyManagedResource>,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        use crate::registry::PinnedLookup;
        self.shutdown_guard()?;
        let managed = Self::resolve_typed_pinned::<R>(PinnedLookup::Found(managed))?;
        Self::taint_gate::<R>(managed)
    }

    /// Shared taint check tail for the acquire-side lookups.
    ///
    /// Every `acquire_*` path funnels through here so a single check
    /// rejects new leases once `revoke_slot` has tainted the resource.
    /// Diagnostic paths (`health_check`, `pool_stats`, `reload_config`) use
    /// the plain `lookup` so they keep working on a tainted resource.
    ///
    /// `warmup_pool` is routed through the acquire funnel (taint-gated) because
    /// it materializes instances via `R::create`.
    ///
    /// Taint rejects with [`ErrorKind::Revoked`](crate::error::ErrorKind::Revoked),
    /// distinct from [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled)
    /// raised by [`Self::shutdown_guard`].
    fn taint_gate<R: Resource>(
        managed: Arc<ManagedResource<R>>,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        if managed.is_tainted() {
            return Err(Self::tainted_error::<R>());
        }
        Ok(managed)
    }

    /// Post-`InFlightCounter::new` taint re-check shared by every
    /// `run_*_acquire` / `try_acquire_*` pipeline.
    ///
    /// The acquire-side [`taint_gate`](Self::taint_gate) ran before the
    /// in-flight counter was constructed, leaving a window where a concurrent
    /// `revoke_slot` could taint *after* the gate but *before* the increment.
    /// Re-checking here — once this acquire is reflected in the resource's
    /// own in-flight counter (the exact counter `revoke_slot` drains) — closes
    /// the revoke-vs-acquire TOCTOU. See the [`manager`](crate::manager) module
    /// docs for the canonical invariant. Same error/classification as the gate
    /// so the caller-facing category is unchanged
    /// (`Revoked` → `ErrorCategory::Unavailable`).
    fn reject_if_tainted_post_count<R: Resource>(
        managed: &Arc<ManagedResource<R>>,
    ) -> Result<(), Error> {
        if managed.is_tainted() {
            return Err(Self::tainted_error::<R>());
        }
        Ok(())
    }

    /// The single typed error both taint checks return — keeps the message
    /// and `Revoked` (→ `Unavailable`) classification identical at the
    /// pre-count gate and the post-count re-check.
    fn tainted_error<R: Resource>() -> Error {
        Error::revoked(format!(
            "{}: resource tainted by credential revoke — new acquires rejected",
            R::key()
        ))
        .with_resource_key(R::key())
    }

    /// Notifies a registered resource that one of its `#[credential]`
    /// slots was rotated, after the engine has installed the fresh guard.
    ///
    /// Resolves `(key, scope)` to the live [`ManagedResource`] via the same
    /// registry lookup the `acquire_*` family uses, then borrows the live
    /// `Runtime` per topology and invokes
    /// [`Resource::on_credential_refresh`] for `slot`. The slot cell itself
    /// lives on the author's resource struct and is populated/rotated by
    /// the engine through `&self` (`SlotCell::store`) — this method does
    /// **not** own a slot map; it only drives the per-resource hook.
    ///
    /// Emits [`ResourceEvent::SlotRefreshed`] on success or
    /// [`ResourceEvent::SlotRefreshFailed`] (with an already-stringified,
    /// credential-free error) on failure, and records the corresponding
    /// slot-refresh metric.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource is registered for
    ///   `key` at `scope`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_refresh` hook maps into [`Error`].
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_refresh",
        skip(self),
        fields(key = %key, slot = %slot, topology, duration_ms)
    )]
    pub async fn refresh_slot(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
    ) -> Result<(), Error> {
        let managed = self.lookup_any_for_slot(key, &scope)?;
        self.refresh_resolved(key, slot, managed).await
    }

    /// [`refresh_slot`](Self::refresh_slot) pinned to the **collision-free
    /// structural** resolved per-slot credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches (via the same
    /// unambiguous-by-construction path [`get_for`](crate::registry::Registry::get_for)
    /// backs), so a multi-tenant `(key, scope)` routes the rotation to the
    /// *specific* resolved row instead of failing closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous). This is
    /// the entry point the engine per-slot rotation fan-out drives once it
    /// has resolved a node's slot bindings; identity-agnostic
    /// [`refresh_slot`](Self::refresh_slot) stays fail-closed for the
    /// no-identity caller. The engine rotation fan-out records the
    /// structural [`SlotIdentity`](crate::dedup::SlotIdentity) at bind time,
    /// so routing is by exact string equality (no digest aliasing).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of `key` at `scope`
    ///   matches `slot_identity`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_refresh` hook maps into [`Error`].
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_refresh",
        skip(self, slot_identity),
        fields(key = %key, slot = %slot, topology, duration_ms)
    )]
    pub async fn refresh_slot_for_identity(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<(), Error> {
        let managed = self.lookup_any_for_slot_identity_structural(key, &scope, slot_identity)?;
        self.refresh_resolved(key, slot, managed).await
    }

    /// Post-resolution refresh dispatch shared by
    /// [`refresh_slot`](Self::refresh_slot) (identity-agnostic) and
    /// [`refresh_slot_for_identity`](Self::refresh_slot_for_identity)
    /// (slot-identity-pinned).
    ///
    /// The two public entry points differ only in how they resolve the row;
    /// the hook dispatch, metric (exactly one outcome per dispatch), and
    /// event emission are identical and live here.
    async fn refresh_resolved(
        &self,
        key: &ResourceKey,
        slot: &str,
        managed: Arc<dyn crate::registry::AnyManagedResource>,
    ) -> Result<(), Error> {
        let started = Instant::now();
        tracing::Span::current().record("topology", managed.topology_tag_erased().as_str());

        let result = managed.dispatch_on_refresh_erased(slot).await;
        tracing::Span::current().record("duration_ms", started.elapsed().as_millis() as u64);

        // Exactly one outcome per dispatch; the attempts total is the sum
        // across `outcome` labels (success + failed + timed_out).
        match &result {
            Ok(()) => {
                if let Some(m) = &self.metrics {
                    m.record_slot_refresh_outcome(crate::metrics::SlotDispatchOutcome::Success);
                }
                let _ = self.event_tx.send(ResourceEvent::SlotRefreshed {
                    key: key.clone(),
                    slot: slot.to_owned(),
                });
                tracing::debug!("slot refresh hook completed");
            },
            Err(e) => {
                if let Some(m) = &self.metrics {
                    m.record_slot_refresh_outcome(crate::metrics::SlotDispatchOutcome::Failed);
                }
                let _ = self.event_tx.send(ResourceEvent::SlotRefreshFailed {
                    key: key.clone(),
                    slot: slot.to_owned(),
                    error: e.to_string(),
                });
                tracing::warn!(error = %e, "slot refresh hook failed");
            },
        }
        result
    }

    /// **Phase 1 of the revoke port — synchronous, runs to completion before
    /// any `.await`.** Resolves the registry row pinned to the
    /// **collision-free structural** resolved per-slot credential identity
    /// and *taints it immediately* so the `acquire_*` funnel rejects new
    /// leases on the revoked credential, then returns a [`TaintedSlot`]
    /// handle the caller passes to
    /// [`drain_and_revoke`](Self::drain_and_revoke) for the cancellation-safe
    /// drain + hook tail.
    ///
    /// Why this is split off as a non-`async` function: the engine fan-out
    /// wraps the awaited tail in `tokio::time::timeout`. A Rust `async fn`
    /// body is *lazy* — if a `timeout` future is dropped before the runtime
    /// first polls it, the body never runs. Were the taint the first
    /// statement of an `async` body, a timeout that fired before the first
    /// poll would drop the future and **skip the taint entirely**, leaving
    /// new acquires accepted on a credential whose revoke "timed out". This
    /// function is plain `fn`: the taint is applied eagerly at the call site,
    /// fully completed before this returns, and therefore *outside* and
    /// *before* any per-resource timeout (per-resource revoke deferral).
    ///
    /// Identity routing: resolves the *exact* resolved registry row by
    /// structural string equality (no digest aliasing) via the
    /// unambiguous-by-construction
    /// [`get_for`](crate::registry::Registry::get_for) path, so a
    /// multi-tenant `(key, scope)` taints the *specific* resolved row
    /// instead of failing closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous). This is
    /// the entry point the engine per-slot rotation fan-out drives on a
    /// lease revoke; identity-agnostic [`taint_slot`](Self::taint_slot) stays
    /// fail-closed for the no-identity caller. Synchronous-before-`.await`
    /// taint guarantee; see the [`manager`](crate::manager) module docs for
    /// the canonical invariant.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of `key` at `scope`
    ///   matches `slot_identity`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    ///
    /// Carries only `key` / `slot` / `topology` (no credential material)
    /// onto the span.
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_taint",
        skip(self, slot_identity),
        fields(key = %key, slot = %slot, topology, op = "revoke")
    )]
    pub fn taint_slot_for_identity(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<TaintedSlot, Error> {
        let managed = self.lookup_any_for_slot_identity_structural(key, &scope, slot_identity)?;
        Ok(Self::taint_now(key, slot, managed))
    }

    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity) for the
    /// slot-identity-agnostic caller (the convenience
    /// [`revoke_slot`](Self::revoke_slot) path and non-fan-out
    /// callers/tests).
    ///
    /// Same eager, pre-`await` taint guarantee as
    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity); only row
    /// resolution differs (identity-agnostic, so a multi-tenant
    /// `(key, scope)` fails closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) rather
    /// than tainting an arbitrary tenant's row).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource is registered for
    ///   `key` at `scope`.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) if more than one
    ///   resolved-credential row exists for `(key, scope)`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_taint",
        skip(self),
        fields(key = %key, slot = %slot, topology, op = "revoke")
    )]
    pub fn taint_slot(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
    ) -> Result<TaintedSlot, Error> {
        let managed = self.lookup_any_for_slot(key, &scope)?;
        Ok(Self::taint_now(key, slot, managed))
    }

    /// Applies the taint synchronously and packages the [`TaintedSlot`]
    /// handle. Shared tail of [`taint_slot`](Self::taint_slot) /
    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity); the
    /// safety-critical
    /// invariant — *taint is fully applied before this returns* — is written
    /// once here. This is **phase 1** of the two-phase revoke; see the
    /// [`manager`](crate::manager) module docs for the canonical invariant
    /// (why both stores are synchronous-before-`.await`, the TOCTOU close,
    /// and the revoke-epoch fence).
    fn taint_now(
        key: &ResourceKey,
        slot: &str,
        managed: Arc<dyn crate::registry::AnyManagedResource>,
    ) -> TaintedSlot {
        tracing::Span::current().record("topology", managed.topology_tag_erased().as_str());
        // Phase-1 taint, synchronously before any caller `.await`: this
        // function is not `async`, so the store has already happened by the
        // time control returns and a subsequently-dropped drain-tail timeout
        // future cannot un-apply it.
        managed.taint_erased();
        // Phase-1 revoke-epoch bump, in the *same* synchronous pre-`.await`
        // step as the taint, so the pooled return-to-idle paths fence any
        // instance authenticated with the now-revoked credential before the
        // hook walks the idle queue.
        managed.bump_revoke_epoch_erased();
        TaintedSlot {
            key: key.clone(),
            slot: slot.to_owned(),
            managed,
            tainted_at: Instant::now(),
        }
    }

    /// Default per-resource revoke budget for the back-compat
    /// back-to-back convenience callers ([`revoke_slot`](Self::revoke_slot)
    /// / [`revoke_slot_for_identity`](Self::revoke_slot_for_identity)).
    ///
    /// 30 s — the same budget the manager-wide `graceful_shutdown` drain
    /// uses and the value [`drain_and_revoke`](Self::drain_and_revoke)
    /// previously hard-coded for the drain wait. The engine rotation
    /// fan-out does **not** use this: it passes its own per-resource
    /// rotation budget so the timeout has one owner end-to-end (resource runtime status
    /// §Deferred / #690 review).
    pub const DEFAULT_REVOKE_DRAIN_TIMEOUT: std::time::Duration =
        std::time::Duration::from_secs(30);

    /// **Phase 2 of the revoke port — the cancellation-safe awaited tail.**
    /// Consumes a [`TaintedSlot`] from [`taint_slot`](Self::taint_slot) /
    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity) (whose
    /// taint already ran
    /// synchronously) and performs the remaining steps:
    ///
    /// 1. **Drain** only *this resource's* in-flight handles via its own per-resource counter
    ///    (per-resource revoke deferral) — never the manager-wide `drain_tracker`, so a revoke is isolated
    ///    from in-flight traffic to unrelated resources.
    /// 2. **Dispatch** [`Resource::on_credential_revoke`] against the live runtime per topology.
    /// 3. Emit [`ResourceEvent::SlotRevoked`] / `SlotRevokeFailed`.
    ///
    /// **Single budget owner (per-resource revoke deferral / #690 review).** The
    /// `drain_timeout` argument is the caller's per-resource budget and is
    /// the *only* timeout governing this tail. It bounds **two** waits
    /// independently:
    ///
    /// - the per-resource **drain** — *best-effort*: a drain timeout is
    ///   non-fatal, it records the `TimedOut` outcome metric and the tail
    ///   **still proceeds to the revoke hook** (the taint already stops
    ///   *new* leases; the hook makes the resource stop emitting on the
    ///   old credential);
    /// - the **revoke hook** itself — a *wedged* `on_credential_revoke`
    ///   is the only thing the budget actually cuts short
    ///   ([`RevokeTail::HookTimedOut`]).
    ///
    /// The caller **must not** wrap this call in its own
    /// `tokio::time::timeout`. The previous design did, and a slow drain
    /// could make that outer timeout elapse and **drop the whole future
    /// before the hook ran** — silently skipping the documented
    /// "hook still runs after a timed-out drain" guarantee. Bounding both
    /// waits *inside* this method (one owner, no outer wrapper) means a
    /// timed-out drain can never skip the hook, and only a hung hook is
    /// bounded — never the taint.
    ///
    /// **Cancellation-safety.** The taint is *not* in this future — it
    /// ran in the synchronous
    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity)
    /// phase. So if this future *is* dropped anyway (an outer abort, task
    /// cancel), the row stays tainted and consistent: new acquires are
    /// still rejected, the credential is never silently un-revoked.
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_drain_revoke",
        skip(self, tainted),
        fields(
            key = %tainted.key,
            slot = %tainted.slot,
            topology = tainted.managed.topology_tag_erased().as_str(),
            duration_ms,
            op = "revoke",
        )
    )]
    pub async fn drain_and_revoke(
        &self,
        tainted: TaintedSlot,
        drain_timeout: std::time::Duration,
    ) -> RevokeTail {
        let TaintedSlot {
            key,
            slot,
            managed,
            tainted_at,
        } = tainted;

        // 1. Drain **only this resource's** in-flight handles (resource runtime status
        //    §Deferred): a revoke on resource A must not block on in-flight
        //    traffic to an unrelated resource B, so this awaits the row's
        //    own per-resource counter — not the manager-wide `drain_tracker`
        //    (which stays the `graceful_shutdown` primitive). Bounded by the
        //    caller's per-resource budget so a stuck handle on *this*
        //    resource cannot wedge revoke; the taint (already applied
        //    synchronously in the phase-1 function) already stops new
        //    leases.
        //
        //    A drain timeout is *terminal* for this dispatch's outcome
        //    metric: it records `TimedOut` and the subsequent hook
        //    success/failure does NOT record a second outcome (one dispatch
        //    = exactly one outcome). The hook still runs and its event /
        //    returned outcome are unaffected — this is the contract the
        //    removed outer `tokio::time::timeout` wrapper used to break.
        let drain_result = managed.wait_for_in_flight_drain_erased(drain_timeout).await;
        let drain_timed_out = drain_result.is_err();
        if let Err(outstanding) = &drain_result {
            if let Some(m) = &self.metrics {
                m.record_slot_revoke_outcome(crate::metrics::SlotDispatchOutcome::TimedOut);
            }
            tracing::warn!(
                outstanding = *outstanding,
                "slot revoke: per-resource drain timed out; proceeding to \
                 revoke hook (resource already tainted, no new leases)"
            );
        }

        // 2. Dispatch the revoke hook against the live runtime, bounded by
        //    the SAME per-resource budget. This is the only place the
        //    budget can cut the tail short: a wedged `on_credential_revoke`
        //    must not pin the fan-out row forever. A timed-out drain (above)
        //    has *already* consumed the metric outcome, so a hook that then
        //    also times out does not double-record.
        let hook_outcome =
            tokio::time::timeout(drain_timeout, managed.dispatch_on_revoke_erased(&slot)).await;
        tracing::Span::current().record("duration_ms", tainted_at.elapsed().as_millis() as u64);

        match hook_outcome {
            Ok(Ok(())) => {
                // Only record Success when the drain did not already record
                // the terminal TimedOut outcome for this dispatch.
                if !drain_timed_out && let Some(m) = &self.metrics {
                    m.record_slot_revoke_outcome(crate::metrics::SlotDispatchOutcome::Success);
                }
                self.emit(ResourceEvent::SlotRevoked {
                    key: key.clone(),
                    slot: slot.clone(),
                });
                tracing::debug!("slot revoke hook completed");
                RevokeTail::Done
            },
            Ok(Err(e)) => {
                if !drain_timed_out && let Some(m) = &self.metrics {
                    m.record_slot_revoke_outcome(crate::metrics::SlotDispatchOutcome::Failed);
                }
                self.emit(ResourceEvent::SlotRevokeFailed {
                    key,
                    slot,
                    error: e.to_string(),
                });
                tracing::warn!(error = %e, "slot revoke hook failed");
                RevokeTail::HookFailed(e)
            },
            Err(_elapsed) => {
                // The hook itself wedged. The row stays tainted (phase 1).
                // Record `TimedOut` unless the drain already did (one
                // dispatch = exactly one outcome).
                if !drain_timed_out && let Some(m) = &self.metrics {
                    m.record_slot_revoke_outcome(crate::metrics::SlotDispatchOutcome::TimedOut);
                }
                self.emit(ResourceEvent::SlotRevokeFailed {
                    key,
                    slot,
                    error: "revoke hook timed out".to_owned(),
                });
                tracing::warn!(
                    timeout_ms = drain_timeout.as_millis() as u64,
                    "slot revoke hook timed out (row stays tainted, no new leases)"
                );
                RevokeTail::HookTimedOut
            },
        }
    }

    /// Notifies a registered resource that one of its `#[credential]` slots
    /// was revoked — **thin two-phase convenience** for non-fan-out callers
    /// and tests.
    ///
    /// Equivalent to [`taint_slot`](Self::taint_slot) immediately followed by
    /// [`drain_and_revoke`](Self::drain_and_revoke). The engine per-slot
    /// rotation fan-out deliberately does **not** call this: it must run the
    /// synchronous taint phase *outside* its `tokio::time::timeout` and wrap
    /// only the awaited drain/hook tail, so a dropped timeout future can
    /// never skip the taint (per-resource revoke deferral). This convenience is for the
    /// no-timeout caller where the two phases run back-to-back on the same
    /// task.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource is registered for
    ///   `key` at `scope`.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) if more than one
    ///   resolved-credential row exists for `(key, scope)`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_revoke` hook maps into [`Error`].
    pub async fn revoke_slot(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
    ) -> Result<(), Error> {
        let tainted = self.taint_slot(key, scope, slot)?;
        self.drain_and_revoke(tainted, Self::DEFAULT_REVOKE_DRAIN_TIMEOUT)
            .await
            .into_result()
    }

    /// [`revoke_slot`](Self::revoke_slot) pinned to the **collision-free
    /// structural** resolved per-slot credential identity — the
    /// slot-identity-aware two-phase convenience.
    ///
    /// Equivalent to
    /// [`taint_slot_for_identity`](Self::taint_slot_for_identity) immediately
    /// followed by [`drain_and_revoke`](Self::drain_and_revoke); a
    /// multi-tenant `(key, scope)` taints/drains/revokes the *specific*
    /// resolved row instead of failing closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous). Like
    /// [`revoke_slot`](Self::revoke_slot) this is the back-compat
    /// back-to-back path; the engine fan-out drives the two phases separately
    /// ([`taint_slot_for_identity`](Self::taint_slot_for_identity) outside
    /// the timeout, then [`drain_and_revoke`](Self::drain_and_revoke)) per
    /// per-resource revoke deferral.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of `key` at `scope`
    ///   matches `slot_identity`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_revoke` hook maps into [`Error`].
    pub async fn revoke_slot_for_identity(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<(), Error> {
        let tainted = self.taint_slot_for_identity(key, scope, slot, slot_identity)?;
        self.drain_and_revoke(tainted, Self::DEFAULT_REVOKE_DRAIN_TIMEOUT)
            .await
            .into_result()
    }

    /// Type-erased `(key, scope)` → live `ManagedResource` resolution for
    /// the slot-rotation entry points.
    ///
    /// `refresh_slot` / `revoke_slot` take a `ResourceKey` (not a generic
    /// `R`), so they cannot use the typed `lookup::<R>`. This mirrors its
    /// shutdown-race guard (reject once `shutting_down` is observed) and
    /// resolves through the same registry the typed path uses, via the
    /// type-erased `AnyManagedResource` view.
    fn lookup_any_for_slot(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
    ) -> Result<Arc<dyn crate::registry::AnyManagedResource>, Error> {
        use crate::registry::LookupOutcome;
        self.shutdown_guard()?;
        match self.registry.get(key, scope) {
            LookupOutcome::Found(any) => Ok(any),
            LookupOutcome::NotFound => Err(Error::not_found(key)),
            // Fail closed: do not drive a rotation/revoke hook against an
            // arbitrarily-chosen tenant's row when several resolved-
            // credential rows share this `(key, scope)`. The engine's
            // per-slot fan-out targets the specific resolved row.
            LookupOutcome::Ambiguous { rows } => Err(Error::ambiguous(format!(
                "{key}: {rows} resolved-credential registrations exist at this scope; \
                 slot rotation/revoke must target a resolved row, not an ambiguous \
                 (key, scope)"
            ))
            .with_resource_key(key.clone())),
        }
    }

    /// Acquires a [`crate::guard::ResourceGuard`] through the registry row's
    /// erased dispatch hook, keyed by the **collision-free structural**
    /// resolved-credential identity (key + scope + slot identity).
    ///
    /// This is the object-safe engine/action-accessor acquire entry used
    /// when the concrete resource type `R` is not known at compile time: the
    /// accessor holds the structural
    /// [`SlotIdentity`](crate::dedup::SlotIdentity) recorded for the key at
    /// activation and passes it here, so the single scope walk resolves the
    /// *exact* resolved row (no digest aliasing). The resolved row is
    /// downcast by the hook with no second registry walk.
    ///
    /// # Errors
    ///
    /// Same as the typed `acquire_*_for_identity` family: not found,
    /// ambiguous (when `slot_identity` does not match a row), shutdown,
    /// taint, topology, and acquire-time failures.
    pub async fn acquire_erased_for(
        manager: Arc<Self>,
        key: &ResourceKey,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<Box<dyn std::any::Any + Send + Sync>, Error> {
        use crate::registry::AcquireLookupOutcome;

        manager.shutdown_guard()?;
        tracing::debug!(
            target: "nebula.resource",
            %key,
            ?slot_identity,
            "acquire_erased: resolving registry hook"
        );
        match manager
            .registry
            .get_acquire_for(key, ctx.scope(), slot_identity)
        {
            AcquireLookupOutcome::Found { acquire, managed } => {
                // `managed` is the row this single scope walk already
                // resolved; the hook downcasts it to the concrete
                // `ManagedResource<R>` instead of re-walking the registry
                // at the matched scope.
                acquire(manager, ctx.clone_for_acquire(), options.clone(), managed).await
            },
            AcquireLookupOutcome::NotFound => {
                tracing::debug!(target: "nebula.resource", %key, "acquire_erased: not found");
                Err(Error::not_found(key))
            },
            AcquireLookupOutcome::Ambiguous { rows } => {
                tracing::warn!(
                    target: "nebula.resource",
                    %key,
                    rows,
                    "acquire_erased: ambiguous scope/slot identity"
                );
                Err(Error::ambiguous(format!(
                    "{key}: {rows} resolved-credential registrations exist at this scope; \
                     acquire must target a resolved row via slot identity"
                ))
                .with_resource_key(key.clone()))
            },
        }
    }

    /// Returns whether a registry row exists for
    /// `(key, scope bag, slot_identity)`, keyed by the **collision-free
    /// structural** resolved-credential identity.
    ///
    /// This is the engine-facing entry: the engine records a structural
    /// [`SlotIdentity`](crate::dedup::SlotIdentity) at activation and asks
    /// the same structural identity here, so a row is visible *only* under
    /// its exact resolved binding set (no digest aliasing).
    #[must_use]
    pub fn has_registered_for_scope_identity(
        &self,
        key: &ResourceKey,
        scope: &nebula_core::Scope,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> bool {
        use crate::registry::AcquireLookupOutcome;
        if self.shutdown_guard().is_err() {
            return false;
        }
        matches!(
            self.registry.get_acquire_for(key, scope, slot_identity),
            AcquireLookupOutcome::Found { .. }
        )
    }

    /// Returns whether a registry row exists for
    /// `(key, scope level, slot_identity)`, keyed by the **collision-free
    /// structural** resolved-credential identity.
    ///
    /// Prefer
    /// [`has_registered_for_scope_identity`](Self::has_registered_for_scope_identity)
    /// when the full scope bag is available (execution + org/workspace).
    #[must_use]
    pub fn has_registered_for_identity(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> bool {
        let scope_bag = crate::context::minimal_scope_for_level(scope);
        self.has_registered_for_scope_identity(key, &scope_bag, slot_identity)
    }

    /// [`lookup_any_for_slot`](Self::lookup_any_for_slot) pinned to a
    /// resolved per-slot credential identity via
    /// [`Registry::get_for`](crate::registry::Registry::get_for).
    ///
    /// [`get_for`](crate::registry::Registry::get_for) returns the
    /// 2-variant [`PinnedLookup`](crate::registry::PinnedLookup): a
    /// resolved slot identity pins exactly one `(scope, slot_identity)` row
    /// by construction, so there is **no `Ambiguous` case to map** — the
    /// "registry invariant breach" arm the old `u64` digest path had to
    /// fabricate a fail-closed deny for is now type-unrepresentable.
    fn lookup_any_for_slot_identity_structural(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<Arc<dyn crate::registry::AnyManagedResource>, Error> {
        use crate::registry::PinnedLookup;
        self.shutdown_guard()?;
        match self.registry.get_for(key, scope, slot_identity) {
            PinnedLookup::Found(any) => Ok(any),
            PinnedLookup::NotFound => Err(Error::not_found(key)),
        }
    }

    /// Acquires a handle to a pooled resource.
    ///
    /// Performs typed lookup, then dispatches to the pool runtime's acquire.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   pool topology.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) — a
    ///   permanent (non-retryable) caller-conflict deny — if more than one
    ///   resolved-credential registration exists for `(R, scope)`
    ///   (multi-tenant). Acquire through the slot-identity-pinned
    ///   [`acquire_pooled_for_identity`](Self::acquire_pooled_for_identity)
    ///   when the resolved slot identity is known; this identity-agnostic
    ///   path stays fail-closed for the no-identity caller.
    /// - Propagates pool-specific acquire errors.
    pub async fn acquire_pooled<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        self.pooled_pipeline(managed, ctx, options).await
    }

    /// [`acquire_pooled`](Self::acquire_pooled) pinned to the
    /// **collision-free structural** resolved per-slot credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches, so a caller
    /// that resolved tenant A's credential reaches tenant A's runtime and
    /// never tenant B's. This is the unambiguous acquire path the engine
    /// resolution layer uses once it has resolved a node's slot bindings;
    /// it is also how callers reach a resource registered with a non-default
    /// [`RegisterOptions::with_slot_bindings`]. Equality is exact (no
    /// digest), so a forced digest collision cannot merge two tenants here.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   pool topology.
    /// - Propagates pool-specific acquire errors.
    pub async fn acquire_pooled_for_identity<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup_for_acquire_with_identity::<R>(ctx, slot_identity)?;
        self.pooled_pipeline(managed, ctx, options).await
    }

    /// [`acquire_pooled_for_identity`](Self::acquire_pooled_for_identity) for
    /// a row already resolved by the erased-acquire scope walk (downcast, no
    /// re-walk).
    pub(crate) async fn acquire_pooled_at_scope<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        resolved: Arc<dyn crate::registry::AnyManagedResource>,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.downcast_resolved_row::<R>(resolved)?;
        self.pooled_pipeline(managed, ctx, options).await
    }

    /// Pool topology dispatch into the shared [`run_acquire`](Self::run_acquire)
    /// pipeline. Holds only the one-arm `TopologyRuntime::Pool` match (the
    /// irreducible per-topology surface: the topology traits are siblings,
    /// not a hierarchy, so the shared generic pipeline cannot prove the
    /// variant statically). `config`/`generation` are recomputed inside the
    /// dispatch closure so they are re-read on every resilience retry.
    async fn pooled_pipeline<R>(
        &self,
        managed: Arc<ManagedResource<R>>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        self.run_acquire(Arc::clone(&managed), || {
            let generation = managed.generation();
            let config = managed.config();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Pool(rt) => {
                        rt.acquire(
                            &managed.resource,
                            &config,
                            ctx,
                            &managed.release_queue,
                            generation,
                            options,
                            self.metrics.clone(),
                        )
                        .await
                    },
                    other => Err(Self::unexpected_topology::<R>(other)),
                }
            }
        })
        .await
    }

    /// The single typed error every topology dispatch returns when the
    /// resolved row's [`TopologyRuntime`] variant does not match the
    /// statically-bound acquire path.
    ///
    /// Registration binds the row's topology to its trait (`R: Pooled`
    /// registers `TopologyRuntime::Pool`, etc.), so a mismatch here is a
    /// registration/lookup invariant breach, not a caller error — but the
    /// per-topology dispatch closures are bound to *one* sibling topology
    /// trait each (the traits are siblings, not a hierarchy), so a single
    /// generic pipeline cannot statically prove the variant. This collapses
    /// the five byte-identical `"{key}: expected X topology, registered as
    /// {tag}"` arms into one shared classifier instead of duplicating the
    /// `format!` once per topology dispatcher.
    fn unexpected_topology<R: Resource>(topology: &TopologyRuntime<R>) -> Error {
        Error::permanent(format!(
            "{}: resolved row topology {} does not match the acquired topology",
            R::key(),
            topology.tag()
        ))
    }

    /// Single generic acquire pipeline (resilience + gate + drain
    /// bookkeeping) over an already-resolved [`ManagedResource`], replacing
    /// the five byte-identical per-topology acquire wrappers. The only thing
    /// that differed between them was the one-arm topology dispatch, which
    /// each caller now supplies as `dispatch` (recomputed per resilience
    /// retry, exactly as the inline closures did). Every public `acquire_*` /
    /// `acquire_*_for` / `acquire_*_at_scope` entry point differs only in
    /// how it resolves the row (identity-agnostic vs. slot-identity-pinned
    /// vs. scope-pinned) and which topology runtime its closure calls; the
    /// pipeline — including the `InFlightCounter` → post-taint re-check
    /// ordering this method owns — is identical.
    async fn run_acquire<R, F, Fut>(
        &self,
        managed: Arc<ManagedResource<R>>,
        dispatch: F,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: Resource,
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<crate::guard::ResourceGuard<R>, Error>> + Send,
    {
        let started = Instant::now();
        // Pre-count this acquire on both the manager-wide and per-resource
        // in-flight trackers, from the moment `lookup()` succeeds. RAII
        // decrements + notifies on every failure / cancel / panic path; on
        // success the slot is handed off to the resulting `ResourceGuard` and
        // held continuously until the guard drops. The `AcqRel` increment here
        // is strictly before the post-taint re-check below. Two-phase-revoke
        // invariant: see the `manager` module documentation.
        let in_flight =
            InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
        // Post-count taint re-check — closes the revoke-vs-acquire TOCTOU now
        // that this acquire is reflected in the per-resource counter
        // `revoke_slot` drains. Same `Revoked` (→ `Unavailable`)
        // classification as the taint gate. Rationale: see the `manager`
        // module documentation.
        Self::reject_if_tainted_post_count::<R>(&managed)?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;
        let resilience = managed.resilience.clone();

        let result = execute_with_resilience(&resilience, dispatch).await;

        // Settle the gate ticket based on the acquire result. #322: this
        // makes the ticket ownership end-to-end — on success we `resolve`,
        // on retryable error we `fail_transient`, on permanent error we
        // `fail_permanent`. The `Drop` impl of `RecoveryTicket` covers
        // cancellation/panic paths.
        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        match result {
            Ok(h) => Ok(h.with_drain_tracker(in_flight.release_to_guard())),
            Err(e) => Err(e),
        }
    }

    /// Acquires a handle to a resident resource.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   resident topology.
    /// - Propagates resident-specific acquire errors.
    pub async fn acquire_resident<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        self.resident_pipeline(managed, ctx, options).await
    }

    /// [`acquire_resident`](Self::acquire_resident) pinned to the
    /// **collision-free structural** resolved per-slot credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches, so a caller
    /// that resolved tenant A's credential reaches tenant A's runtime and
    /// never tenant B's. This is the unambiguous acquire path the engine
    /// resolution layer uses once it has resolved a node's slot bindings;
    /// it is also how callers reach a resource registered with a non-default
    /// [`RegisterOptions::with_slot_bindings`]. Two registrations whose
    /// resolved `(slot, credential)` bindings differ are distinct rows with
    /// distinct runtimes; equality is exact (no digest), so a forced digest
    /// collision cannot merge two tenants here.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   resident topology.
    /// - Propagates resident-specific acquire errors.
    pub async fn acquire_resident_for_identity<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        let managed = self.lookup_for_acquire_with_identity::<R>(ctx, slot_identity)?;
        self.resident_pipeline(managed, ctx, options).await
    }

    /// [`acquire_resident_for_identity`](Self::acquire_resident_for_identity)
    /// for a row already resolved by the erased-acquire scope walk
    /// (downcast, no re-walk).
    pub(crate) async fn acquire_resident_at_scope<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        resolved: Arc<dyn crate::registry::AnyManagedResource>,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        let managed = self.downcast_resolved_row::<R>(resolved)?;
        self.resident_pipeline(managed, ctx, options).await
    }

    /// Resident topology dispatch into the shared
    /// [`run_acquire`](Self::run_acquire) pipeline. Holds only the one-arm
    /// `TopologyRuntime::Resident` match (resident `acquire` takes neither
    /// `release_queue`/`generation` nor `metrics`). `config` is recomputed
    /// inside the dispatch closure so it is re-read on every resilience
    /// retry.
    async fn resident_pipeline<R>(
        &self,
        managed: Arc<ManagedResource<R>>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        self.run_acquire(Arc::clone(&managed), || {
            let config = managed.config();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Resident(rt) => {
                        rt.acquire(&managed.resource, &config, ctx, options).await
                    },
                    other => Err(Self::unexpected_topology::<R>(other)),
                }
            }
        })
        .await
    }

    /// Acquires a handle to a [`Bounded`](crate::topology::bounded::Bounded)
    /// resource.
    ///
    /// The release shape is the resource's [`Cap`](crate::topology::bounded::Bounded::Cap)
    /// typestate — `Unbounded` → owned handle (no release), `Capped<N>` /
    /// `Exclusive` → guarded handle whose drop runs the observed
    /// `release_one` (R17). Identity-agnostic: a multi-tenant `(R, scope)`
    /// fails closed with
    /// [`Ambiguous`](crate::error::ErrorKind::Ambiguous); use
    /// [`acquire_bounded_for_identity`](Self::acquire_bounded_for_identity)
    /// with the resolved structural identity.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   bounded topology.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) if more
    ///   than one resolved-credential registration exists for `(R, scope)`.
    /// - Propagates the cap's acquire errors (permit timeout / closed).
    pub async fn acquire_bounded<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::bounded::BoundedRelease + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        self.bounded_pipeline(managed, ctx, options).await
    }

    /// [`acquire_bounded`](Self::acquire_bounded) keyed by the
    /// **collision-free structural** resolved-credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches exactly (no
    /// digest aliasing), so a caller that resolved tenant A's credential
    /// reaches tenant A's runtime and never tenant B's.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   bounded topology.
    /// - Propagates the cap's acquire errors.
    pub async fn acquire_bounded_for_identity<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::bounded::BoundedRelease + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup_for_acquire_with_identity::<R>(ctx, slot_identity)?;
        self.bounded_pipeline(managed, ctx, options).await
    }

    /// [`acquire_bounded`](Self::acquire_bounded) for a row already resolved
    /// by the erased-acquire scope walk (downcast, no re-walk).
    pub(crate) async fn acquire_bounded_at_scope<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        resolved: Arc<dyn crate::registry::AnyManagedResource>,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::bounded::BoundedRelease + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.downcast_resolved_row::<R>(resolved)?;
        self.bounded_pipeline(managed, ctx, options).await
    }

    /// Bounded topology dispatch into the shared
    /// [`run_acquire`](Self::run_acquire) pipeline. One-arm
    /// `TopologyRuntime::Bounded` match (same shape as transport:
    /// `release_queue`/`generation`/`metrics`, no `config`). `generation`
    /// is recomputed inside the dispatch closure so it is re-read on every
    /// resilience retry.
    async fn bounded_pipeline<R>(
        &self,
        managed: Arc<ManagedResource<R>>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::bounded::BoundedRelease + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        self.run_acquire(Arc::clone(&managed), || {
            let generation = managed.generation();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Bounded(rt) => {
                        rt.acquire(
                            &managed.resource,
                            ctx,
                            &managed.release_queue,
                            generation,
                            options,
                            self.metrics.clone(),
                        )
                        .await
                    },
                    other => Err(Self::unexpected_topology::<R>(other)),
                }
            }
        })
        .await
    }

    /// Returns a snapshot of current pool utilization for a registered Pool resource.
    ///
    /// Returns `None` if the resource is not registered or does not use Pool topology.
    pub async fn pool_stats<R>(&self, scope: &ScopeLevel) -> Option<crate::runtime::pool::PoolStats>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup::<R>(scope).ok()?;
        match &managed.topology {
            TopologyRuntime::Pool(rt) => Some(rt.stats().await),
            _ => None,
        }
    }

    /// Pre-warms a registered Pool resource.
    ///
    /// Per slot model, the resource's `#[credential]` slot fields are
    /// already populated on the resource value — `Pool::warmup` calls
    /// `R::create(config, ctx)` directly, no scheme parameter required.
    ///
    /// This fills the idle queue before production traffic hits, eliminating
    /// cold-start latency on the first batch of requests. Warmup follows the
    /// [`WarmupStrategy`](crate::topology::pooled::config::WarmupStrategy) set
    /// in the pool's configuration.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   pool topology.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) — a
    ///   permanent (non-retryable) caller-conflict deny — if more than one
    ///   resolved-credential registration exists for `(R, scope)`
    ///   (multi-tenant). Warmup is identity-agnostic and stays fail-closed;
    ///   a multi-tenant pool is warmed per resolved row through the
    ///   slot-identity-pinned acquire path
    ///   ([`acquire_pooled_for_identity`](Self::acquire_pooled_for_identity)).
    pub async fn warmup_pool<R>(&self, ctx: &ResourceContext) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        let config = managed.config();
        match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                // `warmup` runs `R::create` against the resolved credential
                // to materialize fresh pool instances — it is acquire-like
                // and must observe the SAME revoke-vs-acquire TOCTOU close
                // the `run_*_acquire` pipelines use (#679 / slot + isolation model).
                // The `lookup_for_acquire` taint gate above ran *before*
                // this in-flight increment, leaving a window where a
                // concurrent `revoke_slot` could taint after the gate yet
                // before warmup creates entries. Pre-count this work in the
                // resource's own in-flight counter (the exact counter
                // `revoke_slot` drains), then re-check the taint: either we
                // observe the taint here and reject, or our increment is
                // visible to the revoke's drain — so no fresh pool entry is
                // ever created on a just-revoked credential. The counter is
                // held for the whole `warmup` await (RAII drop on every
                // exit path).
                let _in_flight =
                    InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
                Self::reject_if_tainted_post_count::<R>(&managed)?;
                let count = rt.warmup(&managed.resource, &config, ctx).await;
                Ok(count)
            },
            _ => Err(Error::permanent(format!(
                "{}: warmup_pool requires Pool topology, registered as {}",
                R::key(),
                managed.topology.tag()
            ))),
        }
    }

    /// Hot-reloads the configuration for a registered resource.
    ///
    /// Validates the new config, swaps it into the [`ArcSwap`](arc_swap::ArcSwap),
    /// increments the generation counter, and — for pool topologies — updates the
    /// fingerprint so idle instances with stale configs are evicted on next
    /// acquire or release.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered for the given scope.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if config validation fails.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shut down.
    pub fn reload_config<R: Resource>(
        &self,
        new_config: R::Config,
        scope: &ScopeLevel,
    ) -> Result<ReloadOutcome, Error> {
        use crate::resource::ResourceConfig as _;

        new_config.validate()?;

        let managed = self.lookup::<R>(scope)?;

        // Fingerprint comparison — bail early if nothing changed.
        let new_fp = new_config.fingerprint();
        let old_fp = managed.config.load().fingerprint();
        if new_fp == old_fp {
            return Ok(ReloadOutcome::NoChange);
        }

        // #387: visible `Reloading` phase for operators polling health
        // mid-swap.
        managed.set_phase(crate::state::ResourcePhase::Reloading);

        // Atomically swap the config.
        managed.config.store(Arc::new(new_config));

        // Update pool fingerprint so stale idle instances are evicted.
        if let TopologyRuntime::Pool(ref pool_rt) = managed.topology {
            pool_rt.set_fingerprint(new_fp);
        }

        // Bump generation — readers snapshot this to detect changes.
        let prev_gen = managed
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::Release);

        // #387: return to `Ready` after publishing the new atomic
        // generation so pollers see the phase transition alongside the
        // config change. `health_check` reads the atomic directly, but
        // `ResourceStatus.generation` is also refreshed by `set_phase`
        // so `status()` snapshots stay self-consistent.
        managed.set_phase(crate::state::ResourcePhase::Ready);

        let _ = self
            .event_tx
            .send(ResourceEvent::ConfigReloaded { key: R::key() });

        // Reload outcome. The former `Service` topology returned
        // `PendingDrain { old_generation }` here; post-fold a former-Service
        // row is `TopologyRuntime::Bounded`, and `reload_config` swaps the
        // config `ArcSwap` without rebuilding the caller-supplied live
        // `Arc<R::Runtime>` for *any* topology (it never did — only the Pool
        // fingerprint is updated, above). So the honest outcome is
        // `SwappedImmediately` for every variant: the config is swapped, the
        // live runtime is not rebuilt. This is a no-op-preserving relabel of
        // the old `PendingDrain` arm (which also never drained/rebuilt) — the
        // genuine "drain + rebuild the live runtime on reload" behavior is
        // the separately-tracked deferred `reload_config` redesign, not a
        // regression introduced by the topology fold.
        let _ = prev_gen;
        let outcome = ReloadOutcome::SwappedImmediately;

        tracing::info!(key = %R::key(), ?outcome, "resource config reloaded");
        Ok(outcome)
    }

    /// Removes a resource from the registry by key.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if
    /// the key is not registered.
    pub fn remove(&self, key: &ResourceKey) -> Result<(), Error> {
        if !self.registry.remove(key) {
            return Err(Error::not_found(key));
        }

        if let Some(m) = &self.metrics {
            m.record_destroy();
        }
        let _ = self
            .event_tx
            .send(ResourceEvent::Removed { key: key.clone() });
        tracing::debug!(%key, "resource removed");
        Ok(())
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
    pub fn health_check<R: Resource>(
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
    /// type-erased `Arc<dyn AnyManagedResource>`.
    ///
    /// Useful for diagnostics and admin APIs that don't need typed access.
    /// Returns `None` both when nothing is registered and when several
    /// resolved-credential rows share `(key, scope)` (ambiguous) — a
    /// diagnostic peek must not arbitrarily pick one tenant's row.
    pub fn get_any(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
    ) -> Option<Arc<dyn crate::registry::AnyManagedResource>> {
        match self.registry.get(key, scope) {
            crate::registry::LookupOutcome::Found(any) => Some(any),
            crate::registry::LookupOutcome::NotFound
            | crate::registry::LookupOutcome::Ambiguous { .. } => None,
        }
    }

    /// Records acquire success/failure in aggregate metrics and emits
    /// the corresponding [`ResourceEvent`].
    fn record_acquire_result<R: Resource>(
        &self,
        result: &Result<crate::guard::ResourceGuard<R>, Error>,
        started: Instant,
    ) {
        match result {
            Ok(_) => {
                if let Some(m) = &self.metrics {
                    m.record_acquire();
                }
                let _ = self.event_tx.send(ResourceEvent::AcquireSuccess {
                    key: R::key(),
                    duration: started.elapsed(),
                });
            },
            Err(e) => {
                if let Some(m) = &self.metrics {
                    m.record_acquire_error();
                }
                let _ = self.event_tx.send(ResourceEvent::AcquireFailed {
                    key: R::key(),
                    error: e.to_string(),
                });
            },
        }
    }

    /// Broadcasts a [`ResourceEvent`] to current subscribers.
    ///
    /// `broadcast::Sender::send` only returns `Err` when there are **zero**
    /// receivers — an expected, non-error condition (events are a passive
    /// observability stream, not a delivery guarantee). This helper names
    /// that contract in one place so the absence of a subscriber is
    /// explicitly a deliberate no-op rather than a silently discarded
    /// `Result` at the emit site.
    fn emit(&self, event: ResourceEvent) {
        match self.event_tx.send(event) {
            Ok(_subscribers) => {},
            // No subscribers attached — the event stream is best-effort
            // observability, so this is the documented normal case, not a
            // failure to propagate.
            Err(broadcast::error::SendError(_dropped)) => {},
        }
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
