//! Central resource manager — registration, acquire dispatch, and shutdown.
//!
//! [`Manager`] is the single entry point for the resource subsystem. It owns
//! the registry, recovery-group registry, and a [`CancellationToken`] for
//! coordinated shutdown.
//!
//! Phase 4 / ADR-0044: the public API drops the `R::Credential` projection
//! that ADR-0036 used to thread `scheme: &<R::Credential as Credential>::Scheme`
//! through every acquire/warmup/register call. Resources now declare
//! credential dependencies as typed slot fields on the struct (via
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
//! # Submodule layout (Tech Spec §5.4)
//!
//! - `options` — `ManagerConfig`, `RegisterOptions`, `ShutdownConfig`, `DrainTimeoutPolicy`
//! - `gate` — `GateAdmission` + `admit_through_gate` + `settle_gate_admission`
//! - `execute` — resilience pipeline + register-time pool config validation
//! - `shutdown` — `graceful_shutdown` + drain helpers + `set_phase_all*`

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering},
    },
    time::Instant,
};

use nebula_core::{LayerLifecycle, ResourceKey, ScopeLevel};
use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;

use crate::{
    context::ResourceContext,
    error::Error,
    events::ResourceEvent,
    integration::AcquireResilience,
    metrics::{ResourceOpsMetrics, ResourceOpsSnapshot},
    options::AcquireOptions,
    recovery::{
        gate::{GateState, RecoveryGate},
        group::RecoveryGroupRegistry,
    },
    registry::Registry,
    release_queue::{ReleaseQueue, ReleaseQueueHandle},
    reload::ReloadOutcome,
    resource::Resource,
    runtime::{TopologyRuntime, managed::ManagedResource},
};

mod execute;
mod gate;
pub(crate) mod options;
pub(crate) mod shutdown;

