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

use nebula_core::ResourceKey;
use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;

use crate::{
    ctx::{Ctx, ScopeLevel},
    error::Error,
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
    pub gate_state: Option<crate::recovery::gate::GateState>,
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
    /// How many `ResourceHandle`s were still outstanding when the drain
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
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            release_queue_workers: 2,
            metrics_registry: None,
        }
    }
}

/// Extended options for resource registration.
///
/// Used with the `register_*_with` convenience methods to configure
/// resilience and recovery beyond the simple `register_*` defaults.
#[derive(Debug, Clone, Default)]
pub struct RegisterOptions {
    /// Scope level for the resource (default: `Global`).
    pub scope: ScopeLevel,
    /// Optional acquire resilience (timeout + retry + circuit breaker).
    pub resilience: Option<AcquireResilience>,
    /// Optional recovery gate for thundering-herd prevention.
    pub recovery_gate: Option<Arc<RecoveryGate>>,
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
    /// Tracks active `ResourceHandle`s for drain-aware shutdown.
    drain_tracker: Arc<(AtomicU64, Notify)>,
    /// CAS-guarded idempotency flag for `graceful_shutdown`. Flipped
    /// false → true by the winning caller; losers return
    /// [`ShutdownError::AlreadyShuttingDown`].
    shutting_down: AtomicBool,
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
        }
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
    /// # Errors
    ///
    /// Returns an error if config validation fails on the
    /// provided config.
    pub fn register<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        scope: ScopeLevel,
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
            generation: std::sync::atomic::AtomicU64::new(0),
            status: arc_swap::ArcSwap::from_pointee(crate::state::ResourceStatus::new()),
            resilience,
            recovery_gate,
        });

        let type_id = TypeId::of::<ManagedResource<R>>();
        self.registry
            .register(key.clone(), type_id, scope, managed.clone());

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
        R: Resource<Auth = ()>,
    {
        use crate::resource::ResourceConfig as _;

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
        R: Resource<Auth = ()>,
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
        R: Resource<Auth = ()>,
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
        R: Resource<Auth = ()>,
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
        R: Resource<Auth = ()>,
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
    /// [`RegisterOptions`] for scope, resilience, and recovery gate.
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
        R: Resource<Auth = ()>,
    {
        use crate::resource::ResourceConfig as _;
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
        )
    }

    /// Registers a resident resource with extended options.
    ///
    /// Like [`register_resident`](Self::register_resident) but accepts
    /// [`RegisterOptions`] for scope, resilience, and recovery gate.
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
        R: Resource<Auth = ()>,
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
        )
    }

    /// Registers a service resource with extended options.
    ///
    /// Like [`register_service`](Self::register_service) but accepts
    /// [`RegisterOptions`] for scope, resilience, and recovery gate.
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
        R: Resource<Auth = ()>,
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
        )
    }

    /// Registers a transport resource with extended options.
    ///
    /// Like [`register_transport`](Self::register_transport) but accepts
    /// [`RegisterOptions`] for scope, resilience, and recovery gate.
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
        R: Resource<Auth = ()>,
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
        )
    }

    /// Registers an exclusive resource with extended options.
    ///
    /// Like [`register_exclusive`](Self::register_exclusive) but accepts
    /// [`RegisterOptions`] for scope, resilience, and recovery gate.
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
        R: Resource<Auth = ()>,
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
        auth: &R::Auth,
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let started = Instant::now();
        let managed = self.lookup::<R>(ctx.scope())?;
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
                            auth,
                            ctx,
                            &managed.release_queue,
                            generation,
                            options,
                            self.metrics.clone(),
                        )
                        .await
                    }
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

    /// Acquires a pooled resource handle without auth.
    ///
    /// Shorthand for [`acquire_pooled`](Self::acquire_pooled) with `auth = &()`.
    /// Only available when `R::Auth = ()`.
    pub async fn acquire_pooled_default<R>(
        &self,
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::pooled::Pooled<Auth = ()> + Clone + Send + Sync + 'static,
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
        auth: &R::Auth,
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        let started = Instant::now();
        let managed = self.lookup::<R>(ctx.scope())?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;
        let resilience = managed.resilience.clone();

        let result = execute_with_resilience(&resilience, || {
            let config = managed.config();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Resident(rt) => {
                        rt.acquire(&managed.resource, &config, auth, ctx, options)
                            .await
                    }
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

    /// Acquires a resident resource handle without auth.
    ///
    /// Shorthand for [`acquire_resident`](Self::acquire_resident) with `auth = &()`.
    /// Only available when `R::Auth = ()`.
    pub async fn acquire_resident_default<R>(
        &self,
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::resident::Resident<Auth = ()> + Send + Sync + 'static,
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
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::service::Service + Clone + Send + Sync + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let started = Instant::now();
        let managed = self.lookup::<R>(ctx.scope())?;
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
                    }
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

    /// Acquires a service resource handle without auth.
    ///
    /// Shorthand for [`acquire_service`](Self::acquire_service) with `auth = &()`.
    pub async fn acquire_service_default<R>(
        &self,
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::service::Service<Auth = ()> + Clone + Send + Sync + 'static,
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
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::transport::Transport + Clone + Send + Sync + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let started = Instant::now();
        let managed = self.lookup::<R>(ctx.scope())?;
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
                    }
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

    /// Acquires a transport resource handle without auth.
    ///
    /// Shorthand for [`acquire_transport`](Self::acquire_transport) with `auth = &()`.
    pub async fn acquire_transport_default<R>(
        &self,
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::transport::Transport<Auth = ()> + Clone + Send + Sync + 'static,
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
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::exclusive::Exclusive + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let started = Instant::now();
        let managed = self.lookup::<R>(ctx.scope())?;
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
                    }
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

    /// Acquires an exclusive resource handle without auth.
    ///
    /// Shorthand for [`acquire_exclusive`](Self::acquire_exclusive) with `auth = &()`.
    pub async fn acquire_exclusive_default<R>(
        &self,
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::exclusive::Exclusive<Auth = ()> + Clone + Send + Sync + 'static,
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
        auth: &R::Auth,
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let started = Instant::now();
        let managed = self.lookup::<R>(ctx.scope())?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;

        let result = match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                let config = managed.config();
                let generation = managed.generation();
                rt.try_acquire(
                    &managed.resource,
                    &config,
                    auth,
                    ctx,
                    &managed.release_queue,
                    generation,
                    options,
                    self.metrics.clone(),
                )
                .await
            }
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

    /// Non-blocking pooled acquire without auth.
    ///
    /// Shorthand for [`try_acquire_pooled`](Self::try_acquire_pooled) with `auth = &()`.
    /// Only available when `R::Auth = ()`.
    pub async fn try_acquire_pooled_default<R>(
        &self,
        ctx: &dyn Ctx,
        options: &AcquireOptions,
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::pooled::Pooled<Auth = ()> + Clone + Send + Sync + 'static,
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
    /// Uses [`Default::default()`] for auth, which works for `R::Auth = ()` and
    /// any auth type that has a meaningful default.
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
    /// # let ctx = nebula_resource::BasicCtx::new(nebula_core::ExecutionId::new());
    /// // manager.warmup_pool::<MyDb>(&ctx).await.unwrap();
    /// # }
    /// ```
    pub async fn warmup_pool<R>(&self, ctx: &dyn Ctx) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
        R::Auth: Default,
    {
        let managed = self.lookup::<R>(ctx.scope())?;
        let config = managed.config();
        let auth = R::Auth::default();
        match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                let count = rt.warmup(&managed.resource, &config, &auth, ctx).await;
                Ok(count)
            }
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
    ) -> Result<(), Error> {
        use crate::resource::ResourceConfig as _;

        new_config.validate()?;

        let managed = self.lookup::<R>(scope)?;

        // Compute fingerprint before swap so we don't clone config.
        let new_fp = new_config.fingerprint();

        // Atomically swap the config.
        managed.config.store(Arc::new(new_config));

        // Update pool fingerprint so stale idle instances are evicted.
        if let TopologyRuntime::Pool(ref pool_rt) = managed.topology {
            pool_rt.set_fingerprint(new_fp);
        }

        // Bump generation — readers snapshot this to detect changes.
        managed
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::Release);

        let _ = self
            .event_tx
            .send(ResourceEvent::ConfigReloaded { key: R::key() });

        tracing::info!(key = %R::key(), "resource config reloaded");
        Ok(())
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

        // Phase 2: DRAIN — wait for in-flight handles to be released.
        // On timeout, respect the policy: Abort preserves "graceful"
        // (returns Err *without* clearing the registry), Force proceeds
        // but records the outstanding count in the report.
        let mut outstanding_after_drain: u64 = 0;
        match self.wait_for_drain(config.drain_timeout).await {
            Ok(()) => {}
            Err(DrainTimeoutError { outstanding }) => match config.on_drain_timeout {
                DrainTimeoutPolicy::Abort => {
                    tracing::warn!(
                        outstanding,
                        "resource manager: drain timeout, policy=Abort — \
                         registry preserved, returning DrainTimeout"
                    );
                    self.shutting_down.store(false, AtomicOrdering::Release);
                    return Err(ShutdownError::DrainTimeout { outstanding });
                }
                DrainTimeoutPolicy::Force => {
                    tracing::warn!(
                        outstanding,
                        "resource manager: drain timeout, policy=Force — \
                         clearing registry anyway"
                    );
                    outstanding_after_drain = outstanding;
                }
            },
        }

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

    /// Waits until all active `ResourceHandle`s are dropped or timeout expires.
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
            metrics: self.metrics.as_ref().map(|m| m.snapshot()),
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
        result: &Result<crate::handle::ResourceHandle<R>, Error>,
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
            }
            Err(e) => {
                if let Some(m) = &self.metrics {
                    m.record_acquire_error();
                }
                let _ = self.event_tx.send(ResourceEvent::AcquireFailed {
                    key: R::key(),
                    error: e.to_string(),
                });
            }
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
                }
                Err(TryBeginError::PermanentlyFailed { message }) => Err(Error::permanent(message)),
            }
        }
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
        }
        (GateAdmission::Probe(ticket), Err(_e)) => {
            // Non-retryable errors are not backend-health signals; keep the
            // gate open to avoid permanently bricking acquires.
            ticket.resolve();
        }
        (GateAdmission::OpenGated(gate), Err(e)) if e.is_retryable() => {
            // First retryable failure on healthy path opens the backoff gate.
            if let Ok(ticket) = gate.try_begin() {
                ticket.fail_transient(e.to_string());
            }
        }
        (GateAdmission::OpenGated(_), _) | (GateAdmission::Open, _) => {}
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
    Fut: std::future::Future<Output = Result<T, Error>> + Send,
{
    let Some(config) = resilience else {
        return operation().await;
    };

    let retry_cfg = config.to_retry_config();
    nebula_resilience::retry_with(retry_cfg, operation)
        .await
        .map_err(|call_err| match call_err {
            nebula_resilience::CallError::Operation(e)
            | nebula_resilience::CallError::RetriesExhausted { last: e, .. } => e,
            nebula_resilience::CallError::Timeout(d) => {
                Error::transient(format!("acquire timed out after {d:?}"))
            }
            other => Error::transient(other.to_string()),
        })
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
}
