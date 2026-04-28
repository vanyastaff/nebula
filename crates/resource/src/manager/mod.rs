//! Central resource manager — registration, acquire dispatch, and shutdown.
//!
//! [`Manager`] is the single entry point for the resource subsystem. It owns
//! the registry, recovery-group registry, and a [`CancellationToken`] for
//! coordinated shutdown.
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
//! - `registration` — `register_inner` private helper + reverse-index write
//! - `gate` — `GateAdmission` + `admit_through_gate` + `settle_gate_admission`
//! - `execute` — resilience pipeline + register-time pool config validation
//! - `rotation` — `ResourceDispatcher` trampoline + `on_credential_*` fan-out
//! - `shutdown` — `graceful_shutdown` + drain helpers + `set_phase_all*`

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
mod registration;
pub(crate) mod rotation;
mod shutdown;

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

/// Central registry and lifecycle manager for all resources.
///
/// Owns the [`ReleaseQueue`] internally — callers never need to create,
/// pass, or shut down the queue manually. The queue is drained during
/// [`graceful_shutdown`](Self::graceful_shutdown).
///
/// Thread-safe: all internal state is behind concurrent data structures.
/// Share via `Arc<Manager>` across tasks.
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
    /// Reverse index: credential_id → dispatchers for resources that bind to this credential.
    ///
    /// Populated at register time when `R::Credential != NoCredential`. Read by
    /// `Manager::on_credential_refreshed` / `_revoked` to fan out rotation hooks
    /// in parallel via `join_all` (per Tech Spec §3.2).
    ///
    /// `Arc<dyn ResourceDispatcher>` provides type-erased dispatch — see
    /// `crate::manager::rotation::TypedDispatcher<R>`.
    pub(super) credential_resources:
        dashmap::DashMap<CredentialId, Vec<Arc<dyn rotation::ResourceDispatcher>>>,
    /// Default per-resource timeout budget for credential rotation hooks.
    ///
    /// Sourced from [`ManagerConfig::credential_rotation_timeout`] at
    /// construction. Per-resource overrides flow through
    /// `TypedDispatcher::timeout_override()` (set at register time, Task 6).
    pub(super) credential_rotation_timeout: Duration,
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
    #[expect(
        clippy::too_many_arguments,
        reason = "consolidation into `RegisterOptions` deferred; the `_with` shorthand variants already use the consolidated form"
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

        // Validate credential-binding contract BEFORE registry mutation.
        // Without this ordering, a credential-bearing resource without a
        // `credential_id` would land in the registry first and only THEN
        // surface `Error::missing_credential_id` — leaving an orphan entry
        // that retries would see as "already registered" (CodeRabbit 🔴 #1).
        registration::validate_credential_binding::<R>(credential_id.as_ref())?;

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
        // Validation already passed in `validate_credential_binding` above,
        // so this path is infallible by construction.
        registration::write_reverse_index::<R>(
            self,
            Arc::clone(&managed),
            credential_id,
            credential_rotation_timeout,
        );

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
        // Defense A against the `graceful_shutdown` race: reject any acquire
        // that arrives after `graceful_shutdown` has flipped the flag, even
        // if the cancel token has not yet been observed (it is set the line
        // after on the same task — see `shutdown::graceful_shutdown` Phase 1).
        // Ordering: `graceful_shutdown` writes `shutting_down` with `AcqRel`,
        // we read with `Acquire`, so we synchronize-with that write and any
        // observation here implies the cancel will follow.
        if self.shutting_down.load(AtomicOrdering::Acquire) || self.cancel.is_cancelled() {
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
        // Defense B against the `graceful_shutdown` race: pre-count this
        // acquire from the moment `lookup()` succeeds. RAII decrements + notifies
        // on every failure / cancel / panic path; on success the slot is handed
        // off to the resulting `ResourceGuard` so the count is held continuously
        // until the guard drops.
        let in_flight = InFlightCounter::new(self.drain_tracker.clone());
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
        match result {
            Ok(h) => Ok(h.with_drain_tracker(in_flight.release_to_guard())),
            Err(e) => Err(e),
        }
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
        // Defense B against the `graceful_shutdown` race — see `acquire_pooled`.
        let in_flight = InFlightCounter::new(self.drain_tracker.clone());
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

        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        match result {
            Ok(h) => Ok(h.with_drain_tracker(in_flight.release_to_guard())),
            Err(e) => Err(e),
        }
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
        // Defense B against the `graceful_shutdown` race — see `acquire_pooled`.
        let in_flight = InFlightCounter::new(self.drain_tracker.clone());
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
        // Defense B against the `graceful_shutdown` race — see `acquire_pooled`.
        let in_flight = InFlightCounter::new(self.drain_tracker.clone());
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
        // Defense B against the `graceful_shutdown` race — see `acquire_pooled`.
        let in_flight = InFlightCounter::new(self.drain_tracker.clone());
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
        // Defense B against the `graceful_shutdown` race — see `acquire_pooled`.
        let in_flight = InFlightCounter::new(self.drain_tracker.clone());
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

        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        match result {
            Ok(h) => Ok(h.with_drain_tracker(in_flight.release_to_guard())),
            Err(e) => Err(e),
        }
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

    /// Pre-warms a registered Pool resource that opts out of credential binding.
    ///
    /// Restricted to `R: Resource<Credential = NoCredential>` — compile-time
    /// gate that makes the unit scheme (`Scheme = ()`) the only callable shape.
    /// Internally passes `&()` to the runtime's warmup path.
    ///
    /// Per Tech Spec §5.2 and security-lead amendment B-3: kept separate from
    /// the credential-bearing [`warmup_pool`](Self::warmup_pool) so the
    /// production hot path never calls `Scheme::default()`. The `NoCredential`
    /// bound is unfaultable at compile time — a credential-bearing resource
    /// cannot be pre-warmed via this method.
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
    /// // manager.warmup_pool_no_credential::<MyDb>(&ctx).await.unwrap();
    /// # }
    /// ```
    pub async fn warmup_pool_no_credential<R>(&self, ctx: &ResourceContext) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled<Credential = nebula_credential::NoCredential>
            + Clone
            + Send
            + Sync
            + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup::<R>(&ctx.scope_level())?;
        let config = managed.config();
        // `NoCredential::Scheme = ()` — pass the unit value directly. NO call
        // to `Scheme::default()` (security amendment B-3).
        let scheme = ();
        match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                let count = rt.warmup(&managed.resource, &config, &scheme, ctx).await;
                Ok(count)
            },
            _ => Err(Error::permanent(format!(
                "{}: warmup_pool_no_credential requires Pool topology, registered as {}",
                R::key(),
                managed.topology.tag()
            ))),
        }
    }

    /// Pre-warms a registered credential-bearing Pool resource.
    ///
    /// The caller resolves the credential first (e.g. via `CredentialAccessor`)
    /// and passes a borrowed scheme; the manager forwards it to `R::create` for
    /// each pre-warmed instance. The borrow does not outlive the call.
    ///
    /// Per Tech Spec §5.2 and security-lead amendment B-3: NO `Default` bound
    /// on `Scheme`, NO `Scheme::default()` call. Forces the caller to supply a
    /// real credential, eliminating the "warm with empty credential → 401
    /// storm on first acquire" footgun. Use
    /// [`warmup_pool_no_credential`](Self::warmup_pool_no_credential) for
    /// resources that opt out of credential binding (`Credential = NoCredential`).
    ///
    /// Caller flow:
    /// 1. Resolve credential → obtain `<R::Credential as Credential>::Scheme`.
    /// 2. Call `warmup_pool::<R>(&scheme, &ctx)`.
    /// 3. Manager threads the scheme through to `R::create` for each instance.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   pool topology.
    pub async fn warmup_pool<R>(
        &self,
        scheme: &<R::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup::<R>(&ctx.scope_level())?;
        let config = managed.config();
        match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                let count = rt.warmup(&managed.resource, &config, scheme, ctx).await;
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
    /// Also prunes any matching dispatchers from the credential rotation
    /// reverse-index so a future credential refresh / revoke does not fan out
    /// to a resource that no longer exists. We compare by `resource_key()` on
    /// each dispatcher because the original `credential_id` is not preserved
    /// outside the dispatcher trampoline at remove-time (CodeRabbit 🔴 #2).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if
    /// the key is not registered.
    pub fn remove(&self, key: &ResourceKey) -> Result<(), Error> {
        if !self.registry.remove(key) {
            return Err(Error::not_found(key));
        }

        // Prune reverse-index: drop dispatchers whose `resource_key()`
        // matches the removed key, then drop empty CredentialId buckets so
        // future lookups don't see hollow Vecs.
        self.credential_resources.iter_mut().for_each(|mut entry| {
            entry.value_mut().retain(|d| d.resource_key() != *key);
        });
        self.credential_resources
            .retain(|_id, dispatchers| !dispatchers.is_empty());

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

impl Default for Manager {
    fn default() -> Self {
        Self::new()
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
//
// The counter slot lifecycle is *exactly* one increment + one decrement
// across the (acquire, guard) pair, with no transient gaps:
//
// 1. `InFlightCounter::new` increments.
// 2. The acquire runs (any number of `await` points).
// 3a. Success — `release_to_guard()` returns the `Arc<(AtomicU64, Notify)>` and
//     suppresses the Drop decrement; the caller hands the slot to
//     `ResourceGuard::with_drain_tracker`, which decrements + notifies on
//     guard Drop. Net effect across the pair: +1 on enter, -1 on guard Drop.
// 3b. Failure / panic / cancel — Drop runs and decrements + notifies. Net
//     effect: +1 on enter, -1 on early return.
struct InFlightCounter {
    tracker: Arc<(AtomicU64, Notify)>,
    /// Set true by `release_to_guard` to skip the Drop decrement once the
    /// slot has been transferred.
    released: bool,
}

impl InFlightCounter {
    /// Increments the in-flight counter immediately.
    fn new(tracker: Arc<(AtomicU64, Notify)>) -> Self {
        // `AcqRel` matches the `Acquire` load in `wait_for_drain` so the
        // increment is visible to a concurrent `wait_for_drain` snapshot
        // before that snapshot can return `Ok(())`.
        tracker.0.fetch_add(1, AtomicOrdering::AcqRel);
        Self {
            tracker,
            released: false,
        }
    }

    /// Hands the counter slot off to the resulting `ResourceGuard`.
    ///
    /// The returned `Arc` is then passed to
    /// [`ResourceGuard::with_drain_tracker`] which assumes the increment has
    /// already happened. This method suppresses the Drop decrement so the
    /// counter is owned by exactly one entity at all times — no double
    /// counting, no transient gap.
    fn release_to_guard(mut self) -> Arc<(AtomicU64, Notify)> {
        self.released = true;
        self.tracker.clone()
    }
}

impl Drop for InFlightCounter {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        // The acquire failed / was cancelled / panicked. Decrement and notify
        // so `wait_for_drain` does not block forever on a phantom in-flight.
        if self.tracker.0.fetch_sub(1, AtomicOrdering::AcqRel) == 1 {
            self.tracker.1.notify_waiters();
        }
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
