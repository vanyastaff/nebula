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

use tokio_util::sync::CancellationToken;

use nebula_core::ResourceKey;

use crate::ctx::{Ctx, ScopeLevel};
use crate::error::Error;
use crate::metrics::ResourceMetrics;
use crate::recovery::group::RecoveryGroupRegistry;
use crate::registry::Registry;
use crate::release_queue::ReleaseQueue;
use crate::resource::Resource;
use crate::runtime::TopologyRuntime;
use crate::runtime::managed::ManagedResource;

/// Central registry and lifecycle manager for all resources.
///
/// Thread-safe: all internal state is behind concurrent data structures.
/// Share via `Arc<Manager>` across tasks.
pub struct Manager {
    registry: Registry,
    recovery_groups: RecoveryGroupRegistry,
    cancel: CancellationToken,
    metrics: ResourceMetrics,
}

impl Manager {
    /// Creates a new empty manager.
    pub fn new() -> Self {
        Self {
            registry: Registry::new(),
            recovery_groups: RecoveryGroupRegistry::new(),
            cancel: CancellationToken::new(),
            metrics: ResourceMetrics::new(),
        }
    }

    /// Registers a resource with its config, credential, scope, topology,
    /// and release queue.
    ///
    /// The resource is wrapped in a [`ManagedResource`] and stored in the
    /// registry under `R::key()`. If a resource with the same key and scope
    /// is already registered, it is silently replaced.
    pub fn register<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        _credential: R::Credential,
        scope: ScopeLevel,
        topology: TopologyRuntime<R>,
        release_queue: Arc<ReleaseQueue>,
    ) -> Result<(), Error> {
        let key = R::key();

        let managed = Arc::new(ManagedResource {
            resource,
            config: arc_swap::ArcSwap::from_pointee(config),
            topology,
            release_queue,
            generation: std::sync::atomic::AtomicU64::new(0),
            status: arc_swap::ArcSwap::from_pointee(crate::state::ResourceStatus::new()),
        });

        let type_id = TypeId::of::<ManagedResource<R>>();
        self.registry.register(key.clone(), type_id, scope, managed);

        self.metrics.record_create();

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
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup::<R>(ctx.scope())?;
        let generation = managed.generation();
        let config = managed.config();

        let result = match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                rt.acquire(
                    &managed.resource,
                    &config,
                    credential,
                    ctx,
                    &managed.release_queue,
                    generation,
                )
                .await
            }
            _ => Err(Error::permanent(format!(
                "{}: expected pool topology",
                R::key()
            ))),
        };

        self.record_acquire_result(&result);
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
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        let managed = self.lookup::<R>(ctx.scope())?;
        let config = managed.config();

        let result = match &managed.topology {
            TopologyRuntime::Resident(rt) => {
                rt.acquire(&managed.resource, &config, credential, ctx)
                    .await
            }
            _ => Err(Error::permanent(format!(
                "{}: expected resident topology",
                R::key()
            ))),
        };

        self.record_acquire_result(&result);
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
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::service::Service + Clone + Send + Sync + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup::<R>(ctx.scope())?;
        let generation = managed.generation();

        let result = match &managed.topology {
            TopologyRuntime::Service(rt) => {
                rt.acquire(&managed.resource, ctx, &managed.release_queue, generation)
                    .await
            }
            _ => Err(Error::permanent(format!(
                "{}: expected service topology",
                R::key()
            ))),
        };

        self.record_acquire_result(&result);
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
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::transport::Transport + Clone + Send + Sync + 'static,
        R::Runtime: Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup::<R>(ctx.scope())?;
        let generation = managed.generation();

        let result = match &managed.topology {
            TopologyRuntime::Transport(rt) => {
                rt.acquire(&managed.resource, ctx, &managed.release_queue, generation)
                    .await
            }
            _ => Err(Error::permanent(format!(
                "{}: expected transport topology",
                R::key()
            ))),
        };

        self.record_acquire_result(&result);
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
    ) -> Result<crate::handle::ResourceHandle<R>, Error>
    where
        R: crate::topology::exclusive::Exclusive + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup::<R>(ctx.scope())?;
        let generation = managed.generation();

        let result = match &managed.topology {
            TopologyRuntime::Exclusive(rt) => {
                rt.acquire(&managed.resource, &managed.release_queue, generation)
                    .await
            }
            _ => Err(Error::permanent(format!(
                "{}: expected exclusive topology",
                R::key()
            ))),
        };

        self.record_acquire_result(&result);
        result
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
        tracing::debug!(%key, "resource removed");
        Ok(())
    }

    /// Triggers a graceful shutdown of all managed resources.
    ///
    /// Cancels the shared [`CancellationToken`], signaling all in-flight
    /// operations to stop. Callers should await pending work separately.
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

    /// Returns a reference to the metrics counters.
    pub fn metrics(&self) -> &ResourceMetrics {
        &self.metrics
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

    /// Records acquire success/failure in metrics.
    fn record_acquire_result<R: Resource>(
        &self,
        result: &Result<crate::handle::ResourceHandle<R>, Error>,
    ) {
        match result {
            Ok(_) => self.metrics.record_acquire(),
            Err(_) => self.metrics.record_acquire_error(),
        }
    }
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
