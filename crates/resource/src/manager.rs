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

use std::any::TypeId;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, broadcast};
use tokio_util::sync::CancellationToken;

use nebula_core::ResourceKey;

use crate::ctx::{Ctx, ScopeLevel};
use crate::error::Error;
use crate::events::ResourceEvent;
use crate::integration::AcquireResilience;
use crate::metrics::ResourceMetrics;
use crate::options::AcquireOptions;
use crate::recovery::gate::{GateState, RecoveryGate};
use crate::recovery::group::RecoveryGroupRegistry;
use crate::registry::Registry;
use crate::release_queue::{ReleaseQueue, ReleaseQueueHandle};
use crate::resource::Resource;
use crate::runtime::TopologyRuntime;
use crate::runtime::managed::ManagedResource;

/// Snapshot of a resource's health and operational state.
#[derive(Debug, Clone)]
pub struct ResourceHealthSnapshot {
    /// The resource's unique key.
    pub key: ResourceKey,
    /// Current lifecycle phase.
    pub phase: crate::state::ResourcePhase,
    /// Recovery gate state (if a gate is attached).
    pub gate_state: Option<crate::recovery::gate::GateState>,
    /// Aggregate operation counters.
    pub metrics: crate::metrics::MetricsSnapshot,
    /// Config generation counter.
    pub generation: u64,
}

/// Configuration for graceful shutdown.
#[derive(Debug, Clone)]
pub struct ShutdownConfig {
    /// How long to wait for in-flight handles to be released.
    pub drain_timeout: Duration,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout: Duration::from_secs(30),
        }
    }
}

