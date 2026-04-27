//! Central resource manager — registration, acquire dispatch, and shutdown.
//!
//! [`Manager`] is the single entry point for the resource subsystem. It owns
//! the [`Registry`], [`RecoveryGroupRegistry`], and a [`CancellationToken`]
//! for coordinated shutdown.
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

use std::{
    any::TypeId,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering},
    },
    time::{Duration, Instant},
};

use nebula_core::{CredentialId, LayerLifecycle, ResourceKey, ScopeLevel};
use nebula_credential::Credential;
use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;

use crate::{
    context::ResourceContext,
    error::{Error, RefreshOutcome, RevokeOutcome},
    events::ResourceEvent,
    integration::AcquireResilience,
    metrics::{ResourceOpsMetrics, ResourceOpsSnapshot},
    options::AcquireOptions,
    recovery::{
        gate::{GateState, RecoveryGate, RecoveryTicket, TryBeginError},
        group::RecoveryGroupRegistry,
    },
    registry::Registry,
    release_queue::{ReleaseQueue, ReleaseQueueHandle},
    reload::ReloadOutcome,
    resource::Resource,
    runtime::{TopologyRuntime, managed::ManagedResource},
};

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

/// Policy that controls what `graceful_shutdown` does when the
/// drain phase expires with handles still outstanding (#302).
///
/// Before this split, `graceful_shutdown` always proceeded to
/// `registry.clear()` even on timeout, dropping live `ManagedResource`s
/// while handles remained outstanding. That turned a cooperative shutdown
/// into a use-after-logical-drop. The policy makes the choice explicit.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DrainTimeoutPolicy {
    /// On drain timeout, return
    /// [`ShutdownError::DrainTimeout`] **without** clearing the registry.
    /// Live handles remain valid and the caller decides what to do next.
    /// This is the default — it preserves the "graceful" guarantee.
    #[default]
    Abort,
    /// On drain timeout, log, clear the registry anyway, and report the
    /// outstanding-handle count in [`ShutdownReport`]. Opt-in escape hatch
    /// for supervisors that must exit on a deadline regardless of cost.
    Force,
}

/// Configuration for graceful shutdown.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ShutdownConfig {
    /// How long to wait for in-flight handles to be released.
    pub drain_timeout: Duration,
    /// What to do on drain timeout. Default: [`DrainTimeoutPolicy::Abort`].
    pub on_drain_timeout: DrainTimeoutPolicy,
    /// Upper bound on how long Phase 4 will wait for release-queue
    /// workers to finish processing outstanding tasks.
    pub release_queue_timeout: Duration,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout: Duration::from_secs(30),
            on_drain_timeout: DrainTimeoutPolicy::Abort,
            release_queue_timeout: Duration::from_secs(10),
        }
    }
}

impl ShutdownConfig {
    /// Override the drain timeout, returning `self` for chaining.
    ///
    /// `#[non_exhaustive]` prevents external crates from using struct
    /// literal construction; this (and the sibling setters) is the
    /// forward-compatible entry point for per-field customization.
    #[must_use]
    pub fn with_drain_timeout(mut self, timeout: Duration) -> Self {
        self.drain_timeout = timeout;
        self
    }

    /// Override the drain-timeout policy.
    #[must_use]
    pub fn with_drain_timeout_policy(mut self, policy: DrainTimeoutPolicy) -> Self {
        self.on_drain_timeout = policy;
        self
    }

    /// Override the release-queue timeout budget for Phase 4.
    #[must_use]
    pub fn with_release_queue_timeout(mut self, timeout: Duration) -> Self {
        self.release_queue_timeout = timeout;
        self
    }
}

/// Structured result of a successful (or forced-through) graceful shutdown.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ShutdownReport {
    /// How many `ResourceGuard`s were still outstanding when the drain
    /// phase finished. Zero on the happy path. Nonzero only when the
    /// caller explicitly opted into [`DrainTimeoutPolicy::Force`].
    pub outstanding_handles_after_drain: u64,
    /// Whether Phase 3 (`registry.clear`) actually ran.
    pub registry_cleared: bool,
    /// Whether Phase 4 (release-queue drain) completed within
    /// `release_queue_timeout`.
    pub release_queue_drained: bool,
}

/// Errors returned by [`Manager::graceful_shutdown`].
///
/// Each variant corresponds to a failure mode that was previously silently
/// absorbed by the old infallible signature. A timeout during drain, for
/// example, used to be a `tracing::warn!` and a forced `registry.clear()`;
/// it is now a typed error that the caller must handle.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ShutdownError {
    /// `graceful_shutdown` was already in progress when this call entered.
    /// CAS-guarded so exactly one caller wins the race.
    #[error("graceful shutdown already in progress")]
    AlreadyShuttingDown,

    /// The drain phase did not finish within `drain_timeout` and the
    /// policy was [`DrainTimeoutPolicy::Abort`]. The registry was **not**
    /// cleared and any outstanding handles remain valid.
    #[error(
        "drain timeout expired with {outstanding} handle(s) still active; registry was NOT cleared (policy=Abort)"
    )]
    DrainTimeout {
        /// Snapshot of the drain-tracker counter at the moment the timeout
        /// fired.
        outstanding: u64,
    },

    /// Phase 4 did not finish within `release_queue_timeout`.
    #[error("release queue workers did not finish within {timeout:?}")]
    ReleaseQueueTimeout {
        /// The budget that was exceeded.
        timeout: Duration,
    },
}

/// Internal drain-phase error used by the private `wait_for_drain` helper.
/// Carries the outstanding-handle count at the moment the drain timer fired.
#[derive(Debug)]
struct DrainTimeoutError {
    outstanding: u64,
}

/// Configuration for the [`Manager`].
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// Number of background workers for the release queue.
    ///
    /// Defaults to 2.
    pub release_queue_workers: usize,
    /// Optional shared metrics registry for telemetry counters.
    ///
    /// When `Some`, the manager records resource operation counters
    /// (`acquire_total`, `release_total`, etc.) into the registry.
    /// When `None`, metrics are silently skipped (zero overhead).
    pub metrics_registry: Option<Arc<nebula_telemetry::metrics::MetricsRegistry>>,
    /// Default per-resource timeout budget for credential rotation hooks
    /// (`on_credential_refresh` / `on_credential_revoke`).
    ///
    /// Each registered resource may override this via
    /// `RegisterOptions::credential_rotation_timeout` (Task 6). When a
    /// rotation hook exceeds the per-resource budget the dispatcher reports
    /// `RefreshOutcome::TimedOut` / `RevokeOutcome::TimedOut` and the
    /// remaining sibling dispatches continue unaffected (security amendment
    /// B-1: per-resource isolation).
    ///
    /// Defaults to 30 seconds.
    pub credential_rotation_timeout: Duration,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            release_queue_workers: 2,
            metrics_registry: None,
            credential_rotation_timeout: Duration::from_secs(30),
        }
    }
}

