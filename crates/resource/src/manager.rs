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
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
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
use crate::release_queue::ReleaseQueue;
use crate::resource::Resource;
use crate::runtime::TopologyRuntime;
use crate::runtime::managed::ManagedResource;

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

/// Central registry and lifecycle manager for all resources.
///
/// Thread-safe: all internal state is behind concurrent data structures.
/// Share via `Arc<Manager>` across tasks.
pub struct Manager {
    registry: Registry,
    recovery_groups: RecoveryGroupRegistry,
    cancel: CancellationToken,
    metrics: Arc<ResourceMetrics>,
    event_tx: broadcast::Sender<ResourceEvent>,
}

impl Manager {
    /// Creates a new empty manager.
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(256);
        Self {
            registry: Registry::new(),
            recovery_groups: RecoveryGroupRegistry::new(),
            cancel: CancellationToken::new(),
            metrics: Arc::new(ResourceMetrics::new()),
            event_tx,
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

    /// Registers a resource with its config, credential, scope, topology,
    /// release queue, optional resilience configuration, and optional
    /// recovery gate.
    ///
    /// The resource is wrapped in a [`ManagedResource`] and stored in the
    /// registry under `R::key()`. If a resource with the same key and scope
    /// is already registered, it is silently replaced.
    ///
    /// When `resilience` is `Some`, acquire calls are wrapped with
    /// timeout and retry logic from [`AcquireResilience`].
    ///
    /// When `recovery_gate` is `Some`, acquire calls check the gate before
    /// proceeding. If the backend is recovering or permanently failed,
    /// callers receive an immediate error instead of hitting the dead
    /// backend. On transient acquire failures the gate is passively
    /// triggered so subsequent callers fast-fail.
    // Reason: register is a constructor that needs all parameters upfront;
    // a builder would be overengineering for a single-call registration API.
    #[allow(clippy::too_many_arguments)]
    pub fn register<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        _credential: R::Credential,
        scope: ScopeLevel,
        topology: TopologyRuntime<R>,
        release_queue: Arc<ReleaseQueue>,
        resilience: Option<AcquireResilience>,
        recovery_gate: Option<Arc<RecoveryGate>>,
    ) -> Result<(), Error> {
        let key = R::key();

        let per_resource_metrics = Arc::new(ResourceMetrics::new());

        let managed = Arc::new(ManagedResource {
            resource,
            config: arc_swap::ArcSwap::from_pointee(config),
            topology,
            release_queue,
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
        credential: &R::Credential,
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
                            credential,
                            ctx,
                            &managed.release_queue,
                            generation,
                            options,
                            Arc::clone(&managed.metrics),
                        )
                        .await
                    }
                    _ => Err(Error::permanent(format!(
                        "{}: expected pool topology",
                        R::key()
                    ))),
                }
            }
        })
        .await;

        if let Err(e) = &result {
            trigger_recovery_on_failure(&managed.recovery_gate, e);
        }
        self.record_acquire_result(&managed, &result, started);
        result
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
        credential: &R::Credential,
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
                        rt.acquire(&managed.resource, &config, credential, ctx, options)
                            .await
                    }
                    _ => Err(Error::permanent(format!(
                        "{}: expected resident topology",
                        R::key()
                    ))),
                }
            }
        })
        .await;

        if let Err(e) = &result {
            trigger_recovery_on_failure(&managed.recovery_gate, e);
        }
        self.record_acquire_result(&managed, &result, started);
        result
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
                        "{}: expected service topology",
                        R::key()
                    ))),
                }
            }
        })
        .await;

        if let Err(e) = &result {
            trigger_recovery_on_failure(&managed.recovery_gate, e);
        }
        self.record_acquire_result(&managed, &result, started);
        result
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
                        "{}: expected transport topology",
                        R::key()
                    ))),
                }
            }
        })
        .await;

        if let Err(e) = &result {
            trigger_recovery_on_failure(&managed.recovery_gate, e);
        }
        self.record_acquire_result(&managed, &result, started);
        result
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
                        "{}: expected exclusive topology",
                        R::key()
                    ))),
                }
            }
        })
        .await;

        if let Err(e) = &result {
            trigger_recovery_on_failure(&managed.recovery_gate, e);
        }
        self.record_acquire_result(&managed, &result, started);
        result
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
    /// 3. **Cleanup** — logs any resources still registered.
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

        // Phase 1: SIGNAL — stop new acquires.
        self.cancel.cancel();

        // Phase 2: DRAIN — wait for in-flight handles.
        tokio::time::sleep(config.drain_timeout).await;

        // Phase 3: CLEANUP — log remaining state.
        let remaining = self.registry.keys();
        if !remaining.is_empty() {
            tracing::warn!(
                count = remaining.len(),
                "resource manager: shutdown complete with {} resources still registered",
                remaining.len()
            );
        }

        tracing::info!("resource manager: shutdown complete");
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
        ticket.fail_transient("acquire failed");
    }
}

/// Executes an async operation with optional timeout and retry from
/// [`AcquireResilience`] configuration.
///
/// When `resilience` is `None`, the operation runs exactly once with no
/// timeout. When configured, transient failures are retried with
/// exponential backoff up to `max_attempts`.
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

    let mut last_error = None;
    for attempt in 0..max_attempts {
        let result = if let Some(timeout) = config.timeout {
            match tokio::time::timeout(timeout, operation()).await {
                Ok(r) => r,
                Err(_) => Err(Error::transient("acquire timed out")),
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
                tokio::time::sleep(backoff).await;
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
            .finish()
    }
}
