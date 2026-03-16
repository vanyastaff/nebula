//! Central resource manager: pool registry, dependency ordering, and graceful shutdown.
//!
//! [`Manager`] is the single entry point for resource lifecycle operations at runtime.
//! It owns one [`Pool<R>`] per registered resource type, stored as `Arc<dyn AnyPool>`
//! behind an [`ArcSwap`] for lock-free reads on the hot acquire path.
//!
//! ## Key responsibilities
//!
//! - **Registration**: validate config, create pool, record metadata, wire credential
//!   handlers and dependency edges.
//! - **Acquisition**: scope-check, hook dispatch, pool acquire, telemetry wrapping.
//! - **Health and quarantine**: background [`HealthChecker`] updates per-resource
//!   [`HealthState`]; [`QuarantineManager`] isolates failing resources.
//! - **Shutdown**: phased drain (wait for in-flight guards) → cleanup → terminate,
//!   respecting the topological order of the dependency graph.
//! - **Hot-reload**: `reload_config` swaps pool config without dropping the manager.
//!
//! [`Pool<R>`]: crate::pool::Pool
//! [`ArcSwap`]: arc_swap::ArcSwap
//! [`HealthChecker`]: crate::health::HealthChecker
//! [`HealthState`]: crate::health::HealthState
//! [`QuarantineManager`]: crate::quarantine::QuarantineManager

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use dashmap::DashMap;
use moka::sync::Cache;
use rustc_hash::FxBuildHasher;
use smallvec::SmallVec;

use crate::autoscale::{AutoScalePolicy, AutoScaler};
use crate::context::Context;
use crate::error::{Error, Result};
use crate::events::{EventBus, QuarantineTrigger, ResourceEvent};
use crate::health::{HealthCheckConfig, HealthCheckable, HealthState};
use crate::hooks::{HookEvent, HookRegistry, HOOKS_INLINE};
use crate::instrumented::InstrumentedGuard;
use crate::manager_guard::ReleaseHookGuard;
use crate::manager_pool::{AnyPool, PoolEntry};
use crate::metadata::ResourceMetadata;
use crate::pool::{Pool, PoolConfig};
use crate::quarantine::{QuarantineConfig, QuarantineManager, QuarantineReason};
use crate::resource::Resource;
use crate::scope::{Scope, Strategy};
use nebula_core::ResourceKey;

// ---------------------------------------------------------------------------
// Re-exports — keep the public API surface on `crate::manager::*`
// ---------------------------------------------------------------------------

pub use crate::dependency_graph::DependencyGraph;
pub use crate::manager_guard::{AnyGuard, AnyGuardTrait, ResourceHandle, TypedResourceGuard};
pub use crate::manager_pool::TypedPool;

// ---------------------------------------------------------------------------
// ShutdownConfig
// ---------------------------------------------------------------------------

/// Configuration for phased graceful shutdown.
#[derive(Debug, Clone)]
pub struct ShutdownConfig {
    /// Maximum time to wait for in-flight acquisitions to complete.
    pub drain_timeout: Duration,
    /// Maximum time to allow cleanup callbacks per pool.
    pub cleanup_timeout: Duration,
    /// Maximum time for forceful termination after cleanup.
    pub terminate_timeout: Duration,
}

/// Lightweight pool dimensions snapshot for external observers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ResourcePoolStatus {
    /// Current number of checked-out instances.
    pub active: usize,
    /// Current number of idle instances.
    pub idle: usize,
    /// Configured maximum pool size.
    pub max_size: usize,
}

/// Aggregate status snapshot for a registered resource.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResourceStatus {
    /// Static metadata for API/UI discovery.
    pub metadata: ResourceMetadata,
    /// Current health state.
    pub health: HealthState,
    /// Current pool dimensions.
    pub pool: ResourcePoolStatus,
    /// Whether the resource is currently quarantined.
    pub quarantined: bool,
    /// Quarantine reason when quarantined.
    pub quarantine_reason: Option<String>,
    /// Registration scope.
    pub scope: Scope,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout: Duration::from_secs(30),
            cleanup_timeout: Duration::from_secs(10),
            terminate_timeout: Duration::from_secs(5),
        }
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

/// Central manager for resource pools and dependency ordering.
///
/// Pools are keyed by the resource's string ID (`Resource::id()`), allowing
/// multiple pools of the same resource type with different IDs.
pub struct Manager {
    /// Pools indexed by resource ID string, with associated scope.
    pools: ArcSwap<HashMap<String, PoolEntry>>,
    /// Dependency graph for initialization ordering.
    deps: parking_lot::RwLock<DependencyGraph>,
    /// Background health checker.
    health_checker: Arc<crate::health::HealthChecker>,
    /// Event bus for lifecycle events.
    event_bus: Arc<EventBus>,
    /// Quarantine manager for unhealthy resources.
    quarantine: Arc<QuarantineManager>,
    /// Per-resource health states (set externally or by health checker events).
    health_states: Cache<String, HealthState>,
    /// Per-resource metadata (from `Resource::metadata()` at registration).
    metadata: DashMap<String, ResourceMetadata, FxBuildHasher>,
    /// Hook registry for lifecycle hooks (Arc-wrapped so pools can share it).
    hooks: Arc<HookRegistry>,
    /// Per-resource auto-scalers (resource_id → JoinHandle).
    auto_scalers: DashMap<String, tokio::task::JoinHandle<()>, FxBuildHasher>,
    /// Default auto-scale policy applied to every pool at registration time.
    /// `None` means auto-scaling is off by default (enable per-resource via
    /// [`enable_autoscaling`](Self::enable_autoscaling)).
    default_autoscale_policy: Option<AutoScalePolicy>,
    /// TypeId-to-ResourceKey index for type-safe acquisition via `acquire_typed`.
    ///
    /// Populated at `register` time; consulted by `acquire_typed` so that
    /// TypeId (not a fragile type-name string) is the lookup key.
    type_index: DashMap<TypeId, ResourceKey, FxBuildHasher>,
}

// ---------------------------------------------------------------------------
// ManagerBuilder
// ---------------------------------------------------------------------------

/// Builder for constructing a [`Manager`] with custom configuration.
///
/// Replaces the combinatorial `with_*` constructors and allows adding
/// new options without API explosion.
///
/// # Example
///
/// ```rust,ignore
/// let manager = ManagerBuilder::new()
///     .health_config(HealthCheckConfig { failure_threshold: 5, ..Default::default() })
///     .event_bus(Arc::new(EventBus::new(2048)))
///     .quarantine_config(QuarantineConfig::default())
///     .build();
/// ```
#[derive(Default)]
pub struct ManagerBuilder {
    health_config: HealthCheckConfig,
    event_bus: Option<Arc<EventBus>>,
    quarantine_config: QuarantineConfig,
    default_autoscale_policy: Option<AutoScalePolicy>,
}

impl ManagerBuilder {
    /// Create a new builder with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a custom health check configuration.
    #[must_use]
    pub fn health_config(mut self, config: HealthCheckConfig) -> Self {
        self.health_config = config;
        self
    }