/// Configuration for the [`Manager`].
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    /// Number of background workers for the release queue.
    ///
    /// Defaults to 2.
    pub release_queue_workers: usize,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            release_queue_workers: 2,
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
    metrics: Arc<ResourceMetrics>,
    event_tx: broadcast::Sender<ResourceEvent>,
    release_queue: Arc<ReleaseQueue>,
    release_queue_handle: tokio::sync::Mutex<Option<ReleaseQueueHandle>>,
    /// Tracks active `ResourceHandle`s for drain-aware shutdown.
    drain_tracker: Arc<(AtomicU64, Notify)>,
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
        Self {
            registry: Registry::new(),
            recovery_groups: RecoveryGroupRegistry::new(),
            cancel,
            metrics: Arc::new(ResourceMetrics::new()),
            event_tx,
            release_queue: Arc::new(release_queue),
            release_queue_handle: tokio::sync::Mutex::new(Some(release_queue_handle)),
            drain_tracker: Arc::new((AtomicU64::new(0), Notify::new())),
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
    /// Returns an error if [`ResourceConfig::validate()`] fails on the
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

        let per_resource_metrics = Arc::new(ResourceMetrics::new());

        let managed = Arc::new(ManagedResource {
            resource,
            config: arc_swap::ArcSwap::from_pointee(config),
            topology,
            release_queue: Arc::clone(&self.release_queue),
            generation: std::sync::atomic::AtomicU64::new(0),
            status: arc_swap::ArcSwap::from_pointee(crate::state::ResourceStatus::new()),
            metrics: per_resource_metrics,
            resilience,
            recovery_gate,
        });

        let type_id = TypeId::of::<ManagedResource<R>>();
        self.registry
            .register(key.clone(), type_id, scope, managed.clone());

        self.metrics.record_create();
        managed.metrics.record_create();
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
    /// The pool fingerprint is initialized from
    /// [`ResourceConfig::fingerprint()`] on the provided config.
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
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no
    ///   resource of type `R` is registered for the given scope.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the
    ///   manager is shutting down.
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
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no
    ///   resource of type `R` is registered.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the
    ///   manager is shutting down.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the
    ///   resource is not using pool topology.
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
        check_recovery_gate(&managed.recovery_gate)?;
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
                            Arc::clone(&managed.metrics),
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

        if let Err(e) = &result {
            trigger_recovery_on_failure(&managed.recovery_gate, e);
        }
        self.record_acquire_result(&managed, &result, started);
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
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no
    ///   resource of type `R` is registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the
    ///   resource is not using resident topology.
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
        check_recovery_gate(&managed.recovery_gate)?;
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

        if let Err(e) = &result {
            trigger_recovery_on_failure(&managed.recovery_gate, e);
        }
        self.record_acquire_result(&managed, &result, started);
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
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no
    ///   resource of type `R` is registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the
    ///   resource is not using service topology.
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
        check_recovery_gate(&managed.recovery_gate)?;
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
                            Arc::clone(&managed.metrics),
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

        if let Err(e) = &result {
            trigger_recovery_on_failure(&managed.recovery_gate, e);
        }
        self.record_acquire_result(&managed, &result, started);
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
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no
    ///   resource of type `R` is registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the
    ///   resource is not using transport topology.
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
        check_recovery_gate(&managed.recovery_gate)?;
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
                            Arc::clone(&managed.metrics),
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

        if let Err(e) = &result {
            trigger_recovery_on_failure(&managed.recovery_gate, e);
        }
        self.record_acquire_result(&managed, &result, started);
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
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no
    ///   resource of type `R` is registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the
    ///   resource is not using exclusive topology.
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
        check_recovery_gate(&managed.recovery_gate)?;
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
                            Arc::clone(&managed.metrics),
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

        if let Err(e) = &result {
            trigger_recovery_on_failure(&managed.recovery_gate, e);
        }
        self.record_acquire_result(&managed, &result, started);
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
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is registered.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting down.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using pool topology.
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
        check_recovery_gate(&managed.recovery_gate)?;

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
                    Arc::clone(&managed.metrics),
                )
                .await
            }
            _ => Err(Error::permanent(format!(
                "{}: expected Pool topology for try_acquire, registered as {}",
                R::key(),
                managed.topology.tag()
            ))),
        };

        if let Err(e) = &result {
            trigger_recovery_on_failure(&managed.recovery_gate, e);
        }
        self.record_acquire_result(&managed, &result, started);
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
    pub async fn pool_stats<R>(
        &self,
        scope: &ScopeLevel,
    ) -> Option<crate::runtime::pool::PoolStats>
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
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using pool topology.
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
    pub async fn warmup_pool<R>(
        &self,
        ctx: &dyn Ctx,
    ) -> Result<usize, Error>
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
                let count = rt
                    .warmup(&managed.resource, &config, &auth, ctx)
                    .await;
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
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no
    ///   resource of type `R` is registered for the given scope.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if config
    ///   validation fails.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the
    ///   manager is shut down.
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
        self.metrics.record_destroy();
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
    /// 2. **Drain** — waits up to [`ShutdownConfig::drain_timeout`] for
    ///    in-flight handles to be released.
    /// 3. **Clear** — drops all managed resources, releasing their
    ///    `Arc<ReleaseQueue>` references so workers can drain and exit.
    /// 4. **Await workers** — waits for the release queue workers to
    ///    finish processing remaining tasks.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use nebula_resource::manager::{Manager, ShutdownConfig};
    /// # use std::time::Duration;
    /// # async fn example() {
    /// let manager = Manager::new();
    /// manager.graceful_shutdown(ShutdownConfig {
    ///     drain_timeout: Duration::from_secs(5),
    /// }).await;
    /// # }
    /// ```
    pub async fn graceful_shutdown(&self, config: ShutdownConfig) {
        tracing::info!("resource manager: starting graceful shutdown");

        // Phase 1: SIGNAL — cancel the shared token. This does two things:
        //   a) Rejects new acquire calls (checked in `lookup`).
        //   b) Tells release queue workers to drain remaining tasks and exit
        //      (they share this token via `ReleaseQueue::with_cancel`).
        self.cancel.cancel();

        // Phase 2: DRAIN — wait for in-flight handles to be released,
        // or timeout if they are not released in time.
        self.wait_for_drain(config.drain_timeout).await;

        // Phase 3: CLEAR — drop all ManagedResources so their
        // Arc<ReleaseQueue> refs are released.
        self.registry.clear();

        // Phase 4: AWAIT WORKERS — workers are already draining (from
        // Phase 1 cancel signal). Await with a bounded timeout in case
        // a release task is slow.
        if let Some(handle) = self.release_queue_handle.lock().await.take() {
            let shutdown_fut = ReleaseQueue::shutdown(handle);
            if tokio::time::timeout(Duration::from_secs(10), shutdown_fut)
                .await
                .is_err()
            {
                tracing::warn!(
                    "resource manager: release queue workers did not \
                     finish within 10s, continuing shutdown"
                );
            }
        }

        tracing::info!("resource manager: shutdown complete");
    }

    /// Waits until all active `ResourceHandle`s are dropped or timeout expires.
    async fn wait_for_drain(&self, timeout: Duration) {
        let active = self.drain_tracker.0.load(AtomicOrdering::Acquire);
        if active == 0 {
            return;
        }

        tracing::debug!(active_handles = active, "waiting for handles to drain");
        let tracker = &self.drain_tracker;
        let drained = tokio::time::timeout(timeout, async {
            loop {
                tracker.1.notified().await;
                if tracker.0.load(AtomicOrdering::Acquire) == 0 {
                    return;
                }
            }
        })
        .await;

        if drained.is_err() {
            tracing::warn!(
                active_handles = tracker.0.load(AtomicOrdering::Relaxed),
                "resource manager: drain timeout expired with handles still active"
            );
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

    /// Returns a reference to the recovery group registry.
    pub fn recovery_groups(&self) -> &RecoveryGroupRegistry {
        &self.recovery_groups
    }

    /// Returns a reference to the aggregate metrics counters.
    pub fn metrics(&self) -> &ResourceMetrics {
        &self.metrics
    }

    /// Returns per-resource metrics for the given key and scope.
    ///
    /// Returns `None` if no resource is registered under the given key
    /// and scope combination.
    pub fn resource_metrics(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
    ) -> Option<Arc<ResourceMetrics>> {
        let managed = self.registry.get(key, scope)?;
        Some(Arc::clone(managed.metrics()))
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
            metrics: managed.metrics.snapshot(),
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

    /// Records acquire success/failure in both per-resource and aggregate
    /// metrics, and emits the corresponding [`ResourceEvent`].
    fn record_acquire_result<R: Resource>(
        &self,
        managed: &ManagedResource<R>,
        result: &Result<crate::handle::ResourceHandle<R>, Error>,
        started: Instant,
    ) {
        match result {
            Ok(_) => {
                self.metrics.record_acquire();
                managed.metrics.record_acquire();
                let _ = self.event_tx.send(ResourceEvent::AcquireSuccess {
                    key: R::key(),
                    duration: started.elapsed(),
                });
            }
            Err(e) => {
                self.metrics.record_acquire_error();
                managed.metrics.record_acquire_error();
                let _ = self.event_tx.send(ResourceEvent::AcquireFailed {
                    key: R::key(),
                    error: e.to_string(),
                });
            }
        }
    }
}

/// Checks the recovery gate before acquire.
///
/// Returns `Ok(())` if the backend is presumed healthy or the backoff has
/// expired (allowing the caller to act as the probe). Returns an
/// appropriate error otherwise.
fn check_recovery_gate(gate: &Option<Arc<RecoveryGate>>) -> Result<(), Error> {
    let Some(gate) = gate else { return Ok(()) };

    match gate.state() {
        GateState::Idle => Ok(()),
        GateState::InProgress { .. } => Err(Error::transient(
            "backend recovery in progress, retry later",
        )),
        GateState::Failed { retry_at, .. } => {
            let now = Instant::now();
            if now < retry_at {
                let wait = retry_at - now;
                Err(Error::exhausted("backend recovering", Some(wait)))
            } else {
                // Backoff expired — allow through so this caller acts as probe.
                Ok(())
            }
        }
        GateState::PermanentlyFailed { message, .. } => Err(Error::permanent(message)),
    }
}

/// If the acquire result is a retryable error and a recovery gate is
/// present, passively trigger recovery so subsequent callers fast-fail
/// instead of independently hitting the dead backend.
fn trigger_recovery_on_failure(gate: &Option<Arc<RecoveryGate>>, error: &Error) {
    let Some(gate) = gate else { return };
    if !error.is_retryable() {
        return;
    }
    if let Ok(ticket) = gate.try_begin() {
        ticket.fail_transient(error.to_string());
    }
}

/// Executes an async operation with optional timeout and retry from
/// [`AcquireResilience`] configuration.
///
/// When `resilience` is `None`, the operation runs exactly once with no
/// timeout. When configured, transient failures are retried with
/// exponential backoff up to `max_attempts`.
///
/// The `timeout` in [`AcquireResilience`] is an **overall wall-clock deadline**
/// shared across all attempts and backoff sleeps. Each attempt receives only
/// the time remaining before the deadline, and backoff sleeps are capped to
/// the same remaining budget.
async fn execute_with_resilience<F, Fut, T>(
    resilience: &Option<AcquireResilience>,
    mut operation: F,
) -> Result<T, Error>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, Error>>,
{
    let Some(config) = resilience else {
        return operation().await;
    };

    let max_attempts = config.retry.as_ref().map_or(1, |r| r.max_attempts);
    let initial_backoff = config
        .retry
        .as_ref()
        .map_or(Duration::from_millis(100), |r| r.initial_backoff);
    let max_backoff = config
        .retry
        .as_ref()
        .map_or(Duration::from_secs(5), |r| r.max_backoff);

    // Compute the overall deadline once. All attempts and backoff sleeps share
    // this budget so the timeout is truly wall-clock bounded end-to-end.
    let deadline = config
        .timeout
        .map(|t| std::time::Instant::now() + t);

    let mut last_error = None;
    for attempt in 0..max_attempts {
        let result = if let Some(dl) = deadline {
            let remaining = dl
                .checked_duration_since(std::time::Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() {
                Err(Error::transient("acquire timed out"))
            } else {
                match tokio::time::timeout(remaining, operation()).await {
                    Ok(r) => r,
                    Err(_) => Err(Error::transient("acquire timed out")),
                }
            }
        } else {
            operation().await
        };

        match result {
            Ok(val) => return Ok(val),
            Err(e) if e.is_retryable() && attempt + 1 < max_attempts => {
                let backoff = std::cmp::min(
                    initial_backoff.saturating_mul(2u32.saturating_pow(attempt)),
                    max_backoff,
                );
                // Cap the backoff sleep to the remaining budget so we never
                // overshoot the overall deadline.
                let sleep_dur = if let Some(dl) = deadline {
                    let remaining = dl
                        .checked_duration_since(std::time::Instant::now())
                        .unwrap_or(Duration::ZERO);
                    backoff.min(remaining)
                } else {
                    backoff
                };
                tokio::time::sleep(sleep_dur).await;
                last_error = Some(e);
            }
            Err(e) => return Err(e),
        }
    }

    Err(last_error.unwrap_or_else(|| Error::transient("retry exhausted")))
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