/// Extended options for resource registration.
///
/// Used with the `register_*_with` convenience methods to configure
/// resilience and recovery beyond the simple `register_*` defaults.
#[derive(Debug, Clone)]
pub struct RegisterOptions {
    /// Scope level for the resource (default: `Global`).
    pub scope: ScopeLevel,
    /// Optional acquire resilience (timeout + retry + circuit breaker).
    pub resilience: Option<AcquireResilience>,
    /// Optional recovery gate for thundering-herd prevention.
    pub recovery_gate: Option<Arc<RecoveryGate>>,
    /// Credential ID this resource binds to.
    ///
    /// Required for resources where `R::Credential != NoCredential` —
    /// `Manager::register` returns [`Error::missing_credential_id`] if a
    /// credential-bearing resource is registered without an ID. Ignored for
    /// `NoCredential`-bound resources (the manager logs a warning if one is
    /// supplied alongside `Credential = NoCredential`).
    ///
    /// Set via [`RegisterOptions::with_credential_id`].
    pub credential_id: Option<CredentialId>,
    /// Per-resource override for the default credential rotation timeout.
    ///
    /// `None` falls back to [`ManagerConfig::credential_rotation_timeout`]
    /// (default `30s`). Only meaningful for credential-bearing resources;
    /// ignored for `NoCredential`-bound resources.
    ///
    /// Set via [`RegisterOptions::with_rotation_timeout`].
    pub credential_rotation_timeout: Option<Duration>,
}

impl Default for RegisterOptions {
    fn default() -> Self {
        Self {
            scope: ScopeLevel::Global,
            resilience: None,
            recovery_gate: None,
            credential_id: None,
            credential_rotation_timeout: None,
        }
    }
}

impl RegisterOptions {
    /// Sets the credential ID this resource binds to.
    ///
    /// Required for credential-bearing resources (`R::Credential != NoCredential`).
    /// `Manager::register` errors with [`Error::missing_credential_id`] if a
    /// credential-bearing resource is registered without an ID.
    #[must_use]
    pub fn with_credential_id(mut self, id: CredentialId) -> Self {
        self.credential_id = Some(id);
        self
    }