    /// Set a custom event bus.
    #[must_use]
    pub fn event_bus(mut self, event_bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Set a custom quarantine configuration.
    #[must_use]
    pub fn quarantine_config(mut self, config: QuarantineConfig) -> Self {
        self.quarantine_config = config;
        self
    }

    /// Set a default auto-scale policy applied to every pool at registration.
    ///
    /// When set, [`Manager::register`] / [`Manager::register_scoped`] will
    /// automatically call [`Manager::enable_autoscaling`] with this policy
    /// for each newly registered resource.
    ///
    /// Individual resources can still override via
    /// [`enable_autoscaling`](Manager::enable_autoscaling) after registration.
    #[must_use]
    pub fn default_autoscale_policy(mut self, policy: AutoScalePolicy) -> Self {
        self.default_autoscale_policy = Some(policy);
        self
    }

    /// Build the [`Manager`].
    ///
    /// Wires the `HealthChecker`'s threshold callback to automatically
    /// quarantine resources and update health states when consecutive
    /// failures exceed the configured threshold.
    #[must_use]
    pub fn build(self) -> Manager {
        let event_bus = self
            .event_bus
            .unwrap_or_else(|| Arc::new(EventBus::default()));
        let quarantine = Arc::new(QuarantineManager::new(self.quarantine_config));
        let health_states: Cache<String, HealthState> = Cache::builder()
            .max_capacity(1024)
            .time_to_idle(Duration::from_secs(300))
            .build();

        // Build the health checker with threshold callback wired to
        // quarantine + health_states + event_bus.
        let mut health_checker = crate::health::HealthChecker::with_event_bus(
            self.health_config,
            Arc::clone(&event_bus),
        );

        // Wire: when threshold exceeded → quarantine resource + set Unhealthy.
        {
            let q = Arc::clone(&quarantine);
            let hs = health_states.clone();
            let bus = Arc::clone(&event_bus);
            health_checker.set_threshold_callback(move |resource_id, consecutive_failures| {
                let previous_health = hs.get(resource_id).unwrap_or(HealthState::Unknown);

                let next_health = HealthState::Unhealthy {
                    reason: format!(
                        "Health check failed ({consecutive_failures} consecutive failures)"
                    ),
                    recoverable: true,
                };

                let newly_quarantined = q.quarantine(
                    resource_id,
                    QuarantineReason::HealthCheckFailed {
                        consecutive_failures,
                    },
                );
                if newly_quarantined {
                    if let Ok(key) = nebula_core::ResourceKey::try_from(resource_id) {
                        bus.emit(ResourceEvent::Quarantined {
                            resource_key: key,
                            reason: format!(
                                "health check failed ({consecutive_failures} consecutive)"
                            ),
                            trigger: QuarantineTrigger::HealthThresholdExceeded {
                                consecutive_failures,
                            },
                            from_health: previous_health.clone(),
                            to_health: next_health.clone(),
                        });
                    } else {
                        tracing::warn!(resource_id, "skipping quarantine event for invalid resource key");
                    }
                }
                hs.insert(resource_id.to_string(), next_health);
            });
        }

        Manager {
            pools: ArcSwap::from_pointee(HashMap::new()),
            deps: parking_lot::RwLock::new(DependencyGraph::default()),
            health_checker: Arc::new(health_checker),
            event_bus,
            quarantine,
            health_states,
            metadata: DashMap::with_hasher(FxBuildHasher::default()),
            hooks: Arc::new(HookRegistry::default()),
            auto_scalers: DashMap::with_hasher(FxBuildHasher::default()),
            default_autoscale_policy: self.default_autoscale_policy,
            type_index: DashMap::with_hasher(FxBuildHasher::default()),
        }
    }
}

impl Default for Manager {
    fn default() -> Self {
        ManagerBuilder::default().build()
    }
}

impl Manager {
    /// Create a new empty resource manager.
    ///
    /// For customization, prefer [`ManagerBuilder`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn pools_snapshot(&self) -> Arc<HashMap<String, PoolEntry>> {
        self.pools.load_full()
    }

    fn pool_get(&self, id: &str) -> Option<PoolEntry> {
        self.pools_snapshot().get(id).cloned()
    }

    fn pool_contains(&self, id: &str) -> bool {
        self.pools_snapshot().contains_key(id)
    }

    fn pool_insert(&self, id: String, entry: PoolEntry) -> Option<PoolEntry> {
        loop {
            let current = self.pools.load_full();
            let mut next = (*current).clone();
            let old = next.insert(id.clone(), entry.clone());
            let prev = self.pools.compare_and_swap(&current, Arc::new(next));
            if Arc::ptr_eq(&prev, &current) {
                return old;
            }
        }
    }

    fn pool_remove(&self, id: &str) -> Option<PoolEntry> {
        loop {
            let current = self.pools.load_full();
            if !current.contains_key(id) {
                return None;
            }
            let mut next = (*current).clone();
            let removed = next.remove(id);
            let prev = self.pools.compare_and_swap(&current, Arc::new(next));
            if Arc::ptr_eq(&prev, &current) {
                return removed;
            }
        }
    }

    fn pool_len(&self) -> usize {
        self.pools_snapshot().len()
    }

    fn pool_keys(&self) -> Vec<String> {
        self.pools_snapshot().keys().cloned().collect()
    }

    fn pool_entries(&self) -> Vec<(String, PoolEntry)> {
        self.pools_snapshot()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    fn pool_clear(&self) {
        self.pools.store(Arc::new(HashMap::new()));
    }

    /// Create a manager with a custom health check configuration.
    ///
    /// **Deprecated:** prefer `ManagerBuilder::new().health_config(c).build()`.
    #[must_use]
    pub fn with_health_config(config: crate::health::HealthCheckConfig) -> Self {
        ManagerBuilder::new().health_config(config).build()
    }

    /// Create a manager with a custom event bus.
    ///
    /// **Deprecated:** prefer `ManagerBuilder::new().event_bus(b).build()`.
    #[must_use]
    pub fn with_event_bus(event_bus: Arc<EventBus>) -> Self {
        ManagerBuilder::new().event_bus(event_bus).build()
    }

    /// Create a manager with a custom quarantine configuration.
    ///
    /// **Deprecated:** prefer `ManagerBuilder::new().quarantine_config(c).build()`.
    #[must_use]
    pub fn with_quarantine_config(quarantine_config: QuarantineConfig) -> Self {
        ManagerBuilder::new()
            .quarantine_config(quarantine_config)
            .build()
    }

    /// Create a manager with both a custom event bus and quarantine config.
    ///
    /// **Deprecated:** prefer [`ManagerBuilder`].
    #[must_use]
    pub fn with_event_bus_and_quarantine(
        event_bus: Arc<EventBus>,
        quarantine_config: QuarantineConfig,
    ) -> Self {
        ManagerBuilder::new()
            .event_bus(event_bus)
            .quarantine_config(quarantine_config)
            .build()
    }

    /// Register a resource with its config and pool settings under [`Scope::Global`].
    ///
    /// The pool is keyed by `resource.id()`. Registering a second resource
    /// with the same ID replaces the previous one (including its dependencies).
    pub fn register<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        pool_config: PoolConfig,
    ) -> Result<()>
    where
        R::Instance: Any,
    {
        self.register_scoped(resource, config, pool_config, Scope::Global)
    }