use execute::{execute_with_resilience, validate_pool_config};
use gate::{admit_through_gate, settle_gate_admission};
pub use options::{DrainTimeoutPolicy, ManagerConfig, RegisterOptions, ShutdownConfig};
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
/// [`Manager::taint_slot_for`](Manager::taint_slot_for) — phase 1 of the
/// two-phase revoke port (ADR-0067 §Deferred).
///
/// Holding one is proof the taint already ran to completion: new acquires on
/// this row's credential are already rejected. It is consumed by
/// [`Manager::drain_and_revoke`](Manager::drain_and_revoke) to run the
/// cancellation-safe drain + revoke-hook tail.
///
/// **Why two phases.** The engine rotation fan-out wraps the awaited
/// drain/hook tail in `tokio::time::timeout`. A Rust `async fn` body is lazy:
/// if the timeout future is dropped before its first poll, the body never
/// runs. Splitting the taint into this synchronous phase guarantees the taint
/// is applied *before and outside* any per-resource timeout — a dropped
/// `drain_and_revoke` future therefore leaves the row tainted and consistent
/// (the credential is never silently un-revoked), it only forgoes the
/// best-effort drain/hook tail.
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
/// `tokio::time::timeout` wrapping the whole tail — that wrapper used to
/// be able to drop the future *before the hook ran* when the drain was
/// slow, contradicting the "hook still runs after a timed-out drain"
/// contract (ADR-0067 §Deferred). So the three terminal states are
/// reported here rather than inferred from a dropped outer future:
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
    /// [`Manager::revoke_slot_for`]) that run taint+tail back-to-back and
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
    pub(super) recovery_groups: RecoveryGroupRegistry,
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
            recovery_groups: RecoveryGroupRegistry::new(),
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

    /// Registers a resource with its config, scope, topology, and optional
    /// resilience / recovery gate configuration.
    ///
    /// Per ADR-0044 the `resource: R` value passed in is expected to have
    /// **all `#[credential]` slot fields already resolved and populated**.
    /// `Manager::register` does not itself resolve credential bindings —
    /// that is the responsibility of the caller (typically the engine
    /// dispatch layer that assembles `R` via the `FromConfig` trait emitted
    /// by `#[derive(Resource)]`).
    ///
    /// The resource is wrapped in a [`ManagedResource`] and stored in the
    /// registry under `R::key()`. If a resource with the same key and scope
    /// is already registered, it is silently replaced.
    ///
    /// The manager's internal [`ReleaseQueue`] is automatically shared with
    /// the managed resource — callers never need to create or manage it.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails on the provided config.
    pub fn register<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        scope: ScopeLevel,
        topology: TopologyRuntime<R>,
        resilience: Option<AcquireResilience>,
        recovery_gate: Option<Arc<RecoveryGate>>,
    ) -> Result<(), Error> {
        self.register_with_identity(
            resource,
            config,
            scope,
            crate::dedup::SLOT_IDENTITY_UNBOUND,
            topology,
            resilience,
            recovery_gate,
        )
    }

    /// [`register`](Self::register) plus a resolved per-slot credential
    /// identity that pins the registry row.
    ///
    /// This is the structural anti-bleed seam: two registrations of the
    /// same resource type at the same `scope` whose `slot_identity` differs
    /// occupy **distinct** registry rows with **distinct** topology
    /// runtimes, so one tenant's runtime can never serve another tenant's
    /// resolved credential. Passing
    /// [`SLOT_IDENTITY_UNBOUND`](crate::dedup::SLOT_IDENTITY_UNBOUND)
    /// (what plain [`register`](Self::register) and the `register_*`
    /// shorthands do) preserves the historical single-row-per-`(key,
    /// scope)` dedup contract (one `Resource::create` for N acquires of the
    /// same credential).
    ///
    /// Compute `slot_identity` from the resolved slot bindings via
    /// [`slot_identity`](crate::dedup::slot_identity).
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails on the provided config.
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors register<R>'s arity plus the slot-identity pin; collapsing into a struct would force the register_*_with shorthands and the engine resolution path through a builder for one extra u64"
    )]
    pub fn register_with_identity<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        scope: ScopeLevel,
        slot_identity: u64,
        topology: TopologyRuntime<R>,
        resilience: Option<AcquireResilience>,
        recovery_gate: Option<Arc<RecoveryGate>>,
    ) -> Result<(), Error> {
        use crate::resource::ResourceConfig as _;
        config.validate()?;

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
        self.registry
            .register(key.clone(), type_id, scope, slot_identity, managed.clone());

        // #387: everything below `register()` is a single funnel — the
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

    /// Registers a pooled resource with sensible defaults.
    ///
    /// Shorthand for [`register`](Self::register) with
    /// `scope = Global`, no resilience, no recovery gate.
    ///
    /// The pool fingerprint is initialized from the provided config.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails.
    pub fn register_pooled<R>(
        &self,
        resource: R,
        config: R::Config,
        pool_config: crate::topology::pooled::config::Config,
    ) -> Result<(), Error>
    where
        R: Resource,
    {
        use crate::resource::ResourceConfig as _;

        validate_pool_config(&pool_config)?;

        let fingerprint = config.fingerprint();
        self.register(
            resource,
            config,
            ScopeLevel::Global,
            TopologyRuntime::Pool(crate::runtime::pool::PoolRuntime::<R>::new(
                pool_config,
                fingerprint,
            )),
            None,
            None,
        )
    }

    /// Registers a resident resource with sensible defaults.
    ///
    /// Shorthand for [`register`](Self::register) with
    /// `scope = Global`, no resilience, no recovery gate.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails.
    pub fn register_resident<R>(
        &self,
        resource: R,
        config: R::Config,
        resident_config: crate::topology::resident::config::Config,
    ) -> Result<(), Error>
    where
        R: Resource,
    {
        self.register(
            resource,
            config,
            ScopeLevel::Global,
            TopologyRuntime::Resident(crate::runtime::resident::ResidentRuntime::<R>::new(
                resident_config,
            )),
            None,
            None,
        )
    }

    /// Registers a service resource with sensible defaults.
    ///
    /// Shorthand for [`register`](Self::register) with
    /// `scope = Global`, no resilience, no recovery gate.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails.
    pub fn register_service<R>(
        &self,
        resource: R,
        config: R::Config,
        runtime: R::Runtime,
        service_config: crate::topology::service::config::Config,
    ) -> Result<(), Error>
    where
        R: Resource,
    {
        self.register(
            resource,
            config,
            ScopeLevel::Global,
            TopologyRuntime::Service(crate::runtime::service::ServiceRuntime::<R>::new(
                runtime,
                service_config,
            )),
            None,
            None,
        )
    }

    /// Registers an exclusive resource with sensible defaults.
    ///
    /// Shorthand for [`register`](Self::register) with
    /// `scope = Global`, no resilience, no recovery gate.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails.
    pub fn register_exclusive<R>(
        &self,
        resource: R,
        config: R::Config,
        runtime: R::Runtime,
        exclusive_config: crate::topology::exclusive::config::Config,
    ) -> Result<(), Error>
    where
        R: Resource,
    {
        self.register(
            resource,
            config,
            ScopeLevel::Global,
            TopologyRuntime::Exclusive(crate::runtime::exclusive::ExclusiveRuntime::<R>::new(
                runtime,
                exclusive_config,
            )),
            None,
            None,
        )
    }

    /// Registers a transport resource with sensible defaults.
    ///
    /// Shorthand for [`register`](Self::register) with
    /// `scope = Global`, no resilience, no recovery gate.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails.
    pub fn register_transport<R>(
        &self,
        resource: R,
        config: R::Config,
        runtime: R::Runtime,
        transport_config: crate::topology::transport::config::Config,
    ) -> Result<(), Error>
    where
        R: Resource,
    {
        self.register(
            resource,
            config,
            ScopeLevel::Global,
            TopologyRuntime::Transport(crate::runtime::transport::TransportRuntime::<R>::new(
                runtime,
                transport_config,
            )),
            None,
            None,
        )
    }

    /// Registers a pooled resource with extended options.
    ///
    /// Like [`register_pooled`](Self::register_pooled) but accepts
    /// [`RegisterOptions`] for scope, resilience, recovery gate.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails.
    pub fn register_pooled_with<R>(
        &self,
        resource: R,
        config: R::Config,
        pool_config: crate::topology::pooled::config::Config,
        options: RegisterOptions,
    ) -> Result<(), Error>
    where
        R: Resource,
    {
        use crate::resource::ResourceConfig as _;

        validate_pool_config(&pool_config)?;

        let fingerprint = config.fingerprint();
        self.register_with_identity(
            resource,
            config,
            options.scope,
            options.slot_identity,
            TopologyRuntime::Pool(crate::runtime::pool::PoolRuntime::<R>::new(
                pool_config,
                fingerprint,
            )),
            options.resilience,
            options.recovery_gate,
        )
    }

    /// Registers a resident resource with extended options.
    ///
    /// Like [`register_resident`](Self::register_resident) but accepts
    /// [`RegisterOptions`] for scope, resilience, recovery gate.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails.
    pub fn register_resident_with<R>(
        &self,
        resource: R,
        config: R::Config,
        resident_config: crate::topology::resident::config::Config,
        options: RegisterOptions,
    ) -> Result<(), Error>
    where
        R: Resource,
    {
        self.register_with_identity(
            resource,
            config,
            options.scope,
            options.slot_identity,
            TopologyRuntime::Resident(crate::runtime::resident::ResidentRuntime::<R>::new(
                resident_config,
            )),
            options.resilience,
            options.recovery_gate,
        )
    }

    /// Registers a service resource with extended options.
    ///
    /// Like [`register_service`](Self::register_service) but accepts
    /// [`RegisterOptions`] for scope, resilience, recovery gate.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails.
    pub fn register_service_with<R>(
        &self,
        resource: R,
        config: R::Config,
        runtime: R::Runtime,
        service_config: crate::topology::service::config::Config,
        options: RegisterOptions,
    ) -> Result<(), Error>
    where
        R: Resource,
    {
        self.register_with_identity(
            resource,
            config,
            options.scope,
            options.slot_identity,
            TopologyRuntime::Service(crate::runtime::service::ServiceRuntime::<R>::new(
                runtime,
                service_config,
            )),
            options.resilience,
            options.recovery_gate,
        )
    }

    /// Registers a transport resource with extended options.
    ///
    /// Like [`register_transport`](Self::register_transport) but accepts
    /// [`RegisterOptions`] for scope, resilience, recovery gate.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails.
    pub fn register_transport_with<R>(
        &self,
        resource: R,
        config: R::Config,
        runtime: R::Runtime,
        transport_config: crate::topology::transport::config::Config,
        options: RegisterOptions,
    ) -> Result<(), Error>
    where
        R: Resource,
    {
        self.register_with_identity(
            resource,
            config,
            options.scope,
            options.slot_identity,
            TopologyRuntime::Transport(crate::runtime::transport::TransportRuntime::<R>::new(
                runtime,
                transport_config,
            )),
            options.resilience,
            options.recovery_gate,
        )
    }

    /// Registers an exclusive resource with extended options.
    ///
    /// Like [`register_exclusive`](Self::register_exclusive) but accepts
    /// [`RegisterOptions`] for scope, resilience, recovery gate.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails.
    pub fn register_exclusive_with<R>(
        &self,
        resource: R,
        config: R::Config,
        runtime: R::Runtime,
        exclusive_config: crate::topology::exclusive::config::Config,
        options: RegisterOptions,
    ) -> Result<(), Error>
    where
        R: Resource,
    {
        self.register_with_identity(
            resource,
            config,
            options.scope,
            options.slot_identity,
            TopologyRuntime::Exclusive(crate::runtime::exclusive::ExclusiveRuntime::<R>::new(
                runtime,
                exclusive_config,
            )),
            options.resilience,
            options.recovery_gate,
        )
    }

    /// Schema-validate an **already-resolved** config JSON tree against
    /// `<R::Config as HasSchema>::schema()` *without* registering anything.
    ///
    /// This is the pure validation core shared with
    /// [`register_from_value`](Self::register_from_value): it runs exactly
    /// the schema pass, the closed-set guard, and the `R::Config`
    /// deserialize step that the live path runs *after* template
    /// resolution — but performs **no** `{{ … }}` resolution, **no**
    /// `Manager` mutation, and constructs **no** `resource: R` /
    /// `TopologyRuntime<R>`. It is the seam a config-CRUD writer uses to
    /// reject a bad `ResourceEntry.config` *before* persistence, keeping
    /// config validation strictly separate from engine-activation live
    /// registration (INTEGRATION_MODEL §13.1 — live registration happens
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
    /// run here, so the live `register_from_value` path consumes this
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
    ///   (PRODUCT_CANON §3.5). The error names only the offending key,
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
        // (PRODUCT_CANON §3.5; ADR-0044 slot model; ADR-0030 redaction; ADR-0036
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
                 (PRODUCT_CANON §3.5)",
                unknown = unknown.as_str(),
                ty = std::any::type_name::<R::Config>(),
            )));
        }

        // Deserialize R::Config from the JSON to surface any residual
        // type-shape mismatch the structural schema pass did not, and
        // return the parsed value: the live `register_from_value` path
        // consumes this owned `R::Config` directly, so the JSON is
        // deserialized exactly once across validation + typed dispatch.
        serde_json::from_value::<R::Config>(config_json).map_err(|e| {
            Error::permanent(format!(
                "validate_config_value: failed to deserialize {ty} config from JSON: {e}",
                ty = std::any::type_name::<R::Config>()
            ))
        })
    }

    /// JSON-driven registration with `{{ ... }}` template resolution + schema validation (Phase 9
    /// of M6 / closes the tail deferred from Phase 4).
    ///
    /// The flow:
    ///
    /// 1. Recursively resolve every `{{ … }}` template inside `config_json` against `expr_engine` +
    ///    an evaluation context populated with the caller-supplied variables.
    /// 2. Deserialize the resolved JSON into `R::Config`.
    /// 3. Validate the deserialized config via [`<R::Config as
    ///    ResourceConfig>::validate`](crate::resource::ResourceConfig::validate) AND against
    ///    `<R::Config as HasSchema>::schema()` (a structural schema pass that catches
    ///    missing/invalid fields a `serde::Deserialize` impl would silently default).
    /// 4. Dispatch into the typed [`register`](Self::register) with the pre-built `resource: R`
    ///    (slots already filled by the caller), `topology`, `scope`, and optional
    ///    `resilience`/`recovery_gate`.
    ///
    /// `slot_bindings` carries the slot-name → credential id map per ADR-0042 hybrid binding.
    /// Credential resolution is the engine dispatch layer's responsibility; the manager itself is
    /// credential-agnostic post-ADR-0044 (see Phase 4 — `R::Credential` was deleted), so this
    /// argument is recorded for tracing only and asserted to match the slot fields the resource
    /// declared via [`DeclaresDependencies`](nebula_core::DeclaresDependencies). The caller
    /// (engine) is expected to have already used these bindings to resolve credentials into the
    /// `resource: R` it hands in.
    ///
    /// `nebula-resource → nebula-expression` is allowed under deny.toml's `[[bans]]`
    /// `nebula-resource` wrapper allowlist (Business → Core layer edge per ADR-0043 §9 / Phase 9,
    /// R-040 R8).
    ///
    /// # Errors
    ///
    /// - [`Error::permanent`] when expression resolution, JSON deserialization, or schema
    ///   validation fails.
    /// - [`Error::permanent`] when the config carries a top-level field the `R::Config` schema
    ///   does not declare (closed-set guard): `ResourceConfig` must carry no secrets, so an
    ///   inlined secret-shaped field is rejected here rather than silently ignored
    ///   (PRODUCT_CANON §3.5). The error names only the offending key, never its value.
    /// - [`Error::permanent`] when a `slot_bindings` key does not correspond to a declared
    ///   credential slot on `R`.
    /// - Any [`Error`](Error) returned by the underlying typed [`register`](Self::register).
    #[tracing::instrument(
        level = "debug",
        target = "nebula_resource::register_from_value",
        skip_all,
        fields(
            resource_key = %R::key(),
            slot_count = slot_bindings.len(),
        )
    )]
    #[allow(
        clippy::too_many_arguments,
        reason = "JSON-driven registration must thread (config_json, expr_engine, slot_bindings, resource, scope, topology, resilience, recovery_gate); collapsing into an options struct would force callers through a builder when the typed register<R> path next door already takes 6 args"
    )]
    pub async fn register_from_value<R>(
        &self,
        config_json: serde_json::Value,
        expr_engine: &nebula_expression::ExpressionEngine,
        slot_bindings: std::collections::HashMap<String, nebula_core::CredentialKey>,
        resource: R,
        scope: ScopeLevel,
        topology: TopologyRuntime<R>,
        resilience: Option<AcquireResilience>,
        recovery_gate: Option<Arc<RecoveryGate>>,
    ) -> Result<(), Error>
    where
        R: Resource + nebula_core::DeclaresDependencies,
        R::Config: serde::de::DeserializeOwned,
    {
        // 0. Validate that every binding matches a declared credential slot. Hard error on unknown
        //    slot — refuses to register a resource whose credential surface diverged from the one
        //    the workflow JSON specified, so misconfiguration surfaces at register time rather than
        //    as a confusing rotation no-op later.
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
                    "register_from_value: slot binding `{slot_name}` does not match any declared credential slot on `{}`",
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
        //    two paths cannot drift: the only difference is that the live
        //    path validates the *post-template-resolution* JSON (step 1
        //    above) whereas the config-CRUD seam validates the stored shape
        //    directly (no expression context at config-create time).
        let config: R::Config = Self::validate_config_value::<R>(resolved)?;

        // 4. Derive the structural slot identity from the *resolved* slot bindings. This is the
        //    per-slot resolved-credential identity available at the register/dedup point on the
        //    JSON path (the caller has already resolved these bindings into `resource: R`). Folding
        //    it into the registry row keeps two registrations that resolved *different* credentials
        //    on separate rows with separate runtimes — the structural barrier against cross-tenant
        //    runtime bleed (ADR-0036 isolation intent, ADR-0044 slot model). It is a hash over
        //    `(slot_key, credential_key)` pairs only — it carries no secret bytes.
        let slot_id = {
            let pairs: Vec<(String, String)> = slot_bindings
                .iter()
                .map(|(slot, cred)| (slot.clone(), cred.as_str().to_owned()))
                .collect();
            crate::dedup::slot_identity(pairs.iter().map(|(s, c)| (s.as_str(), c.as_str())))
        };

        // 5. Dispatch into the typed register. ResourceConfig::validate() runs inside register, so
        //    domain-level rules (e.g. PoolConfig sanity, host non-empty) are still enforced.
        tracing::debug!(
            target: "nebula_resource::register_from_value",
            slot_identity = slot_id,
            "all pre-register checks passed; dispatching into typed register"
        );
        self.register_with_identity(
            resource,
            config,
            scope,
            slot_id,
            topology,
            resilience,
            recovery_gate,
        )
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

    /// [`lookup`](Self::lookup) pinned to a resolved per-slot credential
    /// identity.
    ///
    /// Selects the registry row whose `slot_identity` matches, so a caller
    /// that resolved tenant A's credential can only ever reach tenant A's
    /// runtime. This is the read-side counterpart of
    /// [`register_with_identity`](Self::register_with_identity); use it
    /// whenever the resolved slot identity is known so the lookup is never
    /// ambiguous.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    pub fn lookup_for<R: Resource>(
        &self,
        scope: &ScopeLevel,
        slot_identity: u64,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        self.shutdown_guard()?;
        Self::resolve_typed::<R>(self.registry.get_typed_for::<R>(scope, slot_identity))
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

    /// [`lookup`](Self::lookup) plus the resource-level taint check.
    ///
    /// Every `acquire_*` path funnels through here so a single check
    /// rejects new leases once `revoke_slot` has tainted the resource —
    /// the same single-funnel discipline `lookup` uses for the
    /// `shutting_down` race. Diagnostic paths (`health_check`,
    /// `pool_stats`, `reload_config`) intentionally use the plain
    /// `lookup` so they keep working on a tainted resource.
    ///
    /// `warmup_pool` is intentionally routed through here (taint-gated),
    /// **not** the plain `lookup`: it runs `R::create` to materialize new
    /// instances against the credential, so it is acquire-like and must
    /// be blocked once the resource is tainted by a revoke.
    ///
    /// A tainted resource is rejected with
    /// [`ErrorKind::Revoked`](crate::error::ErrorKind::Revoked) — a
    /// non-terminal, retryable classification (the taint clears when the
    /// credential is re-registered), distinct from the
    /// [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) that
    /// the `shutting_down` funnel raises.
    fn lookup_for_acquire<R: Resource>(
        &self,
        scope: &ScopeLevel,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        Self::taint_gate::<R>(self.lookup::<R>(scope)?)
    }

    /// [`lookup_for_acquire`](Self::lookup_for_acquire) pinned to a resolved
    /// per-slot credential identity (the unambiguous acquire path).
    fn lookup_for_acquire_with<R: Resource>(
        &self,
        scope: &ScopeLevel,
        slot_identity: u64,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        Self::taint_gate::<R>(self.lookup_for::<R>(scope, slot_identity)?)
    }

    /// Shared taint check tail for the acquire-side lookups.
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
    /// own in-flight counter (the exact counter `revoke_slot` drains,
    /// ADR-0067 §Deferred) — closes that revoke-vs-acquire TOCTOU so a guard
    /// is never handed out on a just-revoked credential (ADR-0044/0036).
    /// Same error/classification as the gate so the caller-facing category
    /// is unchanged (`Revoked` → `ErrorCategory::Unavailable`).
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

    /// [`refresh_slot`](Self::refresh_slot) pinned to a resolved per-slot
    /// credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches (via the same
    /// unambiguous-by-construction path [`get_for`](crate::registry::Registry::get_for)
    /// backs), so a multi-tenant `(key, scope)` routes the rotation to the
    /// *specific* resolved row instead of failing closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous). This is
    /// the entry point the engine per-slot rotation fan-out drives once it
    /// has resolved a node's slot bindings; identity-agnostic
    /// [`refresh_slot`](Self::refresh_slot) stays fail-closed for the
    /// no-identity caller.
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
        skip(self),
        fields(key = %key, slot = %slot, slot_identity, topology, duration_ms)
    )]
    pub async fn refresh_slot_for(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: u64,
    ) -> Result<(), Error> {
        tracing::Span::current().record("slot_identity", slot_identity);
        let managed = self.lookup_any_for_slot_identity(key, &scope, slot_identity)?;
        self.refresh_resolved(key, slot, managed).await
    }

    /// Post-resolution refresh dispatch shared by
    /// [`refresh_slot`](Self::refresh_slot) (identity-agnostic) and
    /// [`refresh_slot_for`](Self::refresh_slot_for) (slot-identity-pinned).
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
    /// any `.await`.** Resolves the registry row pinned to a resolved
    /// per-slot credential identity and *taints it immediately* so the
    /// `acquire_*` funnel rejects new leases on the revoked credential, then
    /// returns a [`TaintedSlot`] handle the caller passes to
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
    /// *before* any per-resource timeout (ADR-0067 §Deferred).
    ///
    /// Identity routing: resolves the registry row whose `slot_identity`
    /// matches via the unambiguous-by-construction
    /// [`get_for`](crate::registry::Registry::get_for) path, so a
    /// multi-tenant `(key, scope)` taints the *specific* resolved row
    /// instead of failing closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous). This is
    /// the entry point the engine per-slot rotation fan-out drives on a
    /// lease revoke; identity-agnostic [`taint_slot`](Self::taint_slot) stays
    /// fail-closed for the no-identity caller.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of `key` at `scope`
    ///   matches `slot_identity`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    ///
    /// Carries only `key` / `slot` / `slot_identity` / `topology` (no
    /// credential material) onto the span.
    #[tracing::instrument(
        level = "debug",
        name = "nebula.resource.slot_taint",
        skip(self),
        fields(key = %key, slot = %slot, slot_identity, topology, op = "revoke")
    )]
    pub fn taint_slot_for(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: u64,
    ) -> Result<TaintedSlot, Error> {
        tracing::Span::current().record("slot_identity", slot_identity);
        let managed = self.lookup_any_for_slot_identity(key, &scope, slot_identity)?;
        Ok(Self::taint_now(key, slot, managed))
    }

    /// [`taint_slot_for`](Self::taint_slot_for) for the slot-identity-agnostic
    /// caller (the convenience [`revoke_slot`](Self::revoke_slot) path and
    /// non-fan-out callers/tests).
    ///
    /// Same eager, pre-`await` taint guarantee as
    /// [`taint_slot_for`](Self::taint_slot_for); only row resolution differs
    /// (identity-agnostic, so a multi-tenant `(key, scope)` fails closed with
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
    /// [`taint_slot_for`](Self::taint_slot_for); the safety-critical
    /// invariant — *taint is fully applied before this returns* — is written
    /// once here.
    fn taint_now(
        key: &ResourceKey,
        slot: &str,
        managed: Arc<dyn crate::registry::AnyManagedResource>,
    ) -> TaintedSlot {
        tracing::Span::current().record("topology", managed.topology_tag_erased().as_str());
        // Taint NOW — synchronously, before any caller `.await`. The acquire
        // pipelines re-check this taint *after* their per-resource in-flight
        // increment, so an acquire that passed the taint gate but had not yet
        // incremented cannot slip a guard out on the just-revoked credential
        // (ADR-0044/0036 "no authenticated traffic on a revoked credential
        // post-revoke"). Because this function is not `async`, the store has
        // *already happened* by the time control returns to the caller — a
        // subsequently-dropped timeout future on the drain tail cannot
        // un-apply it.
        managed.taint_erased();
        TaintedSlot {
            key: key.clone(),
            slot: slot.to_owned(),
            managed,
            tainted_at: Instant::now(),
        }
    }

    /// Default per-resource revoke budget for the back-compat
    /// back-to-back convenience callers ([`revoke_slot`](Self::revoke_slot)
    /// / [`revoke_slot_for`](Self::revoke_slot_for)).
    ///
    /// 30 s — the same budget the manager-wide `graceful_shutdown` drain
    /// uses and the value [`drain_and_revoke`](Self::drain_and_revoke)
    /// previously hard-coded for the drain wait. The engine rotation
    /// fan-out does **not** use this: it passes its own per-resource
    /// rotation budget so the timeout has one owner end-to-end (ADR-0067
    /// §Deferred / #690 review).
    pub const DEFAULT_REVOKE_DRAIN_TIMEOUT: std::time::Duration =
        std::time::Duration::from_secs(30);

    /// **Phase 2 of the revoke port — the cancellation-safe awaited tail.**
    /// Consumes a [`TaintedSlot`] from [`taint_slot`](Self::taint_slot) /
    /// [`taint_slot_for`](Self::taint_slot_for) (whose taint already ran
    /// synchronously) and performs the remaining steps:
    ///
    /// 1. **Drain** only *this resource's* in-flight handles via its own per-resource counter
    ///    (ADR-0067 §Deferred) — never the manager-wide `drain_tracker`, so a revoke is isolated
    ///    from in-flight traffic to unrelated resources.
    /// 2. **Dispatch** [`Resource::on_credential_revoke`] against the live runtime per topology.
    /// 3. Emit [`ResourceEvent::SlotRevoked`] / `SlotRevokeFailed`.
    ///
    /// **Single budget owner (ADR-0067 §Deferred / #690 review).** The
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
    /// ran in the synchronous [`taint_slot_for`](Self::taint_slot_for)
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

        // 1. Drain **only this resource's** in-flight handles (ADR-0067
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
    /// never skip the taint (ADR-0067 §Deferred). This convenience is for the
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

    /// [`revoke_slot`](Self::revoke_slot) pinned to a resolved per-slot
    /// credential identity — the slot-identity-aware two-phase convenience.
    ///
    /// Equivalent to [`taint_slot_for`](Self::taint_slot_for) immediately
    /// followed by [`drain_and_revoke`](Self::drain_and_revoke); a
    /// multi-tenant `(key, scope)` taints/drains/revokes the *specific*
    /// resolved row instead of failing closed with
    /// [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous). Like
    /// [`revoke_slot`](Self::revoke_slot) this is the back-compat
    /// back-to-back path; the engine fan-out calls the two phases separately
    /// (sync taint outside the timeout) per ADR-0067 §Deferred.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of `key` at `scope`
    ///   matches `slot_identity`.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - Whatever the resource's `on_credential_revoke` hook maps into [`Error`].
    pub async fn revoke_slot_for(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot: &str,
        slot_identity: u64,
    ) -> Result<(), Error> {
        let tainted = self.taint_slot_for(key, scope, slot, slot_identity)?;
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

    /// [`lookup_any_for_slot`](Self::lookup_any_for_slot) pinned to a
    /// resolved per-slot credential identity.
    ///
    /// Resolves through [`Registry::get_for`](crate::registry::Registry::get_for),
    /// which selects the single row whose `(scope, slot_identity)` matches
    /// and is therefore **unambiguous by construction** — a resolved
    /// credential pins exactly one row, so the
    /// [`LookupOutcome::Ambiguous`](crate::registry::LookupOutcome) arm
    /// cannot occur here (no fail-closed path is bypassed: the engine
    /// fan-out supplies the resolved identity it recorded at register time).
    /// A `slot_identity` that was never registered is
    /// [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound), never an
    /// accidental alias to another tenant's row.
    fn lookup_any_for_slot_identity(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: u64,
    ) -> Result<Arc<dyn crate::registry::AnyManagedResource>, Error> {
        use crate::registry::LookupOutcome;
        self.shutdown_guard()?;
        match self.registry.get_for(key, scope, slot_identity) {
            LookupOutcome::Found(any) => Ok(any),
            LookupOutcome::NotFound => Err(Error::not_found(key)),
            // Unreachable: `get_for` pins a single `(scope, slot_identity)`
            // row, so it never returns `Ambiguous`. Mapped to the same
            // fail-closed deny `lookup_any_for_slot` uses rather than a
            // panic, so a future registry change cannot turn an invariant
            // breach into a process abort.
            LookupOutcome::Ambiguous { rows } => Err(Error::ambiguous(format!(
                "{key}: {rows} rows matched a slot-identity-pinned lookup; \
                 expected exactly one (registry invariant breach)"
            ))
            .with_resource_key(key.clone())),
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
    ///   [`acquire_pooled_for`](Self::acquire_pooled_for) when the resolved
    ///   slot identity is known; this identity-agnostic path stays
    ///   fail-closed for the no-identity caller.
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
        let managed = self.lookup_for_acquire::<R>(&ctx.scope_level())?;
        self.run_pooled_acquire(managed, ctx, options).await
    }

    /// [`acquire_pooled`](Self::acquire_pooled) pinned to a resolved per-slot
    /// credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches, so a caller
    /// that resolved tenant A's credential reaches tenant A's runtime and
    /// never tenant B's. This is the unambiguous acquire path the engine
    /// resolution layer uses once it has resolved a node's slot bindings;
    /// it is also how callers reach a resource registered with a non-default
    /// [`RegisterOptions::with_slot_identity`].
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   pool topology.
    /// - Propagates pool-specific acquire errors.
    pub async fn acquire_pooled_for<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: u64,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup_for_acquire_with::<R>(&ctx.scope_level(), slot_identity)?;
        self.run_pooled_acquire(managed, ctx, options).await
    }

    /// Shared pooled acquire pipeline (resilience + gate + drain
    /// bookkeeping) over an already-resolved [`ManagedResource`]. The two
    /// public pooled-acquire entry points differ only in how they resolve
    /// the row (identity-agnostic vs. slot-identity-pinned); the topology
    /// dispatch itself is identical.
    async fn run_pooled_acquire<R>(
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
        let started = Instant::now();
        // Defense B against the `graceful_shutdown` race: pre-count this
        // acquire from the moment `lookup()` succeeds. RAII decrements + notifies
        // on every failure / cancel / panic path; on success the slot is handed
        // off to the resulting `ResourceGuard` so the count is held continuously
        // until the guard drops. The same counter is the per-resource one
        // `revoke_slot` drains, which is what the post-taint re-check below
        // relies on.
        let in_flight =
            InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
        // Post-count taint re-check (ADR-0044/0036 "no authenticated traffic
        // on a revoked credential post-revoke"): the taint gate ran in
        // `lookup_for_acquire`, but a revoke could have tainted *after* that
        // gate yet *before* the in-flight increment above. Re-checking here —
        // after the per-resource counter is incremented — closes that TOCTOU:
        // `revoke_slot` taints, then drains this same counter, so either we
        // observe the taint here or our increment is visible to its drain.
        // Mirrors the `shutting_down` Defense pattern; same `Revoked`
        // (→ `Unavailable`) classification as the taint gate.
        Self::reject_if_tainted_post_count::<R>(&managed)?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;
        let resilience = managed.resilience.clone();

        let result = execute_with_resilience(&resilience, || {
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
                    _ => Err(Error::permanent(format!(
                        "{}: expected Pool topology, registered as {}",
                        R::key(),
                        managed.topology.tag()
                    ))),
                }
            }
        })
        .await;

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
        let managed = self.lookup_for_acquire::<R>(&ctx.scope_level())?;
        self.run_resident_acquire(managed, ctx, options).await
    }

    /// [`acquire_resident`](Self::acquire_resident) pinned to a resolved
    /// per-slot credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches, so a caller
    /// that resolved tenant A's credential reaches tenant A's runtime and
    /// never tenant B's. This is the unambiguous acquire path the engine
    /// resolution layer uses once it has resolved a node's slot bindings;
    /// it is also how callers reach a resource registered with a non-default
    /// [`RegisterOptions::with_slot_identity`].
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   resident topology.
    /// - Propagates resident-specific acquire errors.
    pub async fn acquire_resident_for<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: u64,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        let managed = self.lookup_for_acquire_with::<R>(&ctx.scope_level(), slot_identity)?;
        self.run_resident_acquire(managed, ctx, options).await
    }

    /// Shared resident acquire pipeline (resilience + gate + drain
    /// bookkeeping) over an already-resolved [`ManagedResource`]. The two
    /// public resident-acquire entry points differ only in how they resolve
    /// the row (identity-agnostic vs. slot-identity-pinned); the topology
    /// dispatch itself is identical.
    async fn run_resident_acquire<R>(
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
        let started = Instant::now();
        // Defense B against the `graceful_shutdown` race — see `acquire_pooled`.
        let in_flight =
            InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
        // Post-count taint re-check — see `run_pooled_acquire` (ADR-0044/0036
        // / ADR-0067 §Deferred): closes the revoke-vs-acquire TOCTOU now that
        // this acquire is reflected in the per-resource counter `revoke_slot`
        // drains.
        Self::reject_if_tainted_post_count::<R>(&managed)?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;
        let resilience = managed.resilience.clone();

        let result = execute_with_resilience(&resilience, || {
            let config = managed.config();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Resident(rt) => {
                        rt.acquire(&managed.resource, &config, ctx, options).await
                    },
                    _ => Err(Error::permanent(format!(
                        "{}: expected Resident topology, registered as {}",
                        R::key(),
                        managed.topology.tag()
                    ))),
                }
            }
        })
        .await;

        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        match result {
            Ok(h) => Ok(h.with_drain_tracker(in_flight.release_to_guard())),
            Err(e) => Err(e),
        }
    }

    /// Acquires a handle to a service resource.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   service topology.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) — a
    ///   permanent (non-retryable) caller-conflict deny — if more than one
    ///   resolved-credential registration exists for `(R, scope)`
    ///   (multi-tenant). Acquire through the slot-identity-pinned
    ///   [`acquire_service_for`](Self::acquire_service_for) when the resolved
    ///   slot identity is known; this identity-agnostic path stays
    ///   fail-closed for the no-identity caller.
    /// - Propagates service-specific acquire errors.
    pub async fn acquire_service<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::service::Service + Clone + Send + Sync + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup_for_acquire::<R>(&ctx.scope_level())?;
        self.run_service_acquire(managed, ctx, options).await
    }

    /// [`acquire_service`](Self::acquire_service) pinned to a resolved
    /// per-slot credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches, so a caller
    /// that resolved tenant A's credential reaches tenant A's runtime and
    /// never tenant B's. This is the unambiguous acquire path the engine
    /// resolution layer uses once it has resolved a node's slot bindings;
    /// it is also how callers reach a resource registered with a non-default
    /// [`RegisterOptions::with_slot_identity`].
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   service topology.
    /// - Propagates service-specific acquire errors.
    pub async fn acquire_service_for<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: u64,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::service::Service + Clone + Send + Sync + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup_for_acquire_with::<R>(&ctx.scope_level(), slot_identity)?;
        self.run_service_acquire(managed, ctx, options).await
    }

    /// Shared service acquire pipeline (resilience + gate + drain
    /// bookkeeping) over an already-resolved [`ManagedResource`]. The two
    /// public service-acquire entry points differ only in how they resolve
    /// the row (identity-agnostic vs. slot-identity-pinned); the topology
    /// dispatch itself is identical.
    async fn run_service_acquire<R>(
        &self,
        managed: Arc<ManagedResource<R>>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::service::Service + Clone + Send + Sync + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let started = Instant::now();
        // Defense B against the `graceful_shutdown` race — see `acquire_pooled`.
        let in_flight =
            InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
        // Post-count taint re-check — see `run_pooled_acquire` (ADR-0044/0036
        // / ADR-0067 §Deferred): closes the revoke-vs-acquire TOCTOU now that
        // this acquire is reflected in the per-resource counter `revoke_slot`
        // drains.
        Self::reject_if_tainted_post_count::<R>(&managed)?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;
        let resilience = managed.resilience.clone();

        let result = execute_with_resilience(&resilience, || {
            let generation = managed.generation();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Service(rt) => {
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
                    _ => Err(Error::permanent(format!(
                        "{}: expected Service topology, registered as {}",
                        R::key(),
                        managed.topology.tag()
                    ))),
                }
            }
        })
        .await;

        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        match result {
            Ok(h) => Ok(h.with_drain_tracker(in_flight.release_to_guard())),
            Err(e) => Err(e),
        }
    }

    /// Acquires a handle to a transport resource.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   transport topology.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) — a
    ///   permanent (non-retryable) caller-conflict deny — if more than one
    ///   resolved-credential registration exists for `(R, scope)`
    ///   (multi-tenant). Acquire through the slot-identity-pinned
    ///   [`acquire_transport_for`](Self::acquire_transport_for) when the
    ///   resolved slot identity is known; this identity-agnostic path stays
    ///   fail-closed for the no-identity caller.
    /// - Propagates transport-specific acquire errors.
    pub async fn acquire_transport<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::transport::Transport + Clone + Send + Sync + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup_for_acquire::<R>(&ctx.scope_level())?;
        self.run_transport_acquire(managed, ctx, options).await
    }

    /// [`acquire_transport`](Self::acquire_transport) pinned to a resolved
    /// per-slot credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches, so a caller
    /// that resolved tenant A's credential reaches tenant A's runtime and
    /// never tenant B's. This is the unambiguous acquire path the engine
    /// resolution layer uses once it has resolved a node's slot bindings;
    /// it is also how callers reach a resource registered with a non-default
    /// [`RegisterOptions::with_slot_identity`].
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   transport topology.
    /// - Propagates transport-specific acquire errors.
    pub async fn acquire_transport_for<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: u64,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::transport::Transport + Clone + Send + Sync + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup_for_acquire_with::<R>(&ctx.scope_level(), slot_identity)?;
        self.run_transport_acquire(managed, ctx, options).await
    }

    /// Shared transport acquire pipeline (resilience + gate + drain
    /// bookkeeping) over an already-resolved [`ManagedResource`]. The two
    /// public transport-acquire entry points differ only in how they resolve
    /// the row (identity-agnostic vs. slot-identity-pinned); the topology
    /// dispatch itself is identical.
    async fn run_transport_acquire<R>(
        &self,
        managed: Arc<ManagedResource<R>>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::transport::Transport + Clone + Send + Sync + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let started = Instant::now();
        // Defense B against the `graceful_shutdown` race — see `acquire_pooled`.
        let in_flight =
            InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
        // Post-count taint re-check — see `run_pooled_acquire` (ADR-0044/0036
        // / ADR-0067 §Deferred): closes the revoke-vs-acquire TOCTOU now that
        // this acquire is reflected in the per-resource counter `revoke_slot`
        // drains.
        Self::reject_if_tainted_post_count::<R>(&managed)?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;
        let resilience = managed.resilience.clone();

        let result = execute_with_resilience(&resilience, || {
            let generation = managed.generation();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Transport(rt) => {
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
                    _ => Err(Error::permanent(format!(
                        "{}: expected Transport topology, registered as {}",
                        R::key(),
                        managed.topology.tag()
                    ))),
                }
            }
        })
        .await;

        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        match result {
            Ok(h) => Ok(h.with_drain_tracker(in_flight.release_to_guard())),
            Err(e) => Err(e),
        }
    }

    /// Acquires a handle to an exclusive resource.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   exclusive topology.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) — a
    ///   permanent (non-retryable) caller-conflict deny — if more than one
    ///   resolved-credential registration exists for `(R, scope)`
    ///   (multi-tenant). Acquire through the slot-identity-pinned
    ///   [`acquire_exclusive_for`](Self::acquire_exclusive_for) when the
    ///   resolved slot identity is known; this identity-agnostic path stays
    ///   fail-closed for the no-identity caller.
    /// - Propagates exclusive-specific acquire errors.
    pub async fn acquire_exclusive<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::exclusive::Exclusive + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup_for_acquire::<R>(&ctx.scope_level())?;
        self.run_exclusive_acquire(managed, options).await
    }

    /// [`acquire_exclusive`](Self::acquire_exclusive) pinned to a resolved
    /// per-slot credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches, so a caller
    /// that resolved tenant A's credential reaches tenant A's runtime and
    /// never tenant B's. This is the unambiguous acquire path the engine
    /// resolution layer uses once it has resolved a node's slot bindings;
    /// it is also how callers reach a resource registered with a non-default
    /// [`RegisterOptions::with_slot_identity`].
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   exclusive topology.
    /// - Propagates exclusive-specific acquire errors.
    pub async fn acquire_exclusive_for<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: u64,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::exclusive::Exclusive + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup_for_acquire_with::<R>(&ctx.scope_level(), slot_identity)?;
        self.run_exclusive_acquire(managed, options).await
    }

    /// Shared exclusive acquire pipeline (resilience + gate + drain
    /// bookkeeping) over an already-resolved [`ManagedResource`]. The two
    /// public exclusive-acquire entry points differ only in how they resolve
    /// the row (identity-agnostic vs. slot-identity-pinned); the topology
    /// dispatch itself is identical.
    ///
    /// Unlike the other topologies' shared pipelines this takes no
    /// `ResourceContext`: exclusive `rt.acquire` is context-free, and the
    /// only `ctx` use (scope resolution) already happened in the two public
    /// entry points before the row was resolved.
    async fn run_exclusive_acquire<R>(
        &self,
        managed: Arc<ManagedResource<R>>,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::exclusive::Exclusive + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let started = Instant::now();
        // Defense B against the `graceful_shutdown` race — see `acquire_pooled`.
        let in_flight =
            InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
        // Post-count taint re-check — see `run_pooled_acquire` (ADR-0044/0036
        // / ADR-0067 §Deferred): closes the revoke-vs-acquire TOCTOU now that
        // this acquire is reflected in the per-resource counter `revoke_slot`
        // drains.
        Self::reject_if_tainted_post_count::<R>(&managed)?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;
        let resilience = managed.resilience.clone();

        let result = execute_with_resilience(&resilience, || {
            let generation = managed.generation();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Exclusive(rt) => {
                        rt.acquire(
                            &managed.resource,
                            &managed.release_queue,
                            generation,
                            options,
                            self.metrics.clone(),
                        )
                        .await
                    },
                    _ => Err(Error::permanent(format!(
                        "{}: expected Exclusive topology, registered as {}",
                        R::key(),
                        managed.topology.tag()
                    ))),
                }
            }
        })
        .await;

        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        match result {
            Ok(h) => Ok(h.with_drain_tracker(in_flight.release_to_guard())),
            Err(e) => Err(e),
        }
    }

    /// Attempts a non-blocking acquire of a pooled resource.
    ///
    /// Returns immediately with [`ErrorKind::Backpressure`](crate::error::ErrorKind::Backpressure)
    /// if all `max_size` pool slots are currently occupied by active handles.
    /// Unlike [`acquire_pooled`](Self::acquire_pooled), this method **never** queues
    /// the caller — use it to shed load rather than back-pressure callers.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Backpressure`](crate::error::ErrorKind::Backpressure) if the pool is full.
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   pool topology.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) — a
    ///   permanent (non-retryable) caller-conflict deny — if more than one
    ///   resolved-credential registration exists for `(R, scope)`
    ///   (multi-tenant). This non-blocking try-path is identity-agnostic and
    ///   stays fail-closed; use the slot-identity-pinned
    ///   [`acquire_pooled_for`](Self::acquire_pooled_for) when the resolved
    ///   slot identity is known and a blocking acquire is acceptable.
    pub async fn try_acquire_pooled<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let started = Instant::now();
        let managed = self.lookup_for_acquire::<R>(&ctx.scope_level())?;
        // Defense B against the `graceful_shutdown` race — see `acquire_pooled`.
        let in_flight =
            InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
        // Post-count taint re-check — see `run_pooled_acquire` (ADR-0044/0036
        // / ADR-0067 §Deferred): closes the revoke-vs-acquire TOCTOU now that
        // this acquire is reflected in the per-resource counter `revoke_slot`
        // drains.
        Self::reject_if_tainted_post_count::<R>(&managed)?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;

        let result = match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                let config = managed.config();
                let generation = managed.generation();
                rt.try_acquire(
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
            _ => Err(Error::permanent(format!(
                "{}: expected Pool topology for try_acquire, registered as {}",
                R::key(),
                managed.topology.tag()
            ))),
        };

        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        match result {
            Ok(h) => Ok(h.with_drain_tracker(in_flight.release_to_guard())),
            Err(e) => Err(e),
        }
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
    /// Per ADR-0044, the resource's `#[credential]` slot fields are
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
    ///   ([`acquire_pooled_for`](Self::acquire_pooled_for)).
    pub async fn warmup_pool<R>(&self, ctx: &ResourceContext) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup_for_acquire::<R>(&ctx.scope_level())?;
        let config = managed.config();
        match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                // `warmup` runs `R::create` against the resolved credential
                // to materialize fresh pool instances — it is acquire-like
                // and must observe the SAME revoke-vs-acquire TOCTOU close
                // the `run_*_acquire` pipelines use (#679 / ADR-0044/0036).
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

        // Determine outcome based on topology.
        let outcome = match managed.topology {
            TopologyRuntime::Service(_) => ReloadOutcome::PendingDrain {
                old_generation: prev_gen,
            },
            _ => ReloadOutcome::SwappedImmediately,
        };

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

    /// Returns a reference to the recovery group registry.
    pub fn recovery_groups(&self) -> &RecoveryGroupRegistry {
        &self.recovery_groups
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
/// Used by [`Manager::register_from_value`] to evaluate dynamic config values before serde
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
                    "register_from_value: template parse failed for `{s}`: {e}"
                ))
            })?;
            let rendered = engine.render_template(&template, ctx).map_err(|e| {
                Error::permanent(format!(
                    "register_from_value: template render failed for `{s}`: {e}"
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

// RAII guard that pre-counts an in-flight `acquire_*` call against
// `Manager::drain_tracker` from the moment `lookup()` succeeds until either
// (a) the acquire completes and the slot is handed off to the resulting
// `ResourceGuard`, or (b) the acquire fails / panics / is cancelled and the
// slot is decremented + waiters notified on drop.
//
// This is **Defense B** of the `graceful_shutdown` race fix (Defense A is
// the `shutting_down` check inside `Manager::lookup`). Without pre-
// counting, an acquire that passes `lookup()` before `cancel.cancel()` can
// complete *after* `wait_for_drain()` saw `0` and the registry was cleared
// — the caller would end up holding a `ResourceGuard` to a registry that
// has been torn down.

pub(crate) struct InFlightCounter {
    /// Manager-wide drain tracker — the `graceful_shutdown` drain primitive.
    manager: crate::guard::DrainTracker,
    /// Per-`ManagedResource` in-flight tracker — the *only* counter
    /// `revoke_slot` drains (ADR-0067 §Deferred), so a revoke on one
    /// resource never blocks on a sibling's in-flight work.
    per_resource: crate::guard::DrainTracker,
    armed: bool,
}

impl InFlightCounter {
    /// Pre-counts an in-flight acquire against **both** the manager-wide
    /// drain tracker (shutdown) and the per-resource tracker (revoke drain
    /// + the taint→increment→re-check TOCTOU close, ADR-0044/0036/0067).
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