    /// Overrides the default credential rotation timeout for this resource.
    ///
    /// Falls back to [`ManagerConfig::credential_rotation_timeout`] (default
    /// `30s`) when not set. Only meaningful for credential-bearing resources.
    #[must_use]
    pub fn with_rotation_timeout(mut self, timeout: Duration) -> Self {
        self.credential_rotation_timeout = Some(timeout);
        self
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
pub struct Manager {
    registry: Registry,
    recovery_groups: RecoveryGroupRegistry,
    cancel: CancellationToken,
    metrics: Option<ResourceOpsMetrics>,
    event_tx: broadcast::Sender<ResourceEvent>,
    release_queue: Arc<ReleaseQueue>,
    release_queue_handle: tokio::sync::Mutex<Option<ReleaseQueueHandle>>,
    /// Tracks active `ResourceGuard`s for drain-aware shutdown.
    drain_tracker: Arc<(AtomicU64, Notify)>,
    /// CAS-guarded idempotency flag for `graceful_shutdown`. Flipped
    /// false → true by the winning caller; losers return
    /// [`ShutdownError::AlreadyShuttingDown`].
    shutting_down: AtomicBool,
    /// Reverse index: credential_id → dispatchers for resources that bind to this credential.
    ///
    /// Populated at register time when `R::Credential != NoCredential`. Read by
    /// `Manager::on_credential_refreshed` / `_revoked` to fan out rotation hooks
    /// in parallel via `join_all` (per Tech Spec §3.2).
    ///
    /// `Arc<dyn ResourceDispatcher>` provides type-erased dispatch — see
    /// `crate::rotation::TypedDispatcher<R>`.
    credential_resources:
        dashmap::DashMap<CredentialId, Vec<Arc<dyn crate::rotation::ResourceDispatcher>>>,
    /// Default per-resource timeout budget for credential rotation hooks.
    ///
    /// Sourced from [`ManagerConfig::credential_rotation_timeout`] at
    /// construction. Per-resource overrides flow through
    /// `TypedDispatcher::timeout_override()` (set at register time, Task 6).
    credential_rotation_timeout: Duration,
    /// Optional lifecycle handle for coordinated cancellation (spec 08).
    lifecycle: Option<LayerLifecycle>,
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
        let metrics = config
            .metrics_registry
            .as_ref()
            .map(|reg| ResourceOpsMetrics::new(reg));
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
            credential_resources: dashmap::DashMap::new(),
            credential_rotation_timeout: config.credential_rotation_timeout,
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

    /// Registers a resource with its config, auth, scope, topology,
    /// optional resilience configuration, and optional recovery gate.
    ///
    /// The resource is wrapped in a [`ManagedResource`] and stored in the
    /// registry under `R::key()`. If a resource with the same key and scope
    /// is already registered, it is silently replaced.
    ///
    /// The manager's internal [`ReleaseQueue`] is automatically shared with
    /// the managed resource — callers never need to create or manage it.
    ///
    /// When `resilience` is `Some`, acquire calls are wrapped with
    /// timeout and retry logic from [`AcquireResilience`].
    ///
    /// When `recovery_gate` is `Some`, acquire calls check the gate before
    /// proceeding. If the backend is recovering or permanently failed,
    /// callers receive an immediate error instead of hitting the dead
    /// backend. On transient acquire failures the gate is passively
    /// triggered so subsequent callers fast-fail.
    ///
    /// `credential_id` binds the resource into the credential rotation
    /// reverse-index. Must be `Some` for credential-bearing resources
    /// (`R::Credential != NoCredential`); ignored (with a warning) for
    /// `NoCredential`-bound resources. Sourced from
    /// [`RegisterOptions::credential_id`] in the `_with` shorthand variants.
    ///
    /// `credential_rotation_timeout` overrides
    /// [`ManagerConfig::credential_rotation_timeout`] (default `30s`) for
    /// this resource only. `None` keeps the manager-wide default. Sourced
    /// from [`RegisterOptions::credential_rotation_timeout`] in the `_with`
    /// shorthand variants.
    ///
    /// # Errors
    ///
    /// Returns an error if config validation fails on the provided config,
    /// or if a credential-bearing resource is registered without a
    /// `credential_id` (see [`Error::missing_credential_id`]).
    // Task 11 (Manager file-split) is slated to consolidate scope/resilience/
    // recovery_gate/credential_id/credential_rotation_timeout into a single
    // `RegisterOptions` parameter. Until that lands, the positional surface
    // mirrors the existing pre-Task-6 signature with two new credential
    // parameters wired through to `register_inner`.
    #[expect(
        clippy::too_many_arguments,
        reason = "consolidation into `RegisterOptions` deferred to Task 11 (Manager file-split); self-fires when arg count drops"
    )]
    pub fn register<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        scope: ScopeLevel,
        topology: TopologyRuntime<R>,
        resilience: Option<AcquireResilience>,
        recovery_gate: Option<Arc<RecoveryGate>>,
        credential_id: Option<CredentialId>,
        credential_rotation_timeout: Option<Duration>,
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
            credential_id,
        });

        let type_id = TypeId::of::<ManagedResource<R>>();
        self.registry
            .register(key.clone(), type_id, scope, managed.clone());

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

        // Reverse-index write — populates `credential_resources` for
        // credential-bearing resources, no-op for `NoCredential`-bound ones.
        // `credential_id` and `credential_rotation_timeout` flow through
        // [`RegisterOptions`] in the `_with` shorthand variants.
        self.register_inner(
            Arc::clone(&managed),
            credential_id,
            credential_rotation_timeout,
        )?;

        tracing::debug!(%key, "resource registered");
        Ok(())
    }

    /// Internal helper: write to `credential_resources` reverse-index when the
    /// resource binds a real credential; no-op for `NoCredential`-bound resources.
    ///
    /// Called by `register<R>` after the registry write succeeds. The `TypeId`
    /// check distinguishes credential-bearing resources from opt-out marker types
    /// at compile time.
    ///
    /// # Errors
    ///
    /// Returns `Error::missing_credential_id` when a credential-bearing resource
    /// (`R::Credential != NoCredential`) is registered without a `credential_id`.
    fn register_inner<R: Resource>(
        &self,
        managed: Arc<ManagedResource<R>>,
        credential_id: Option<CredentialId>,
        timeout_override: Option<Duration>,
    ) -> Result<(), Error> {
        let opted_out =
            TypeId::of::<R::Credential>() == TypeId::of::<nebula_credential::NoCredential>();

        match (opted_out, credential_id) {
            (true, Some(_)) => {
                tracing::warn!(
                    resource = %R::key(),
                    "register: NoCredential resource provided a credential_id; ignoring"
                );
            },
            (true, None) => {
                // Normal path for NoCredential-bound resources — no reverse-index write.
            },
            (false, None) => {
                return Err(Error::missing_credential_id(R::key()));
            },
            (false, Some(id)) => {
                let dispatcher: Arc<dyn crate::rotation::ResourceDispatcher> = Arc::new(
                    crate::rotation::TypedDispatcher::new(managed, timeout_override),
                );
                self.credential_resources
                    .entry(id)
                    .or_default()
                    .push(dispatcher);
            },
        }
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
        R: Resource<Credential = nebula_credential::NoCredential>,
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
        R: Resource<Credential = nebula_credential::NoCredential>,
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
        R: Resource<Credential = nebula_credential::NoCredential>,
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
        R: Resource<Credential = nebula_credential::NoCredential>,
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
        R: Resource<Credential = nebula_credential::NoCredential>,
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
            None,
            None,
        )
    }

    /// Registers a pooled resource with extended options.
    ///
    /// Like [`register_pooled`](Self::register_pooled) but accepts
    /// [`RegisterOptions`] for scope, resilience, recovery gate, and
    /// credential rotation binding.
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
        R: Resource<Credential = nebula_credential::NoCredential>,
    {
        use crate::resource::ResourceConfig as _;

        validate_pool_config(&pool_config)?;

        let fingerprint = config.fingerprint();
        self.register(
            resource,
            config,
            options.scope,
            TopologyRuntime::Pool(crate::runtime::pool::PoolRuntime::<R>::new(
                pool_config,
                fingerprint,
            )),
            options.resilience,
            options.recovery_gate,
            options.credential_id,
            options.credential_rotation_timeout,
        )
    }

    /// Registers a resident resource with extended options.
    ///
    /// Like [`register_resident`](Self::register_resident) but accepts
    /// [`RegisterOptions`] for scope, resilience, recovery gate, and
    /// credential rotation binding.
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
        R: Resource<Credential = nebula_credential::NoCredential>,
    {
        self.register(
            resource,
            config,
            options.scope,
            TopologyRuntime::Resident(crate::runtime::resident::ResidentRuntime::<R>::new(
                resident_config,
            )),
            options.resilience,
            options.recovery_gate,
            options.credential_id,
            options.credential_rotation_timeout,
        )
    }

    /// Registers a service resource with extended options.
    ///
    /// Like [`register_service`](Self::register_service) but accepts
    /// [`RegisterOptions`] for scope, resilience, recovery gate, and
    /// credential rotation binding.
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
        R: Resource<Credential = nebula_credential::NoCredential>,
    {
        self.register(
            resource,
            config,
            options.scope,
            TopologyRuntime::Service(crate::runtime::service::ServiceRuntime::<R>::new(
                runtime,
                service_config,
            )),
            options.resilience,
            options.recovery_gate,
            options.credential_id,
            options.credential_rotation_timeout,
        )
    }

    /// Registers a transport resource with extended options.
    ///
    /// Like [`register_transport`](Self::register_transport) but accepts
    /// [`RegisterOptions`] for scope, resilience, recovery gate, and
    /// credential rotation binding.
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
        R: Resource<Credential = nebula_credential::NoCredential>,
    {
        self.register(
            resource,
            config,
            options.scope,
            TopologyRuntime::Transport(crate::runtime::transport::TransportRuntime::<R>::new(
                runtime,
                transport_config,
            )),
            options.resilience,
            options.recovery_gate,
            options.credential_id,
            options.credential_rotation_timeout,
        )
    }

    /// Registers an exclusive resource with extended options.
    ///
    /// Like [`register_exclusive`](Self::register_exclusive) but accepts
    /// [`RegisterOptions`] for scope, resilience, recovery gate, and
    /// credential rotation binding.
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
        R: Resource<Credential = nebula_credential::NoCredential>,
    {
        self.register(
            resource,
            config,
            options.scope,
            TopologyRuntime::Exclusive(crate::runtime::exclusive::ExclusiveRuntime::<R>::new(
                runtime,
                exclusive_config,
            )),
            options.resilience,
            options.recovery_gate,
            options.credential_id,
            options.credential_rotation_timeout,
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
        if self.cancel.is_cancelled() {
            return Err(Error::cancelled());
        }

        self.registry
            .get_typed::<R>(scope)
            .ok_or_else(|| Error::not_found(&R::key()))
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
    /// - Propagates pool-specific acquire errors.
    pub async fn acquire_pooled<R>(
        &self,
        scheme: &<R::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let started = Instant::now();
        let managed = self.lookup::<R>(&ctx.scope_level())?;
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
                            scheme,
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
        result.map(|h| h.with_drain_tracker(self.drain_tracker.clone()))
    }

    /// Acquires a pooled resource handle without scheme material.
    ///
    /// Shorthand for [`acquire_pooled`](Self::acquire_pooled) with `scheme = &()`.
    /// Only available when `R::Credential = NoCredential` (so
    /// `<R::Credential as Credential>::Scheme = ()`).
    pub async fn acquire_pooled_default<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled<Credential = nebula_credential::NoCredential>
            + Clone
            + Send
            + Sync
            + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        self.acquire_pooled::<R>(&(), ctx, options).await
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
        scheme: &<R::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        let started = Instant::now();
        let managed = self.lookup::<R>(&ctx.scope_level())?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;
        let resilience = managed.resilience.clone();

        let result = execute_with_resilience(&resilience, || {
            let config = managed.config();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Resident(rt) => {
                        rt.acquire(&managed.resource, &config, scheme, ctx, options)
                            .await
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

        // Settle the gate ticket based on the acquire result. #322: this
        // makes the ticket ownership end-to-end — on success we `resolve`,
        // on retryable error we `fail_transient`, on permanent error we
        // `fail_permanent`. The `Drop` impl of `RecoveryTicket` covers
        // cancellation/panic paths.
        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        result.map(|h| h.with_drain_tracker(self.drain_tracker.clone()))
    }

    /// Acquires a resident resource handle without scheme material.
    ///
    /// Shorthand for [`acquire_resident`](Self::acquire_resident) with `scheme = &()`.
    /// Only available when `R::Credential = NoCredential`.
    pub async fn acquire_resident_default<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::resident::Resident<Credential = nebula_credential::NoCredential>
            + Send
            + Sync
            + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        self.acquire_resident::<R>(&(), ctx, options).await
    }

    /// Acquires a handle to a service resource.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   service topology.
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
        let started = Instant::now();
        let managed = self.lookup::<R>(&ctx.scope_level())?;
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

        // Settle the gate ticket based on the acquire result. #322: this
        // makes the ticket ownership end-to-end — on success we `resolve`,
        // on retryable error we `fail_transient`, on permanent error we
        // `fail_permanent`. The `Drop` impl of `RecoveryTicket` covers
        // cancellation/panic paths.
        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        result.map(|h| h.with_drain_tracker(self.drain_tracker.clone()))
    }

    /// Acquires a service resource handle without scheme material.
    ///
    /// Shorthand for [`acquire_service`](Self::acquire_service); only available
    /// when `R::Credential = NoCredential`.
    pub async fn acquire_service_default<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::service::Service<Credential = nebula_credential::NoCredential>
            + Clone
            + Send
            + Sync
            + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        self.acquire_service::<R>(ctx, options).await
    }

    /// Acquires a handle to a transport resource.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   transport topology.
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
        let started = Instant::now();
        let managed = self.lookup::<R>(&ctx.scope_level())?;
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

        // Settle the gate ticket based on the acquire result. #322: this
        // makes the ticket ownership end-to-end — on success we `resolve`,
        // on retryable error we `fail_transient`, on permanent error we
        // `fail_permanent`. The `Drop` impl of `RecoveryTicket` covers
        // cancellation/panic paths.
        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        result.map(|h| h.with_drain_tracker(self.drain_tracker.clone()))
    }

    /// Acquires a transport resource handle without scheme material.
    ///
    /// Shorthand for [`acquire_transport`](Self::acquire_transport); only available
    /// when `R::Credential = NoCredential`.
    pub async fn acquire_transport_default<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::transport::Transport<Credential = nebula_credential::NoCredential>
            + Clone
            + Send
            + Sync
            + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        self.acquire_transport::<R>(ctx, options).await
    }

    /// Acquires a handle to an exclusive resource.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   exclusive topology.
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
        let started = Instant::now();
        let managed = self.lookup::<R>(&ctx.scope_level())?;
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

        // Settle the gate ticket based on the acquire result. #322: this
        // makes the ticket ownership end-to-end — on success we `resolve`,
        // on retryable error we `fail_transient`, on permanent error we
        // `fail_permanent`. The `Drop` impl of `RecoveryTicket` covers
        // cancellation/panic paths.
        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        result.map(|h| h.with_drain_tracker(self.drain_tracker.clone()))
    }

    /// Acquires an exclusive resource handle without scheme material.
    ///
    /// Shorthand for [`acquire_exclusive`](Self::acquire_exclusive); only available
    /// when `R::Credential = NoCredential`.
    pub async fn acquire_exclusive_default<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::exclusive::Exclusive<Credential = nebula_credential::NoCredential>
            + Clone
            + Send
            + Sync
            + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        self.acquire_exclusive::<R>(ctx, options).await
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
    pub async fn try_acquire_pooled<R>(
        &self,
        scheme: &<R::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let started = Instant::now();
        let managed = self.lookup::<R>(&ctx.scope_level())?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;

        let result = match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                let config = managed.config();
                let generation = managed.generation();
                rt.try_acquire(
                    &managed.resource,
                    &config,
                    scheme,
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

        // Settle the gate ticket based on the acquire result. #322: this
        // makes the ticket ownership end-to-end — on success we `resolve`,
        // on retryable error we `fail_transient`, on permanent error we
        // `fail_permanent`. The `Drop` impl of `RecoveryTicket` covers
        // cancellation/panic paths.
        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        result.map(|h| h.with_drain_tracker(self.drain_tracker.clone()))
    }

    /// Non-blocking pooled acquire without scheme material.
    ///
    /// Shorthand for [`try_acquire_pooled`](Self::try_acquire_pooled) with `scheme = &()`.
    /// Only available when `R::Credential = NoCredential`.
    pub async fn try_acquire_pooled_default<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled<Credential = nebula_credential::NoCredential>
            + Clone
            + Send
            + Sync
            + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        self.try_acquire_pooled::<R>(&(), ctx, options).await
    }

    /// Returns a snapshot of current pool utilization for a registered Pool resource.
    ///
    /// Returns `None` if the resource is not registered or does not use Pool topology.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_resource::{Manager, ScopeLevel};
    /// # async fn example(manager: &Manager) {
    /// // if let Some(stats) = manager.pool_stats::<MyDb>(&ScopeLevel::Global).await {
    /// //     println!("pool: {}/{} slots in use, {} idle", stats.in_use, stats.capacity, stats.idle);
    /// // }
    /// # }
    /// ```
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

    /// Warms up a registered Pool resource by pre-creating instances up to `min_size`.
    ///
    /// This fills the idle queue before production traffic hits, eliminating
    /// cold-start latency on the first batch of requests. Warmup follows the
    /// [`WarmupStrategy`](crate::topology::pooled::config::WarmupStrategy) set
    /// in the pool's configuration.
    ///
    /// Uses [`Default::default()`] for the projected scheme, which works for
    /// `R::Credential = NoCredential` (Scheme = `()`) and any scheme type that
    /// has a meaningful default.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   pool topology.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_resource::{Manager, ScopeLevel};
    /// # async fn example(manager: &Manager) {
    /// # let ctx = nebula_resource::context::ResourceContext::minimal(
    /// #     Default::default(),
    /// #     tokio_util::sync::CancellationToken::new(),
    /// # );
    /// // manager.warmup_pool::<MyDb>(&ctx).await.unwrap();
    /// # }
    /// ```
    pub async fn warmup_pool<R>(&self, ctx: &ResourceContext) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
        // INTERIM (П1): retains a Default bound on the projected Scheme so
        // `NoCredential` (Scheme = ()) keeps working. П2 replaces this with a
        // credential-bearing warmup signature per ADR-0036 §Decision +
        // security-lead amendment B-3 (no Scheme::default() in production
        // hot paths). TODO(П2): remove Default bound, accept Scheme borrow.
        <R::Credential as Credential>::Scheme: Default,
    {
        let managed = self.lookup::<R>(&ctx.scope_level())?;
        let config = managed.config();
        let scheme = <<R::Credential as Credential>::Scheme as Default>::default();
        match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                let count = rt.warmup(&managed.resource, &config, &scheme, ctx).await;
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
            TopologyRuntime::Daemon(_) => ReloadOutcome::Restarting,
            _ => ReloadOutcome::SwappedImmediately,
        };

        tracing::info!(key = %R::key(), ?outcome, "resource config reloaded");
        Ok(outcome)
    }

    /// Handle a credential refresh event by fanning out
    /// `Resource::on_credential_refresh` hooks to every resource bound to
    /// `credential_id`.
    ///
    /// Each per-resource future runs concurrently via `join_all` and has its
    /// own timeout budget — one slow or failing resource never poisons
    /// siblings (security amendment B-1 isolation). The per-resource budget
    /// is sourced from `RegisterOptions::credential_rotation_timeout` if set
    /// at register time, otherwise the manager's
    /// [`ManagerConfig::credential_rotation_timeout`] default.
    ///
    /// Returns a list of `(ResourceKey, RefreshOutcome)` pairs — one per
    /// affected resource. If no resources are bound to the credential the
    /// result is an empty `Vec`.
    ///
    /// # Caller contract
    ///
    /// The caller (typically the engine refresh coordinator) constructs a
    /// [`SchemeFactory<C>`](nebula_credential::SchemeFactory) over the
    /// freshly-projected post-refresh state. Per fan-out branch the manager
    /// clones the factory, type-erases it through a `Box<dyn Any>`, and
    /// passes it to the resource's typed dispatcher. The dispatcher
    /// downcasts back to `SchemeFactory<R::Credential>` and calls
    /// `factory.acquire().await` to mint a fresh
    /// [`SchemeGuard<'_, R::Credential>`](nebula_credential::SchemeGuard)
    /// inside its own typed scope before invoking the resource hook.
    ///
    /// **Why dispatcher-side acquire (not manager-side):** `Box<dyn Any>`
    /// is `'static`-bound (because `Any: 'static`), so a non-`'static`
    /// `SchemeGuard<'_, C>` cannot be type-erased through it. `SchemeFactory<C>`
    /// IS `'static`, so it can. The acquire call is the same in either
    /// design (one per dispatcher); only the call boundary moves.
    ///
    /// # Errors
    ///
    /// This call itself never returns `Err`; per-resource failures are
    /// reported in the returned `Vec` via [`RefreshOutcome::Failed`] and
    /// [`RefreshOutcome::TimedOut`]. The `Result` shape is preserved for
    /// forward-compat with future caller-level guards (e.g. shutdown-in-
    /// progress short-circuit).
    pub async fn on_credential_refreshed<C: Credential>(
        &self,
        credential_id: &CredentialId,
        factory: nebula_credential::SchemeFactory<C>,
        ctx: &nebula_credential::CredentialContext,
    ) -> Result<Vec<(ResourceKey, RefreshOutcome)>, Error> {
        use futures::future::join_all;

        let dispatchers = self
            .credential_resources
            .get(credential_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default();

        if dispatchers.is_empty() {
            return Ok(Vec::new());
        }

        use tracing::Instrument as _;

        let scheme_type_id = TypeId::of::<<C as Credential>::Scheme>();
        let default_timeout = self.credential_rotation_timeout;

        let span = tracing::info_span!(
            "resource.credential_refresh",
            credential_id = %credential_id,
            resources_affected = dispatchers.len(),
        );

        // Per-resource futures with isolation. Each future has its own
        // timeout budget; one slow or failing resource never poisons
        // siblings (security amendment B-1).
        let futures = dispatchers.into_iter().filter_map(|d| {
            // Defensive scheme-type check. The dispatcher's
            // `scheme_type_id()` should match the call's `C` if
            // `register_inner` was correct; if not, log and skip rather
            // than panic.
            if d.scheme_type_id() != scheme_type_id {
                tracing::error!(
                    resource = %d.resource_key(),
                    expected_scheme_type = ?scheme_type_id,
                    got_scheme_type = ?d.scheme_type_id(),
                    "dispatcher scheme_type_id mismatch — skipping (register_inner bug?)",
                );
                return None;
            }

            let timeout = d.timeout_override().unwrap_or(default_timeout);
            let key = d.resource_key();
            // Cheap Arc bump per dispatcher; the boxed factory becomes the
            // dispatcher's owned `SchemeFactory<R::Credential>` after
            // downcast.
            let factory_box: Box<dyn std::any::Any + Send + Sync> = Box::new(factory.clone());

            Some(async move {
                let dispatch = d.dispatch_refresh(factory_box, ctx);
                let outcome = match tokio::time::timeout(timeout, dispatch).await {
                    Ok(Ok(())) => RefreshOutcome::Ok,
                    Ok(Err(e)) => RefreshOutcome::Failed(e),
                    Err(_) => RefreshOutcome::TimedOut { budget: timeout },
                };
                (key, outcome)
            })
        });

        let results: Vec<(ResourceKey, RefreshOutcome)> = join_all(futures).instrument(span).await;

        // Task 7 wires aggregate event emission here.
        // Task 8 wires counter + histogram observation here.

        Ok(results)
    }

    /// Handle a credential revocation event by fanning out
    /// `Resource::on_credential_revoke` hooks to every resource bound to
    /// `credential_id`.
    ///
    /// Symmetric to [`on_credential_refreshed`](Self::on_credential_refreshed)
    /// minus the scheme-material plumbing — `on_credential_revoke` takes only
    /// `&CredentialId`, so no `SchemeFactory<C>` or generic `C: Credential`
    /// bound is required here.
    ///
    /// Each per-resource future runs concurrently via `join_all` and has its
    /// own timeout budget — one slow or failing resource never poisons
    /// siblings (security amendment B-1 isolation). The per-resource budget
    /// is sourced from `RegisterOptions::credential_rotation_timeout` if set
    /// at register time, otherwise the manager's
    /// [`ManagerConfig::credential_rotation_timeout`] default.
    ///
    /// Per security amendment B-2, every non-`Ok` per-resource outcome
    /// (`Failed` or `TimedOut`) emits a
    /// [`ResourceEvent::HealthChanged`] event with `healthy: false` inline
    /// so the operator sees a per-resource failure signal even if the
    /// aggregate `CredentialRevoked` event (Task 7) is dropped by a
    /// saturated subscriber. Successful revocations emit only the aggregate
    /// event.
    ///
    /// Returns a list of `(ResourceKey, RevokeOutcome)` pairs — one per
    /// affected resource. If no resources are bound to the credential the
    /// result is an empty `Vec`.
    ///
    /// # Errors
    ///
    /// This call itself never returns `Err`; per-resource failures are
    /// reported in the returned `Vec` via [`RevokeOutcome::Failed`] and
    /// [`RevokeOutcome::TimedOut`]. The `Result` shape is preserved for
    /// forward-compat with future caller-level guards (e.g. shutdown-in-
    /// progress short-circuit).
    pub async fn on_credential_revoked(
        &self,
        credential_id: &CredentialId,
    ) -> Result<Vec<(ResourceKey, RevokeOutcome)>, Error> {
        use futures::future::join_all;

        let dispatchers = self
            .credential_resources
            .get(credential_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default();

        if dispatchers.is_empty() {
            return Ok(Vec::new());
        }

        use tracing::Instrument as _;

        let default_timeout = self.credential_rotation_timeout;
        let event_tx = self.event_tx.clone();

        let span = tracing::warn_span!(
            "resource.credential_revoke",
            credential_id = %credential_id,
            resources_affected = dispatchers.len(),
        );

        // Per-resource futures with isolation. Each future has its own
        // timeout budget; one slow or failing resource never poisons
        // siblings (security amendment B-1). Per security amendment B-2:
        // emit HealthChanged{healthy:false} inline for any non-Ok outcome
        // so the operator sees a per-resource failure signal even if the
        // aggregate event is lost.
        let futures = dispatchers.into_iter().map(|d| {
            let timeout = d.timeout_override().unwrap_or(default_timeout);
            let key = d.resource_key();
            let credential_id = *credential_id;
            let event_tx = event_tx.clone();

            async move {
                let dispatch = d.dispatch_revoke(&credential_id);
                let outcome = match tokio::time::timeout(timeout, dispatch).await {
                    Ok(Ok(())) => RevokeOutcome::Ok,
                    Ok(Err(e)) => RevokeOutcome::Failed(e),
                    Err(_) => RevokeOutcome::TimedOut { budget: timeout },
                };

                // Security amendment B-2: emit HealthChanged{healthy:false}
                // for non-Ok outcomes. Successful revocations emit only the
                // aggregate CredentialRevoked event (Task 7). Broadcast send
                // errors (no live subscribers) are intentionally ignored.
                if !matches!(outcome, RevokeOutcome::Ok) {
                    let _ = event_tx.send(ResourceEvent::HealthChanged {
                        key: key.clone(),
                        healthy: false,
                    });
                }

                (key, outcome)
            }
        });

        let results: Vec<(ResourceKey, RevokeOutcome)> = join_all(futures).instrument(span).await;

        // Task 7 wires aggregate CredentialRevoked event emission here.
        // Task 8 wires counter + histogram observation here.

        Ok(results)
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

    /// Triggers graceful shutdown with drain and cleanup.
    ///
    /// 1. **Signal** — cancels the token so new acquires are rejected.
    /// 2. **Drain** — waits up to [`ShutdownConfig::drain_timeout`] for in-flight handles to be
    ///    released.
    /// 3. **Clear** — drops all managed resources, releasing their `Arc<ReleaseQueue>` references
    ///    so workers can drain and exit.
    /// 4. **Await workers** — waits for the release queue workers to finish processing remaining
    ///    tasks.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_resource::manager::{Manager, ShutdownConfig};
    /// # use std::time::Duration;
    /// # async fn example() {
    /// let manager = Manager::new();
    /// manager
    ///     .graceful_shutdown(ShutdownConfig::default().with_drain_timeout(Duration::from_secs(5)))
    ///     .await
    ///     .expect("graceful shutdown should succeed");
    /// # }
    /// ```
    pub async fn graceful_shutdown(
        &self,
        config: ShutdownConfig,
    ) -> Result<ShutdownReport, ShutdownError> {
        // CAS idempotency guard: exactly one caller wins. Concurrent callers
        // that arrive after this CAS see `AlreadyShuttingDown` immediately
        // rather than re-entering the drain logic against a half-torn state.
        if self
            .shutting_down
            .compare_exchange(false, true, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
            .is_err()
        {
            return Err(ShutdownError::AlreadyShuttingDown);
        }

        tracing::info!("resource manager: starting graceful shutdown");

        // Phase 1: SIGNAL — cancel the shared token. This does two things:
        //   a) Rejects new acquire calls (checked in `lookup`).
        //   b) Tells release queue workers to drain remaining tasks and exit
        //      (they share this token via `ReleaseQueue::with_cancel`).
        self.cancel.cancel();

        // #387: mark every registered resource as `Draining` so operators
        // polling `health_check` during the drain window see the correct
        // lifecycle phase instead of a stale `Ready`.
        self.set_phase_all(crate::state::ResourcePhase::Draining);

        // Phase 2: DRAIN — wait for in-flight handles to be released.
        // On timeout, respect the policy: Abort preserves "graceful"
        // (returns Err *without* clearing the registry), Force proceeds
        // but records the outstanding count in the report.
        let mut outstanding_after_drain: u64 = 0;
        match self.wait_for_drain(config.drain_timeout).await {
            Ok(()) => {},
            Err(DrainTimeoutError { outstanding }) => match config.on_drain_timeout {
                DrainTimeoutPolicy::Abort => {
                    tracing::warn!(
                        outstanding,
                        "resource manager: drain timeout, policy=Abort — \
                         registry preserved, returning DrainTimeout"
                    );
                    // #387 / PR #399 review: we already flipped every
                    // resource to `Draining` above. The Abort policy
                    // preserves the "graceful" guarantee and keeps live
                    // handles valid, so we must also restore the phase
                    // back to `Ready` — otherwise `is_accepting()` would
                    // falsely keep returning `false` and the manager
                    // would reject new acquires forever.
                    self.set_phase_all(crate::state::ResourcePhase::Ready);
                    self.shutting_down.store(false, AtomicOrdering::Release);
                    return Err(ShutdownError::DrainTimeout { outstanding });
                },
                DrainTimeoutPolicy::Force => {
                    tracing::warn!(
                        outstanding,
                        "resource manager: drain timeout, policy=Force — \
                         clearing registry anyway"
                    );
                    outstanding_after_drain = outstanding;
                },
            },
        }

        // #387: drain has completed (or been force-released). Mark every
        // resource as `ShuttingDown` so a health snapshot captured in the
        // narrow window between here and `registry.clear()` reflects the
        // real lifecycle state.
        self.set_phase_all(crate::state::ResourcePhase::ShuttingDown);

        // Phase 3: CLEAR — drop all ManagedResources so their
        // Arc<ReleaseQueue> refs are released.
        self.registry.clear();

        // Phase 4: AWAIT WORKERS — workers are already draining (from
        // Phase 1 cancel signal). Await with a bounded timeout; failure
        // to finish in time is a typed error, not a swallowed warning.
        if let Some(handle) = self.release_queue_handle.lock().await.take() {
            let shutdown_fut = ReleaseQueue::shutdown(handle);
            if tokio::time::timeout(config.release_queue_timeout, shutdown_fut)
                .await
                .is_err()
            {
                tracing::warn!(
                    timeout = ?config.release_queue_timeout,
                    "resource manager: release queue workers did not \
                     finish within release_queue_timeout"
                );
                return Err(ShutdownError::ReleaseQueueTimeout {
                    timeout: config.release_queue_timeout,
                });
            }
        }

        tracing::info!("resource manager: shutdown complete");
        Ok(ShutdownReport {
            outstanding_handles_after_drain: outstanding_after_drain,
            registry_cleared: true,
            // If we reached this line Phase 4 either succeeded or had no
            // work to drain — either way the contract is "drained".
            release_queue_drained: true,
        })
    }

    /// Drives every registered resource to the given lifecycle phase.
    ///
    /// Type-erased bulk update used during graceful shutdown so that
    /// `health_check` returns the correct phase while the drain/cleanup
    /// is in flight (#387).
    fn set_phase_all(&self, phase: crate::state::ResourcePhase) {
        for managed in self.registry.all_managed() {
            managed.set_phase_erased(phase);
        }
    }

    /// Waits until all active `ResourceGuard`s are dropped or timeout expires.
    ///
    /// The loop uses a `register-then-check` ordering to avoid the classic
    /// `Notify::notify_waiters` lost-wakeup:
    ///
    /// 1. Construct + pin + `enable()` a fresh `Notified` future. Calling `enable()` registers this
    ///    waiter on the `Notify` queue without requiring a `.await`, so any subsequent
    ///    `notify_waiters()` (fired when a handle's `Drop` decrements the counter from 1 → 0) will
    ///    reach us.
    /// 2. Re-check the counter. If it already hit 0 between the outer initial check and our
    ///    registration, return now — the wakeup we would otherwise wait for has already been
    ///    consumed.
    /// 3. Only then await the `Notified` future.
    ///
    /// Without this ordering, a burst of handle drops that completes the
    /// drain *before* the first `notified().await` poll would leak the
    /// notification entirely, stalling `graceful_shutdown` for the full
    /// `drain_timeout` (default 30 s) and risking `SIGKILL` escalation
    /// under a tight orchestrator shutdown window.
    async fn wait_for_drain(&self, timeout: Duration) -> Result<(), DrainTimeoutError> {
        let active = self.drain_tracker.0.load(AtomicOrdering::Acquire);
        if active == 0 {
            return Ok(());
        }

        tracing::debug!(active_handles = active, "waiting for handles to drain");
        let tracker = &self.drain_tracker;
        let drained = tokio::time::timeout(timeout, async {
            loop {
                // Pre-register this waiter BEFORE re-checking the counter.
                let notified = tracker.1.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();

                // Re-check after registration. If the last handle dropped
                // while we were between the outer check and `enable()`,
                // the counter is now 0 and we would otherwise wait on a
                // notification that has already fired.
                if tracker.0.load(AtomicOrdering::Acquire) == 0 {
                    return;
                }

                notified.await;

                if tracker.0.load(AtomicOrdering::Acquire) == 0 {
                    return;
                }
            }
        })
        .await;

        if drained.is_err() {
            let outstanding = tracker.0.load(AtomicOrdering::Acquire);
            tracing::warn!(
                outstanding,
                "resource manager: drain timeout expired with handles still active"
            );
            return Err(DrainTimeoutError { outstanding });
        }
        Ok(())
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
    pub fn get_any(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
    ) -> Option<Arc<dyn crate::registry::AnyManagedResource>> {
        self.registry.get(key, scope)
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
}

/// Outcome of the pre-acquire gate admission check (#322).
///
/// The old `check_recovery_gate` → `trigger_recovery_on_failure` split was
/// a stampede hazard: after the backoff expired, every caller's `state()`
/// read returned the same snapshot and all of them proceeded through to
/// hit the dead backend. The gate was only advanced to `InProgress` *after*
/// failure, in `trigger_recovery_on_failure`, which is too late.
///
/// This enum pushes the CAS-based single-probe claim into the pre-acquire
/// check: either the caller is granted a ticket (`Probe`) that must be
/// resolved end-to-end with the acquire result, or the gate was idle/absent
/// (`Open`), or we got a typed error out directly.
enum GateAdmission {
    /// No gate attached. Proceed normally; no ticket ownership.
    Open,
    /// Gate attached and currently healthy (`Idle`). Proceed without a
    /// ticket, but retain the gate so a retryable acquire error can mark it
    /// failed and open the backoff window for subsequent callers.
    OpenGated(Arc<RecoveryGate>),
    /// This caller has been granted the single recovery slot. The acquire
    /// **must** consume the ticket by calling `resolve()`, `fail_transient`,
    /// or `fail_permanent` based on the acquire result. Dropping it without
    /// resolution auto-fails to `GateState::Failed` via its `Drop` impl —
    /// so even a cancellation or panic in the acquire path is safe.
    Probe(RecoveryTicket),
}

/// Admits a caller through the optional recovery gate.
///
/// Healthy gates (`Idle`) admit immediately with no CAS so regular traffic
/// keeps full pool concurrency. Only callers entering while the gate is in a
/// retryable `Failed` state claim a probe ticket.
fn admit_through_gate(gate: &Option<Arc<RecoveryGate>>) -> Result<GateAdmission, Error> {
    let Some(gate) = gate else {
        return Ok(GateAdmission::Open);
    };

    match gate.state() {
        GateState::Idle => Ok(GateAdmission::OpenGated(Arc::clone(gate))),
        GateState::InProgress { .. } => Err(Error::transient(
            "backend recovery in progress, retry later",
        )),
        GateState::Failed { retry_at, .. } => {
            if Instant::now() < retry_at {
                let wait = retry_at.saturating_duration_since(Instant::now());
                return Err(Error::exhausted("backend recovering", Some(wait)));
            }
            match gate.try_begin() {
                Ok(ticket) => Ok(GateAdmission::Probe(ticket)),
                Err(TryBeginError::AlreadyInProgress(_waiter)) => Err(Error::transient(
                    "backend recovery in progress, retry later",
                )),
                Err(TryBeginError::RetryLater { retry_at }) => {
                    let wait = retry_at.saturating_duration_since(Instant::now());
                    Err(Error::exhausted("backend recovering", Some(wait)))
                },
                Err(TryBeginError::PermanentlyFailed { message }) => Err(Error::permanent(message)),
            }
        },
        GateState::PermanentlyFailed { message } => Err(Error::permanent(message)),
    }
}

/// Resolves the ticket granted by [`admit_through_gate`] based on the
/// acquire result. No-op when the admission was [`GateAdmission::Open`]
/// (no gate attached), so callers can always call this unconditionally.
fn settle_gate_admission<T>(admission: GateAdmission, result: &Result<T, Error>) {
    match (admission, result) {
        (GateAdmission::Probe(ticket), Ok(_)) => ticket.resolve(),
        (GateAdmission::Probe(ticket), Err(e)) if e.is_retryable() => {
            ticket.fail_transient(e.to_string());
        },
        (GateAdmission::Probe(ticket), Err(_e)) => {
            // Non-retryable errors are not backend-health signals; keep the
            // gate open to avoid permanently bricking acquires.
            ticket.resolve();
        },
        (GateAdmission::OpenGated(gate), Err(e)) if e.is_retryable() => {
            // First retryable failure on healthy path opens the backoff gate.
            if let Ok(ticket) = gate.try_begin() {
                ticket.fail_transient(e.to_string());
            }
        },
        (GateAdmission::OpenGated(_) | GateAdmission::Open, _) => {},
    }
}

/// Executes an async operation with optional timeout and retry from
/// [`AcquireResilience`] configuration.
///
/// Delegates to [`nebula_resilience::retry_with`] which handles exponential
/// backoff, wall-clock budget, retry-after hints, and `Classify`-based
/// error filtering automatically.
async fn execute_with_resilience<F, Fut, T>(
    resilience: &Option<AcquireResilience>,
    mut operation: F,
) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Error>> + Send,
{
    let Some(config) = resilience else {
        return operation().await;
    };

    // #383: `to_retry_config` returns `None` only if the underlying
    // `RetryConfig::new` rejects the clamped attempt count. That path is
    // unreachable today; if it ever fires we prefer to fall through to a
    // single un-retried attempt rather than panic the manager.
    let Some(retry_cfg) = config.to_retry_config() else {
        return operation().await;
    };
    nebula_resilience::retry_with(retry_cfg, operation)
        .await
        .map_err(|call_err| match call_err {
            nebula_resilience::CallError::Operation(e)
            | nebula_resilience::CallError::RetriesExhausted { last: e, .. } => e,
            nebula_resilience::CallError::Timeout(d) => {
                Error::transient(format!("acquire timed out after {d:?}"))
            },
            other => Error::transient(other.to_string()),
        })
}

/// Validates pool config invariants at registration time.
///
/// Catches obviously broken configs (`max_size == 0`, `min_size > max_size`)
/// before they reach [`PoolRuntime`], so warmup never inflates beyond
/// `max_size` and callers cannot deadlock on an empty semaphore (#390).
fn validate_pool_config(cfg: &crate::topology::pooled::config::Config) -> Result<(), Error> {
    if cfg.max_size == 0 {
        return Err(Error::permanent("pool max_size must be > 0"));
    }
    if cfg.min_size > cfg.max_size {
        return Err(Error::permanent(format!(
            "pool min_size ({}) must be <= max_size ({})",
            cfg.min_size, cfg.max_size,
        )));
    }
    Ok(())
}

impl Default for Manager {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Manager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Manager")
            .field("registered_count", &self.registry.keys().len())
            .field("is_shutdown", &self.is_shutdown())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod gate_admission_tests {
    use super::*;
    use crate::recovery::gate::RecoveryGateConfig;

    /// #322: after `Failed { retry_at = past }`, concurrent callers must
    /// see **exactly one** `Probe` ticket, not a stampede. The CAS-based
    /// single-probe claim lives in `admit_through_gate`. Each spawned
    /// task parks on a `Barrier` before calling so the 32 attempts
    /// really contend, instead of being serviced one at a time.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn expired_failed_state_admits_only_one_probe() {
        let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig {
            max_attempts: 16,
            base_backoff: Duration::from_millis(5),
        }));

        // Drive the gate into Failed { retry_at ≈ past }.
        let ticket = gate.try_begin().expect("first ticket");
        ticket.fail_transient("seed");
        tokio::time::sleep(Duration::from_millis(20)).await;

        async fn contend(
            gate: Arc<RecoveryGate>,
            barrier: Arc<tokio::sync::Barrier>,
        ) -> (u32, u32) {
            // Park here until every task is ready so we really stress
            // the CAS claim.
            barrier.wait().await;
            let some_gate: Option<Arc<RecoveryGate>> = Some(gate);
            match admit_through_gate(&some_gate) {
                Ok(GateAdmission::Probe(ticket)) => {
                    // Hold the probe until the test is done counting so
                    // a second caller can't race in after a fast
                    // resolve/fail cycle.
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    drop(ticket);
                    (1, 0)
                },
                Ok(GateAdmission::Open | GateAdmission::OpenGated(_)) => (0, 1),
                Err(_) => (0, 1),
            }
        }

        let barrier = Arc::new(tokio::sync::Barrier::new(32));
        let mut handles = Vec::with_capacity(32);
        for _ in 0..32 {
            handles.push(tokio::spawn(contend(
                Arc::clone(&gate),
                Arc::clone(&barrier),
            )));
        }

        let mut probes = 0u32;
        let mut blocked = 0u32;
        for h in handles {
            let (p, b) = h.await.expect("admission task");
            probes += p;
            blocked += b;
        }

        assert_eq!(probes, 1, "exactly one Probe ticket must be granted (#322)");
        assert_eq!(
            probes + blocked,
            32,
            "every call must be accounted for: probes={probes}, blocked={blocked}",
        );
    }
}

#[cfg(test)]
mod drain_race_tests {
    use super::*;

    /// Regression for the drain-race bug: previously `wait_for_drain`
    /// did `tracker.1.notified().await` without pre-registering the
    /// `Notified` future, so a handle dropping (and firing
    /// `notify_waiters()`) in the window between the outer
    /// `active == 0` check and the first `notified().await` poll would
    /// leak the wakeup. Stall persisted until the full `drain_timeout`
    /// elapsed.
    ///
    /// The fix pre-enables the `Notified` future and re-checks the
    /// counter *after* registration, so a drop that completes the drain
    /// mid-race is observed on the re-check and returns immediately.
    ///
    /// This test exercises the normal "handle drops while we're waiting"
    /// path and asserts we return far sooner than the timeout.
    #[tokio::test]
    async fn wait_for_drain_returns_promptly_when_handle_drops() {
        let mgr = Manager::new();
        // Simulate one active handle.
        mgr.drain_tracker.0.fetch_add(1, AtomicOrdering::Release);

        let tracker = mgr.drain_tracker.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            if tracker.0.fetch_sub(1, AtomicOrdering::Release) == 1 {
                tracker.1.notify_waiters();
            }
        });

        let start = Instant::now();
        mgr.wait_for_drain(Duration::from_secs(30))
            .await
            .expect("handle drop must drain under the timeout");
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_secs(1),
            "wait_for_drain should return within 1s when a handle drops, took {elapsed:?}"
        );
        assert_eq!(mgr.drain_tracker.0.load(AtomicOrdering::Acquire), 0);
    }

    /// Regression: if the counter reaches 0 *before* `wait_for_drain`
    /// gets to pre-register the `Notified`, the post-enable re-check
    /// must catch it and return immediately rather than stalling.
    ///
    /// We simulate the race by setting `active = 1` (so the outer
    /// early-return doesn't fire), then immediately decrementing to 0
    /// before `wait_for_drain` is polled.
    #[tokio::test]
    async fn wait_for_drain_catches_drop_via_recheck() {
        let mgr = Manager::new();
        mgr.drain_tracker.0.fetch_add(1, AtomicOrdering::Release);

        // Decrement + notify synchronously — the counter is 0 before
        // `wait_for_drain` is even called, but we want to prove that
        // even if the outer check observed `active == 1` and then
        // the counter hit 0 *between* that check and the inner enable,
        // the inner re-check would catch it.
        //
        // Simulated here by priming the state and then calling
        // wait_for_drain directly; the inner loop's re-check should
        // fire on the very first iteration because the counter is
        // already 0. The outer check is bypassed by the fetch_add
        // above leaving active == 1 until... wait, we need to
        // decrement BETWEEN the outer check and the inner enable.
        //
        // Easiest approximation: skip the outer early-return by
        // keeping active = 1 through the outer check, then decrement
        // via a spawned task that runs before wait_for_drain gets
        // scheduler time.
        let tracker = mgr.drain_tracker.clone();
        tokio::task::yield_now().await;
        let handle = tokio::spawn(async move {
            // Yield so that wait_for_drain's outer load sees active = 1,
            // then decrement before the inner poll happens.
            tokio::task::yield_now().await;
            if tracker.0.fetch_sub(1, AtomicOrdering::Release) == 1 {
                tracker.1.notify_waiters();
            }
        });

        let start = Instant::now();
        mgr.wait_for_drain(Duration::from_secs(30))
            .await
            .expect("recheck path must drain under the timeout");
        let elapsed = start.elapsed();
        handle.await.unwrap();

        assert!(
            elapsed < Duration::from_secs(1),
            "wait_for_drain must return promptly even under race, took {elapsed:?}"
        );
    }

    /// #302: Abort policy must return a typed `DrainTimeout` error and
    /// leave the registry untouched. Before the policy split
    /// `graceful_shutdown` would log a warning and proceed to
    /// `registry.clear()` anyway, turning a cooperative shutdown into a
    /// logical use-after-free.
    #[tokio::test]
    async fn graceful_shutdown_abort_policy_returns_drain_timeout_error() {
        let mgr = Manager::new();
        // Simulate an outstanding handle.
        mgr.drain_tracker.0.fetch_add(1, AtomicOrdering::Release);

        let cfg = ShutdownConfig::default()
            .with_drain_timeout(Duration::from_millis(50))
            .with_drain_timeout_policy(DrainTimeoutPolicy::Abort);

        let err = mgr
            .graceful_shutdown(cfg)
            .await
            .expect_err("Abort policy must surface drain timeout");
        match err {
            ShutdownError::DrainTimeout { outstanding } => {
                assert_eq!(outstanding, 1, "outstanding count mismatch");
            },
            other => panic!("wrong error variant: {other:?}"),
        }
    }

    /// #302: Force policy must clear the registry and report the
    /// outstanding-handle count in `ShutdownReport` so operators can see
    /// exactly how much in-flight work was abandoned.
    #[tokio::test]
    async fn graceful_shutdown_force_policy_clears_registry_with_outstanding_count() {
        let mgr = Manager::new();
        mgr.drain_tracker.0.fetch_add(2, AtomicOrdering::Release);

        let cfg = ShutdownConfig::default()
            .with_drain_timeout(Duration::from_millis(50))
            .with_drain_timeout_policy(DrainTimeoutPolicy::Force);

        let report = mgr
            .graceful_shutdown(cfg)
            .await
            .expect("Force policy must succeed");
        assert!(report.registry_cleared);
        assert_eq!(report.outstanding_handles_after_drain, 2);
    }
}