    /// Register a resource with its config, pool settings, and an explicit scope.
    pub fn register_scoped<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        pool_config: PoolConfig,
        scope: Scope,
    ) -> Result<()>
    where
        R::Instance: Any,
    {
        let meta = resource.metadata();
        let resource_key = meta.key.clone();
        let id = resource_key.to_string();

        // Create pool first -- if this fails, nothing is modified.
        // Pass event bus and hooks so the pool can fire Create/Cleanup hooks.
        let pool = Pool::with_hooks(
            resource,
            config,
            pool_config,
            Some(Arc::clone(&self.event_bus)),
            Some(Arc::clone(&self.hooks)),
        )?;

        // register_scoped carries no explicit dependencies; just clear any
        // stale edges left from a previous registration of the same key.
        self.deps.write().remove_all_for(&id);

        let typed_pool: Arc<TypedPool<R>> = Arc::new(TypedPool { pool });
        let any_pool: Arc<dyn AnyPool> = typed_pool.clone();
        let _ = self.pool_insert(
            id.clone(),
            PoolEntry {
                pool: any_pool,
                scope: scope.clone(),
                typed_handle: Some(typed_pool),
            },
        );

        // `ResourceMetadata` already carries the canonical `ResourceKey`,
        // so we use it as the single source of truth for events and autoscaling.
        self.metadata.insert(id.clone(), meta);

        // Record TypeId → ResourceKey so acquire_typed<R>() can look up by type.
        self.type_index.insert(TypeId::of::<R>(), resource_key.clone());

        self.event_bus.emit(ResourceEvent::Created {
            resource_key: resource_key.clone(),
            scope,
        });

        // Apply default auto-scale policy if configured.
        if let Some(policy) = &self.default_autoscale_policy {
            // Errors are non-fatal — the pool is registered regardless.
            let _ = self.enable_autoscaling(&resource_key, policy.clone());
        }

        tracing::debug!(resource_id = %id, "Registered resource");

        Ok(())
    }

    /// Acquire a resource instance by resource key.
    ///
    /// Returns an [`AnyGuard`] that provides `&dyn Any` access to the
    /// instance. When the guard is dropped, the instance is returned to
    /// the pool.
    ///
    /// Checks quarantine status, health state, and scope compatibility
    /// before delegating to the pool.
    pub async fn acquire(&self, resource_key: &ResourceKey, ctx: &Context) -> Result<AnyGuard> {
        let id: &str = &resource_key;
        // Check quarantine -- quarantined resources cannot be acquired.
        if self.quarantine.is_quarantined(id) {
            return Err(Error::Unavailable {
                resource_key: resource_key.clone(),
                reason: "Resource is quarantined".to_string(),
                retryable: true,
            });
        }

        // Check health state -- block on Unhealthy, warn on Degraded.
        if let Some(state) = self.health_states.get(id) {
            match &state {
                HealthState::Unhealthy { recoverable, .. } => {
                    self.event_bus.emit(ResourceEvent::Error {
                        resource_key: resource_key.clone(),
                        error: "Resource is unhealthy".to_string(),
                    });
                    return Err(Error::Unavailable {
                        resource_key: resource_key.clone(),
                        reason: "Resource is unhealthy".to_string(),
                        retryable: *recoverable,
                    });
                }
                HealthState::Degraded { reason, .. } => {
                    tracing::warn!(
                        resource_id = %id,
                        reason = reason.as_str(),
                        "Acquiring degraded resource"
                    );
                    let _ = reason;
                }
                HealthState::Healthy | HealthState::Unknown => {}
            }
        }

        // Clone the Arc and scope to release the DashMap shard lock before
        // awaiting the potentially long-running acquire_any().
        let (pool, resource_scope) = match self.pool_get(id) {
            Some(entry) => (Arc::clone(&entry.pool), entry.scope.clone()),
            None => {
                let err = Error::Unavailable {
                    resource_key: resource_key.clone(),
                    reason: "Resource not registered".to_string(),
                    retryable: false,
                };
                self.event_bus.emit(ResourceEvent::Error {
                    resource_key: resource_key.clone(),
                    error: err.to_string(),
                });
                return Err(err);
            }
        };

        // Validate scope: the resource scope must contain the caller's scope
        // under the Hierarchical strategy.
        if !Strategy::Hierarchical.is_compatible(&resource_scope, &ctx.scope) {
            return Err(Error::Unavailable {
                resource_key: resource_key.clone(),
                reason: format!(
                    "Scope mismatch: resource scope {} does not contain requested scope {}",
                    resource_scope, ctx.scope
                ),
                retryable: false,
            });
        }

        // Run before-hooks; if any hook cancels, abort the acquire.
        self.hooks.run_before(&HookEvent::Acquire, id, ctx).await?;

        match pool.acquire_any(ctx).await {
            Ok((guard, wait_duration)) => {
                let acquired_at = Instant::now();
                tracing::debug!(
                    resource_id = %id,
                    wait_ms = wait_duration.as_millis() as u64,
                    "Acquired resource instance"
                );

                self.event_bus.emit(ResourceEvent::Acquired {
                    resource_key: resource_key.clone(),
                    wait_duration,
                });

                // Run after-hooks for Acquire; errors are logged but never propagated.
                self.hooks
                    .run_after(&HookEvent::Acquire, id, ctx, true)
                    .await;

                // Run Release hooks when the guard is dropped.
                let release_resource_id = resource_key.clone();
                let release_hooks = self.hooks_ref();
                let release_bus = Arc::clone(&self.event_bus);
                let release_ctx = ctx.clone();
                let guard_with_release = self.wrap_guard_with_release_hook(
                    guard,
                    release_resource_id,
                    release_hooks,
                    release_bus,
                    release_ctx,
                );

                // Tier 1: wrap in InstrumentedGuard so drop records usage.
                let recorder = ctx.recorder();
                let instrumented = InstrumentedGuard::new(
                    guard_with_release,
                    resource_key.clone(),
                    acquired_at,
                    wait_duration,
                    recorder,
                );
                Ok(Box::new(instrumented))
            }
            Err(err) => {
                // Run after-hooks for the failure path too.
                self.hooks
                    .run_after(&HookEvent::Acquire, id, ctx, false)
                    .await;

                self.event_bus.emit(ResourceEvent::Error {
                    resource_key: resource_key.clone(),
                    error: err.to_string(),
                });
                Err(err)
            }
        }
    }

    /// Acquire a resource by type, without string ID or manual downcast.
    ///
    /// Use this when the resource type is known at compile time. Returns a
    /// [`TypedResourceGuard<R::Instance>`] so you can access typed instance via `.get()`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let guard = manager.acquire_typed::<TelegramBotResource>(&ctx).await?;
    /// let bot = guard.get().expect("typed guard must match resource type");
    /// ```
    pub async fn acquire_typed<R: Resource>(
        &self,
        ctx: &Context,
    ) -> Result<TypedResourceGuard<R::Instance>>
    where
        R::Instance: Any,
    {
        let key = self
            .type_index
            .get(&TypeId::of::<R>())
            .map(|r| r.clone())
            .ok_or_else(|| Error::Unavailable {
                resource_key: nebula_core::resource_key!("unknown"),
                reason: format!(
                    "Resource type {} not registered",
                    std::any::type_name::<R>()
                ),
                retryable: false,
            })?;
        let guard = self.acquire(&key, ctx).await?;
        Ok(TypedResourceGuard {
            guard,
            _marker: std::marker::PhantomData,
        })
    }

    /// Get an Arc reference to the hooks registry for use in spawned tasks.
    fn hooks_ref(&self) -> SmallVec<[Arc<dyn crate::hooks::ResourceHook>; HOOKS_INLINE]> {
        self.hooks.snapshot()
    }

    /// Get an `Arc` reference to the hook registry (e.g. for passing to pools).
    #[must_use]
    pub fn hooks_arc(&self) -> &Arc<HookRegistry> {
        &self.hooks
    }

    /// Wrap an AnyGuard so that Release hooks fire when it is dropped.
    fn wrap_guard_with_release_hook(
        &self,
        inner: AnyGuard,
        resource_id: ResourceKey,
        hooks: SmallVec<[Arc<dyn crate::hooks::ResourceHook>; HOOKS_INLINE]>,
        event_bus: Arc<EventBus>,
        ctx: Context,
    ) -> AnyGuard {
        Box::new(ReleaseHookGuard {
            inner: Some(inner),
            resource_id,
            hooks,
            event_bus,
            ctx,
        })
    }

    /// Check whether a resource is registered (without acquiring an instance).
    ///
    /// This is a lightweight check that does not acquire from the pool.
    #[must_use]
    pub fn is_registered(&self, resource_key: &ResourceKey) -> bool {
        self.pool_contains(&resource_key)
    }

    /// Get a typed pool reference for a registered resource.
    ///
    /// Returns `None` if the resource is not registered or the pool does not
    /// support typed access (e.g. from hot-reload before typed_handle is set).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let pool = manager.get_pool::<HttpClientResource>(&resource)
    ///     .expect("http-client registered");
    /// pool.handle_rotation(&new_state, strategy, credential_key).await?;
    /// ```
    #[must_use]
    pub fn get_pool<R: Resource>(&self, resource: &R) -> Option<Arc<TypedPool<R>>>
    where
        R::Instance: Any,
    {
        let key = resource.metadata().key.clone();
        let entry = self.pool_get(&key)?;
        let handle = entry.typed_handle.as_ref()?;
        handle.clone().downcast::<TypedPool<R>>().ok()
    }

    /// Deregister a resource, shutting down its pool, cancelling its
    /// auto-scaler, stopping health monitoring, releasing it from
    /// quarantine, and removing all dependency edges.
    ///
    /// Returns `true` if the resource was registered, `false` otherwise.
    pub async fn deregister(&self, resource_key: &ResourceKey) -> bool {
        let id: &str = &resource_key;
        // Cancel auto-scaler (if any) — must happen before pool shutdown
        // so the scaler doesn't keep the Arc<dyn AnyPool> alive.
        if let Some((_, handle)) = self.auto_scalers.remove(id) {
            handle.abort();
        }

        // Stop all health monitoring tasks for this resource.
        self.health_checker.stop_monitoring_resource(id);

        let removed = self.pool_remove(id);
        self.deps.write().remove_all_for(id);
        self.metadata.remove(id);
        // Remove the TypeId → key mapping (scan is O(n) but deregister is rare).
        self.type_index.retain(|_, v| v.as_str() != id);
        self.health_states.invalidate(id);

        // Release from quarantine (if quarantined) and emit event.
        if let Some(entry) = self.quarantine.release(id) {
            self.event_bus.emit(ResourceEvent::QuarantineReleased {
                resource_key: resource_key.clone(),
                recovery_attempts: entry.recovery_attempts,
            });
        }

        if let Some(entry) = removed {
            let _ = entry.pool.shutdown().await;
            self.event_bus.emit(ResourceEvent::CleanedUp {
                resource_key: resource_key.clone(),
                reason: crate::events::CleanupReason::Evicted,
            });
            true
        } else {
            false
        }
    }

    /// Shut down all pools whose scope is contained by `scope`.
    ///
    /// Pools are shut down in reverse topological order to respect
    /// dependency relationships.
    pub async fn shutdown_scope(&self, scope: &Scope) -> Result<()> {
        // Collect pool IDs whose scope is contained by the target scope.
        let affected_ids: Vec<String> = self
            .pool_entries()
            .into_iter()
            .filter(|(_, entry)| scope.contains(&entry.scope))
            .map(|(id, _)| id)
            .collect();

        // Build reverse topo sort among affected IDs.
        let full_order = self.deps.read().topological_sort().unwrap_or_default();
        let affected_set: HashSet<&str> = affected_ids.iter().map(String::as_str).collect();
        let mut ordered: Vec<String> = full_order
            .into_iter()
            .filter(|id| affected_set.contains(id.as_str()))
            .collect();
        ordered.reverse();

        // Include any affected pools not in the dependency graph (no declared deps).
        let ordered_set: HashSet<String> = ordered.iter().cloned().collect();
        for id in &affected_ids {
            if !ordered_set.contains(id) {
                ordered.push(id.clone());
            }
        }

        for id in &ordered {
            if let Some(entry) = self.pool_remove(id) {
                let _ = entry.pool.shutdown().await;
                if let Ok(key) = nebula_core::ResourceKey::try_from(id.as_str()) {
                    self.event_bus.emit(ResourceEvent::CleanedUp {
                        resource_key: key,
                        reason: crate::events::CleanupReason::Shutdown,
                    });
                } else {
                    tracing::warn!(resource_id = %id, "skipping cleanup event for invalid resource key");
                }
            }
            self.deps.write().remove_all_for(id);
        }

        Ok(())
    }

    /// Get a reference to the event bus.
    #[must_use]
    pub fn event_bus(&self) -> &Arc<EventBus> {
        &self.event_bus
    }

    /// Get a read-only status snapshot for a single resource.
    #[must_use]
    pub fn get_status(&self, resource_key: &ResourceKey) -> Option<ResourceStatus> {
        let id: &str = &resource_key;
        let entry = self.pool_get(id)?;
        let scope = entry.scope.clone();
        let (active, idle, max_size) = entry.pool.utilization_snapshot();

        let metadata = self
            .metadata
            .get(id)
            .map(|m| m.value().clone())
            .unwrap_or_else(|| ResourceMetadata::from_key(resource_key.clone()));

        let health = self
            .health_states
            .get(id)
            .clone()
            .unwrap_or(HealthState::Unknown);

        let quarantine_entry = self.quarantine.get(id);
        let quarantined = quarantine_entry.is_some();
        let quarantine_reason = quarantine_entry.map(|entry| entry.reason.to_string());

        Some(ResourceStatus {
            metadata,
            health,
            pool: ResourcePoolStatus {
                active,
                idle,
                max_size,
            },
            quarantined,
            quarantine_reason,
            scope,
        })
    }

    /// Get status snapshots for all registered resources.
    #[must_use]
    pub fn list_status(&self) -> Vec<ResourceStatus> {
        let mut statuses: Vec<ResourceStatus> = self
            .pool_keys()
            .into_iter()
            .filter_map(|id| {
                let key = ResourceKey::try_from(id.as_str()).ok()?;
                self.get_status(&key)
            })
            .collect();

        statuses.sort_by(|a, b| (*a.metadata.key).cmp(&*b.metadata.key));
        statuses
    }

    /// Get a reference to the health checker.
    #[must_use]
    pub fn health_checker(&self) -> &Arc<crate::health::HealthChecker> {
        &self.health_checker
    }

    /// Get a reference to the hook registry.
    #[must_use]
    pub fn hooks(&self) -> &Arc<HookRegistry> {
        &self.hooks
    }

    /// Start health monitoring for a resource.
    ///
    /// This is a convenience wrapper around
    /// [`HealthChecker::start_monitoring`](crate::health::HealthChecker::start_monitoring).
    /// It generates a UUID instance ID automatically.
    ///
    /// **Note:** [`register`](Self::register) /
    /// [`register_scoped`](Self::register_scoped) do **not** start
    /// monitoring automatically because building a
    /// [`HealthCheckable`] requires
    /// access to the resource and config (which are consumed by the
    /// pool). Use [`ResourceHealthAdapter`](crate::health::ResourceHealthAdapter)
    /// to bridge a `Resource` to `HealthCheckable`, then call this
    /// method:
    ///
    /// ```rust,ignore
    /// let adapter = ResourceHealthAdapter::new(resource_clone, config_clone, scope);
    /// manager.start_health_monitoring("postgres", adapter);
    /// ```
    pub fn start_health_monitoring<H: HealthCheckable + 'static>(
        &self,
        resource_id: &str,
        checkable: H,
    ) {
        let instance_id = uuid::Uuid::new_v4();
        self.health_checker.start_monitoring(
            instance_id,
            resource_id.to_string(),
            Arc::new(checkable),
        );
    }

    /// Get a reference to the quarantine manager.
    #[must_use]
    pub fn quarantine(&self) -> &Arc<QuarantineManager> {
        &self.quarantine
    }

    /// Get the current health state of a resource, if set.
    #[must_use]
    pub fn get_health_state(&self, resource_key: &ResourceKey) -> Option<HealthState> {
        self.health_states.get(resource_key.as_ref())
    }

    /// Set a resource's health state and propagate the effect to dependents.
    ///
    /// When a resource becomes `Unhealthy`, all its direct dependents are
    /// marked `Degraded` (with reason referencing the unhealthy dependency).
    /// When a resource becomes `Healthy`, any dependent whose degraded
    /// reason mentions this resource has its degraded state cleared.
    pub fn set_health_state(&self, resource_key: &ResourceKey, state: HealthState) {
        self.propagate_health(resource_key, state);
    }

    /// Internal implementation of health state propagation.
    fn propagate_health(&self, resource_key: &ResourceKey, state: HealthState) {
        let id: &str = &resource_key;
        self.health_states.insert(id.to_string(), state.clone());

        let dependents = self.deps.read().get_dependents(id);

        match &state {
            HealthState::Unhealthy { .. } => {
                let reason = format!("Dependency {id} is unhealthy");
                for dep in &dependents {
                    // Only degrade if the dependent is not already unhealthy
                    // (unhealthy is worse than degraded, don't overwrite it).
                    let dominated = self
                        .health_states
                        .get(dep)
                        .is_some_and(|s| matches!(s, HealthState::Unhealthy { .. }));

                    if !dominated {
                        self.health_states.insert(
                            dep.clone(),
                            HealthState::Degraded {
                                reason: reason.clone(),
                                performance_impact: 0.5,
                            },
                        );
                    }
                }
            }
            HealthState::Healthy => {
                // Clear degraded states that were caused by this resource.
                for dep in &dependents {
                    let should_clear = self.health_states.get(dep).is_some_and(|s| {
                        matches!(s, HealthState::Degraded { reason, .. }
                            if reason.contains(id))
                    });

                    if should_clear {
                        self.health_states.insert(dep.clone(), HealthState::Healthy);
                    }
                }
            }
            // Degraded / Unknown: set the state but don't cascade.
            _ => {}
        }
    }

    /// Get the initialization order based on dependency graph.
    pub fn initialization_order(&self) -> Result<Vec<String>> {
        self.deps.read().topological_sort()
    }

    /// Enable auto-scaling for a registered resource pool.
    ///
    /// The auto-scaler monitors pool utilization and triggers scale-up /
    /// scale-down operations based on the given [`AutoScalePolicy`].
    ///
    /// Returns `Ok(())` if the scaler was started, or an error if the
    /// resource is not registered or the policy is invalid.
    pub fn enable_autoscaling(
        &self,
        resource_key: &ResourceKey,
        policy: AutoScalePolicy,
    ) -> Result<()> {
        policy.validate()?;
        let id: &str = &resource_key;

        let pool_entry = self.pool_get(id).ok_or_else(|| Error::Unavailable {
            resource_key: resource_key.clone(),
            reason: "Resource not registered".to_string(),
            retryable: false,
        })?;
        let pool = Arc::clone(&pool_entry.pool);

        // Cancel any existing scaler for this resource.
        if let Some((_, old_handle)) = self.auto_scalers.remove(id) {
            old_handle.abort();
        }

        let cancel = tokio_util::sync::CancellationToken::new();
        let scaler = AutoScaler::new(policy, cancel);

        let pool_for_stats = Arc::clone(&pool);
        let pool_for_up = Arc::clone(&pool);
        let pool_for_down = Arc::clone(&pool);

        let handle = scaler.start(
            move || pool_for_stats.utilization_snapshot(),
            move |count| {
                let p = Arc::clone(&pool_for_up);
                async move { p.scale_up(count).await }
            },
            move |count| {
                let p = Arc::clone(&pool_for_down);
                async move { p.scale_down(count).await }
            },
        );

        self.auto_scalers.insert(id.to_string(), handle);

        tracing::info!(resource_id = %id, "Auto-scaling enabled");

        Ok(())
    }

    /// Disable auto-scaling for a resource.
    pub fn disable_autoscaling(&self, resource_key: &ResourceKey) {
        let id: &str = &resource_key;
        if let Some((_, handle)) = self.auto_scalers.remove(id) {
            handle.abort();
        }
    }

    /// Shut down all registered pools (simple, non-phased).
    pub async fn shutdown(&self) -> Result<()> {
        // Cancel all auto-scalers first.
        for entry in self.auto_scalers.iter() {
            entry.value().abort();
        }
        self.auto_scalers.clear();

        let pools: Vec<(String, Arc<dyn AnyPool>)> = self
            .pool_entries()
            .into_iter()
            .map(|(id, entry)| (id, Arc::clone(&entry.pool)))
            .collect();

        for (id, pool) in pools {
            pool.shutdown().await?;
            if let Ok(key) = nebula_core::ResourceKey::try_from(id.as_str()) {
                self.event_bus.emit(ResourceEvent::CleanedUp {
                    resource_key: key,
                    reason: crate::events::CleanupReason::Shutdown,
                });
            } else {
                tracing::warn!(resource_id = %id, "skipping cleanup event for invalid resource key");
            }
        }

        self.pool_clear();
        self.metadata.clear();
        self.health_checker.shutdown();
        Ok(())
    }

    /// Phased graceful shutdown with dependency ordering and timeouts.
    ///
    /// 1. Drain: wait up to `drain_timeout` for in-flight operations.
    /// 2. Cleanup: shut down each pool in reverse topological order,
    ///    with `cleanup_timeout` per pool.
    /// 3. Terminate: force-clear remaining pools after `terminate_timeout`.
    pub async fn shutdown_phased(&self, config: ShutdownConfig) -> Result<()> {
        // Get reverse topological order for all registered pools.
        let full_order = self.deps.read().topological_sort().unwrap_or_default();
        let registered: HashSet<String> = self.pool_keys().into_iter().collect();
        let mut ordered: Vec<String> = full_order
            .into_iter()
            .filter(|id| registered.contains(id))
            .collect();
        ordered.reverse();

        // Include any pools not in the dependency graph (no declared deps).
        let ordered_set: HashSet<String> = ordered.iter().cloned().collect();
        for id in &registered {
            if !ordered_set.contains(id) {
                ordered.push(id.clone());
            }
        }

        // Phase 1: Drain -- give in-flight operations time to complete.
        tokio::time::sleep(config.drain_timeout).await;

        // Phase 2: Cleanup each pool in dependency order with per-pool timeout.
        for id in &ordered {
            if let Some(entry) = self.pool_remove(id) {
                let _ = tokio::time::timeout(config.cleanup_timeout, entry.pool.shutdown()).await;
                self.metadata.remove(id);
                if let Ok(key) = nebula_core::ResourceKey::try_from(id.as_str()) {
                    self.event_bus.emit(ResourceEvent::CleanedUp {
                        resource_key: key,
                        reason: crate::events::CleanupReason::Shutdown,
                    });
                } else {
                    tracing::warn!(resource_id = %id, "skipping cleanup event for invalid resource key");
                }
            }
        }

        // Phase 3: Terminate -- force-clear anything remaining.
        if self.pool_len() != 0 {
            tokio::time::sleep(config.terminate_timeout).await;
            self.pool_clear();
            self.metadata.clear();
        }

        self.health_checker.shutdown();
        Ok(())
    }

    /// Hot-reload a resource's pool configuration.
    ///
    /// Creates a new pool with the new config, shuts down the old pool,
    /// and swaps the entry in the registry. The resource's dependency
    /// edges are preserved.
    ///
    /// If the resource is not registered, it is treated as a fresh registration.
    ///
    /// # Errors
    /// Returns error if the new pool cannot be created (e.g. invalid config).
    pub async fn reload_config<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        pool_config: PoolConfig,
    ) -> Result<()>
    where
        R::Instance: Any,
    {
        let key = resource.metadata().key.clone();
        let id = key.to_string();
        let had_existing_pool = self.pool_contains(&id);

        // Build the new pool before touching the registry.
        let new_pool = match Pool::with_hooks(
            resource,
            config,
            pool_config,
            Some(Arc::clone(&self.event_bus)),
            Some(Arc::clone(&self.hooks)),
        ) {
            Ok(pool) => pool,
            Err(err) => {
                tracing::warn!(
                    resource_id = %id,
                    error = %err,
                    "Rejected config reload attempt due to invalid configuration"
                );
                self.event_bus.emit(ResourceEvent::ConfigReloadRejected {
                    resource_key: key.clone(),
                    error: err.to_string(),
                    had_existing_pool,
                });
                return Err(err);
            }
        };

        // Shut down the old pool (if any), preserving the existing scope.
        let existing_scope = if let Some(entry) = self.pool_remove(&id) {
            let scope = entry.scope.clone();
            let _ = entry.pool.shutdown().await;
            scope
        } else {
            Scope::Global
        };

        let typed_pool: Arc<TypedPool<R>> = Arc::new(TypedPool { pool: new_pool });
        let any_pool: Arc<dyn AnyPool> = typed_pool.clone();
        let _ = self.pool_insert(
            id.clone(),
            PoolEntry {
                pool: any_pool,
                scope: existing_scope.clone(),
                typed_handle: Some(typed_pool),
            },
        );

        if let Ok(key) = nebula_core::ResourceKey::try_from(id.as_str()) {
            self.event_bus.emit(ResourceEvent::ConfigReloaded {
                resource_key: key,
                scope: existing_scope,
            });
        } else {
            tracing::warn!(resource_id = %id, "skipping config reloaded event for invalid resource key");
        }

        Ok(())
    }
}

impl std::fmt::Debug for Manager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Manager")
            .field("pool_count", &self.pool_len())
            .field("auto_scalers", &self.auto_scalers.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quarantine::QuarantineReason;
    use crate::resource::Config;
    use crate::scope::Scope;
    use nebula_core::resource_key;

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TestConfig {
        value: String,
    }

    impl Config for TestConfig {
        fn validate(&self) -> Result<()> {
            if self.value.is_empty() {
                return Err(Error::configuration("value cannot be empty"));
            }
            Ok(())
        }
    }

    struct TestResource;

    impl Resource for TestResource {
        type Config = TestConfig;
        type Instance = String;

        fn key(&self) -> ResourceKey {
            resource_key!("test")
        }

        async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            Ok(format!("instance-{}", config.value))
        }
    }

    fn ctx() -> Context {
        Context::new(
            Scope::Global,
            nebula_core::WorkflowId::new(),
            nebula_core::ExecutionId::new(),
        )
    }

    #[tokio::test]
    async fn register_and_acquire() {
        let mgr = Manager::new();
        let config = TestConfig {
            value: "hello".into(),
        };
        mgr.register(TestResource, config, PoolConfig::default())
            .unwrap();

        let key = resource_key!("test");
        let guard = mgr.acquire(&key, &ctx()).await.unwrap();
        let instance = guard
            .as_any()
            .downcast_ref::<String>()
            .expect("should downcast to String");
        assert_eq!(instance, "instance-hello");
    }

    #[tokio::test]
    async fn acquire_unregistered_fails() {
        let mgr = Manager::new();
        let key = resource_key!("test");
        let result = mgr.acquire(&key, &ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_clears_pools() {
        let mgr = Manager::new();
        let config = TestConfig { value: "x".into() };
        mgr.register(TestResource, config, PoolConfig::default())
            .unwrap();
        mgr.shutdown().await.unwrap();
        assert_eq!(mgr.pool_len(), 0);
    }

    #[tokio::test]
    async fn acquire_returns_to_pool_on_drop() {
        let mgr = Manager::new();
        let config = TestConfig {
            value: "pooled".into(),
        };
        let pool_config = PoolConfig {
            min_size: 0,
            max_size: 1,
            ..Default::default()
        };
        mgr.register(TestResource, config, pool_config).unwrap();

        let key = resource_key!("test");

        // Acquire and drop — should return to pool
        {
            let _guard = mgr.acquire(&key, &ctx()).await.unwrap();
        }
        // Give the spawn a moment to return the instance
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Should be able to acquire again (pool recycled)
        let guard = mgr.acquire(&key, &ctx()).await.unwrap();
        let instance = guard
            .as_any()
            .downcast_ref::<String>()
            .expect("should downcast");
        assert_eq!(instance, "instance-pooled");
    }

    #[test]
    fn register_with_invalid_pool_config_leaves_clean_state() {
        let mgr = Manager::new();
        let bad_pool = PoolConfig {
            max_size: 0, // invalid
            ..Default::default()
        };

        // Registration should fail because max_size == 0
        let result = mgr.register(TestResource, TestConfig { value: "x".into() }, bad_pool);
        assert!(result.is_err());

        // Dependency graph should be clean — no phantom "test" entry
        let order = mgr.initialization_order().unwrap();
        assert!(
            !order.contains(&"test".to_string()),
            "failed register should not leave deps in graph"
        );
    }

    #[test]
    fn re_register_same_key_replaces_pool() {
        struct ResourceA;
        impl Resource for ResourceA {
            type Config = TestConfig;
            type Instance = String;
            fn key(&self) -> ResourceKey {
                resource_key!("a")
            }
            async fn create(&self, config: &TestConfig, _ctx: &Context) -> Result<String> {
                Ok(config.value.clone())
            }
        }

        let mgr = Manager::new();
        mgr.register(
            ResourceA,
            TestConfig { value: "v1".into() },
            PoolConfig::default(),
        )
        .unwrap();
        mgr.register(
            ResourceA,
            TestConfig { value: "v2".into() },
            PoolConfig::default(),
        )
        .unwrap();
        // Second register replaces the pool for the same key; no panic.
    }

    // -----------------------------------------------------------------------
    // Shared test hook structs (extracted to module level to avoid nesting)
    // -----------------------------------------------------------------------

    struct CountingHook {
        before_count: std::sync::atomic::AtomicU32,
        after_count: std::sync::atomic::AtomicU32,
    }

    #[async_trait::async_trait]
    impl crate::hooks::ResourceHook for CountingHook {
        fn name(&self) -> &str {
            "counter"
        }
        fn events(&self) -> Vec<crate::hooks::HookEvent> {
            vec![crate::hooks::HookEvent::Acquire]
        }
        async fn before(
            &self,
            _event: &crate::hooks::HookEvent,
            _resource_id: &str,
            _ctx: &Context,
        ) -> crate::hooks::HookResult {
            self.before_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            crate::hooks::HookResult::Continue
        }
        async fn after(
            &self,
            _event: &crate::hooks::HookEvent,
            _resource_id: &str,
            _ctx: &Context,
            _success: bool,
        ) {
            self.after_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    struct BlockerHook;

    #[async_trait::async_trait]
    impl crate::hooks::ResourceHook for BlockerHook {
        fn name(&self) -> &str {
            "blocker"
        }
        fn events(&self) -> Vec<crate::hooks::HookEvent> {
            vec![crate::hooks::HookEvent::Acquire]
        }
        async fn before(
            &self,
            _event: &crate::hooks::HookEvent,
            _resource_id: &str,
            _ctx: &Context,
        ) -> crate::hooks::HookResult {
            let key = resource_key!("test");
            crate::hooks::HookResult::Cancel(Error::Unavailable {
                resource_key: key,
                reason: "blocked by hook".to_string(),
                retryable: false,
            })
        }
        async fn after(
            &self,
            _event: &crate::hooks::HookEvent,
            _resource_id: &str,
            _ctx: &Context,
            _success: bool,
        ) {
        }
    }

    // -----------------------------------------------------------------------
    // Hooks wired into acquire
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn acquire_runs_before_and_after_hooks() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let mgr = Manager::new();
        let hook = Arc::new(CountingHook {
            before_count: AtomicU32::new(0),
            after_count: AtomicU32::new(0),
        });
        mgr.hooks()
            .register(Arc::clone(&hook) as Arc<dyn crate::hooks::ResourceHook>);

        mgr.register(
            TestResource,
            TestConfig {
                value: "hook".into(),
            },
            PoolConfig::default(),
        )
        .unwrap();

        let key = resource_key!("test");
        let _guard = mgr.acquire(&key, &ctx()).await.unwrap();

        assert_eq!(hook.before_count.load(Ordering::SeqCst), 1);
        assert_eq!(hook.after_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn before_hook_cancel_blocks_acquire() {
        let mgr = Manager::new();
        mgr.hooks()
            .register(Arc::new(BlockerHook) as Arc<dyn crate::hooks::ResourceHook>);

        mgr.register(
            TestResource,
            TestConfig {
                value: "hook".into(),
            },
            PoolConfig::default(),
        )
        .unwrap();

        let key = resource_key!("test");
        let result = mgr.acquire(&key, &ctx()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("blocked by hook"),
            "error should contain the hook's reason, got: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Health state propagation
    // -----------------------------------------------------------------------

    #[test]
    fn set_health_state_stores_state() {
        let mgr = Manager::new();
        let key = resource_key!("db");
        mgr.set_health_state(
            &key,
            HealthState::Unhealthy {
                reason: "down".into(),
                recoverable: true,
            },
        );
        let state = mgr.health_states.get("db").unwrap();
        assert!(
            matches!(state, HealthState::Unhealthy { .. }),
            "expected Unhealthy, got: {:?}",
            state
        );
    }

    #[test]
    fn unhealthy_propagates_degraded_to_dependents() {
        let mgr = Manager::new();
        // Set up dependency: "app" depends on "db"
        mgr.deps.write().add_dependency("app", "db").unwrap();

        let key = resource_key!("db");
        mgr.set_health_state(
            &key,
            HealthState::Unhealthy {
                reason: "connection refused".into(),
                recoverable: true,
            },
        );

        let app_state = mgr.health_states.get("app").unwrap();
        match &app_state {
            HealthState::Degraded { reason, .. } => {
                assert!(
                    reason.contains("db"),
                    "degraded reason should mention the unhealthy dependency, got: {reason}"
                );
            }
            other => panic!("expected Degraded, got: {other:?}"),
        }
    }

    #[test]
    fn healthy_clears_degraded_on_dependents() {
        let mgr = Manager::new();
        mgr.deps.write().add_dependency("app", "db").unwrap();

        let key = resource_key!("db");
        // First mark db unhealthy (cascades to app)
        mgr.set_health_state(
            &key,
            HealthState::Unhealthy {
                reason: "down".into(),
                recoverable: true,
            },
        );
        assert!(matches!(
            mgr.health_states.get("app").unwrap(),
            HealthState::Degraded { .. }
        ));

        // Now mark db healthy (should clear app)
        mgr.set_health_state(&key, HealthState::Healthy);

        let app_state = mgr.health_states.get("app").unwrap();
        assert_eq!(
            app_state,
            HealthState::Healthy,
            "app should be cleared back to Healthy"
        );
    }

    #[test]
    fn healthy_does_not_clear_unrelated_degraded() {
        let mgr = Manager::new();
        // "app" depends on both "db" and "cache"
        {
            let mut deps = mgr.deps.write();
            deps.add_dependency("app", "db").unwrap();
            deps.add_dependency("app", "cache").unwrap();
        }

        let cache_key = resource_key!("cache");
        let db_key = resource_key!("db");

        // Mark cache unhealthy (degrades app)
        mgr.set_health_state(
            &cache_key,
            HealthState::Unhealthy {
                reason: "evicted".into(),
                recoverable: true,
            },
        );

        // Now mark db healthy -- should NOT clear the degraded state
        // caused by cache
        mgr.set_health_state(&db_key, HealthState::Healthy);

        let app_state = mgr.health_states.get("app").unwrap();
        assert!(
            matches!(app_state, HealthState::Degraded { ref reason, .. } if reason.contains("cache")),
            "degraded state from cache should remain, got: {:?}",
            app_state
        );
    }

    #[test]
    fn unhealthy_does_not_downgrade_already_unhealthy_dependent() {
        let mgr = Manager::new();
        mgr.deps.write().add_dependency("app", "db").unwrap();

        let app_key = resource_key!("app");
        let db_key = resource_key!("db");

        // Mark app itself as unhealthy (independent of db)
        mgr.set_health_state(
            &app_key,
            HealthState::Unhealthy {
                reason: "crashed".into(),
                recoverable: false,
            },
        );

        // Mark db unhealthy -- should NOT overwrite app's Unhealthy with Degraded
        mgr.set_health_state(
            &db_key,
            HealthState::Unhealthy {
                reason: "timeout".into(),
                recoverable: true,
            },
        );

        let app_state = mgr.health_states.get("app").unwrap();
        match &app_state {
            HealthState::Unhealthy { reason, .. } => {
                assert_eq!(
                    reason, "crashed",
                    "app's own unhealthy reason should be preserved"
                );
            }
            other => panic!("expected Unhealthy, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn unhealthy_resource_blocks_acquire() {
        let mgr = Manager::new();
        mgr.register(
            TestResource,
            TestConfig { value: "x".into() },
            PoolConfig::default(),
        )
        .unwrap();

        let key = resource_key!("test");
        mgr.set_health_state(
            &key,
            HealthState::Unhealthy {
                reason: "down".into(),
                recoverable: true,
            },
        );

        let result = mgr.acquire(&key, &ctx()).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("unhealthy"),
            "acquire should fail with unhealthy reason"
        );
    }

    // -----------------------------------------------------------------------
    // reload_config availability gap
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn reload_config_swaps_pool() {
        let mgr = Manager::new();
        let config_a = TestConfig { value: "A".into() };
        let config_b = TestConfig { value: "B".into() };

        mgr.register(TestResource, config_a, PoolConfig::default())
            .unwrap();

        let key = resource_key!("test");

        // Acquire from pool A
        let guard = mgr.acquire(&key, &ctx()).await.unwrap();
        let inst = guard.as_any().downcast_ref::<String>().expect("downcast");
        assert_eq!(inst, "instance-A");
        drop(guard);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Reload with config B
        mgr.reload_config(TestResource, config_b, PoolConfig::default())
            .await
            .unwrap();

        // Acquire from pool B
        let guard = mgr.acquire(&key, &ctx()).await.unwrap();
        let inst = guard.as_any().downcast_ref::<String>().expect("downcast");
        assert_eq!(inst, "instance-B");
    }

    #[tokio::test]
    async fn reload_config_while_guard_held() {
        let mgr = Manager::new();
        mgr.register(
            TestResource,
            TestConfig {
                value: "old".into(),
            },
            PoolConfig::default(),
        )
        .unwrap();

        let key = resource_key!("test");

        // Hold a guard from the old pool
        let old_guard = mgr.acquire(&key, &ctx()).await.unwrap();
        let old_inst = old_guard
            .as_any()
            .downcast_ref::<String>()
            .expect("downcast")
            .clone();
        assert_eq!(old_inst, "instance-old");

        // Reload: old pool shut down, new pool installed
        mgr.reload_config(
            TestResource,
            TestConfig {
                value: "new".into(),
            },
            PoolConfig::default(),
        )
        .await
        .unwrap();

        // Drop old guard — should not panic (old pool still alive via Arc)
        drop(old_guard);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Acquire from new pool
        let new_guard = mgr.acquire(&key, &ctx()).await.unwrap();
        let new_inst = new_guard
            .as_any()
            .downcast_ref::<String>()
            .expect("downcast");
        assert_eq!(new_inst, "instance-new");
    }

    #[test]
    fn get_status_returns_snapshot() {
        let mgr = Manager::new();
        mgr.register(
            TestResource,
            TestConfig {
                value: "status".into(),
            },
            PoolConfig {
                min_size: 0,
                max_size: 7,
                ..Default::default()
            },
        )
        .unwrap();

        let key = resource_key!("test");
        mgr.set_health_state(
            &key,
            HealthState::Degraded {
                reason: "latency".into(),
                performance_impact: 0.25,
            },
        );
        assert!(mgr.quarantine.quarantine(
            "test",
            QuarantineReason::ManualQuarantine {
                reason: "operator".into()
            }
        ));

        let status = mgr.get_status(&key).expect("status should exist");
        assert_eq!(status.metadata.key, key);
        assert!(matches!(status.health, HealthState::Degraded { .. }));
        assert_eq!(status.pool.max_size, 7);
        assert_eq!(status.scope, Scope::Global);
        assert!(status.quarantined);
        assert!(
            status
                .quarantine_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("operator"))
        );
    }

    #[test]
    fn list_status_returns_sorted_snapshots() {
        struct Alpha;
        impl Resource for Alpha {
            type Config = TestConfig;
            type Instance = String;
            fn key(&self) -> ResourceKey {
                resource_key!("alpha")
            }
            async fn create(&self, config: &TestConfig, _ctx: &Context) -> Result<String> {
                Ok(format!("instance-{}", config.value))
            }
        }

        struct Zeta;
        impl Resource for Zeta {
            type Config = TestConfig;
            type Instance = String;
            fn key(&self) -> ResourceKey {
                resource_key!("zeta")
            }
            async fn create(&self, config: &TestConfig, _ctx: &Context) -> Result<String> {
                Ok(format!("instance-{}", config.value))
            }
        }

        let mgr = Manager::new();
        mgr.register(
            Zeta,
            TestConfig { value: "z".into() },
            PoolConfig::default(),
        )
        .unwrap();
        mgr.register(
            Alpha,
            TestConfig { value: "a".into() },
            PoolConfig::default(),
        )
        .unwrap();

        let statuses = mgr.list_status();
        let keys: Vec<String> = statuses
            .into_iter()
            .map(|status| status.metadata.key.to_string())
            .collect();
        assert_eq!(keys, vec!["alpha".to_string(), "zeta".to_string()]);
    }
}
