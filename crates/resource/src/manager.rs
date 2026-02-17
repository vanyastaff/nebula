//! Resource manager — central registry, pool orchestration, and dependency ordering.

use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::events::{EventBus, ResourceEvent};
use crate::guard::Guard;
use crate::health::HealthState;
use crate::hooks::{HookEvent, HookRegistry};
use crate::pool::{Pool, PoolConfig};
use crate::quarantine::{QuarantineConfig, QuarantineManager};
use crate::resource::Resource;
use crate::scope::{Scope, Strategy};

// ---------------------------------------------------------------------------
// DependencyGraph
// ---------------------------------------------------------------------------

/// Dependency graph for managing resource initialization order.
///
/// Resources are identified by plain string keys (matching `Resource::id()`).
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// resource key -> list of dependencies (what this resource depends on)
    dependencies: HashMap<String, Vec<String>>,
    /// resource key -> list of dependents (what depends on this resource)
    dependents: HashMap<String, Vec<String>>,
}

impl DependencyGraph {
    /// Create a new empty dependency graph
    #[must_use]
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
        }
    }

    /// Add a dependency relationship: `resource` depends on `depends_on`
    ///
    /// # Errors
    /// Returns error if adding this dependency would create a cycle
    pub fn add_dependency(
        &mut self,
        resource: impl Into<String>,
        depends_on: impl Into<String>,
    ) -> Result<()> {
        let resource = resource.into();
        let depends_on = depends_on.into();

        // Don't allow self-dependency
        if resource == depends_on {
            return Err(Error::CircularDependency {
                cycle: format!("{resource} -> {resource}"),
            });
        }

        // Skip if this edge already exists
        let deps = self.dependencies.entry(resource.clone()).or_default();
        if deps.contains(&depends_on) {
            return Ok(());
        }

        // Add to dependencies map
        deps.push(depends_on.clone());

        // Add to dependents map
        self.dependents
            .entry(depends_on.clone())
            .or_default()
            .push(resource.clone());

        // Check for cycles after adding
        if let Some(cycle) = self.detect_cycle() {
            // Rollback the changes
            self.remove_dependency(&resource, &depends_on);
            return Err(Error::CircularDependency {
                cycle: cycle.join(" -> "),
            });
        }

        Ok(())
    }

    /// Remove a single dependency relationship.
    fn remove_dependency(&mut self, resource: &str, depends_on: &str) {
        if let Some(deps) = self.dependencies.get_mut(resource) {
            deps.retain(|d| d != depends_on);
        }
        if let Some(deps) = self.dependents.get_mut(depends_on) {
            deps.retain(|d| d != resource);
        }
    }

    /// Remove all dependency edges involving `resource` (both as source and target).
    ///
    /// Used when re-registering a resource to ensure a clean slate.
    pub fn remove_all_for(&mut self, resource: &str) {
        // Remove edges where `resource` is the dependent (resource -> X)
        if let Some(deps) = self.dependencies.remove(resource) {
            for dep in &deps {
                if let Some(rev) = self.dependents.get_mut(dep.as_str()) {
                    rev.retain(|d| d != resource);
                }
            }
        }
        // Remove edges where `resource` is the dependency (X -> resource)
        if let Some(dependents) = self.dependents.remove(resource) {
            for dep in &dependents {
                if let Some(fwd) = self.dependencies.get_mut(dep.as_str()) {
                    fwd.retain(|d| d != resource);
                }
            }
        }
    }

    /// Get all dependencies for a resource
    #[must_use]
    pub fn get_dependencies(&self, resource: &str) -> Vec<String> {
        self.dependencies.get(resource).cloned().unwrap_or_default()
    }

    /// Get all dependents of a resource (what depends on this resource)
    #[must_use]
    pub fn get_dependents(&self, resource: &str) -> Vec<String> {
        self.dependents.get(resource).cloned().unwrap_or_default()
    }

    /// Detect if there's a cycle in the dependency graph
    ///
    /// # Returns
    /// `Some(cycle_path)` if a cycle is detected, None otherwise
    #[must_use]
    pub fn detect_cycle(&self) -> Option<Vec<String>> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for node in self.dependencies.keys() {
            if !visited.contains(node.as_str())
                && let Some(cycle) =
                    self.detect_cycle_dfs(node, &mut visited, &mut rec_stack, &mut path)
            {
                return Some(cycle);
            }
        }

        None
    }

    /// DFS-based cycle detection helper
    fn detect_cycle_dfs(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());
        path.push(node.to_string());

        let result = self.check_deps_for_cycle(node, visited, rec_stack, path);

        rec_stack.remove(node);
        path.pop();
        result
    }

    /// Check each dependency of `node` for cycles.
    fn check_deps_for_cycle(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        let deps = self.dependencies.get(node)?;
        for dep in deps {
            if !visited.contains(dep.as_str()) {
                let cycle = self.detect_cycle_dfs(dep, visited, rec_stack, path);
                if cycle.is_some() {
                    return cycle;
                }
            } else if rec_stack.contains(dep.as_str()) {
                let cycle_start = path
                    .iter()
                    .position(|p| p == dep)
                    .expect("Cycle detected but start node not found in path - this is a bug in cycle detection logic");
                return Some(path[cycle_start..].to_vec());
            }
        }
        None
    }

    /// Perform topological sort to get initialization order
    ///
    /// # Returns
    /// Ordered list of resource keys where dependencies come before dependents
    ///
    /// # Errors
    /// Returns error if there's a cycle in the graph
    pub fn topological_sort(&self) -> Result<Vec<String>> {
        // Use Kahn's algorithm
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut all_nodes = HashSet::new();

        // Collect all nodes and calculate in-degrees
        for (node, deps) in &self.dependencies {
            all_nodes.insert(node.clone());
            in_degree.entry(node.clone()).or_insert(0);

            for dep in deps {
                all_nodes.insert(dep.clone());
                in_degree.entry(dep.clone()).or_insert(0);
                *in_degree.entry(node.clone()).or_insert(0) += 1;
            }
        }

        // Find all nodes with no incoming edges
        let mut queue: VecDeque<String> = in_degree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(node, _)| node.clone())
            .collect();

        let mut sorted = Vec::new();

        while let Some(node) = queue.pop_front() {
            sorted.push(node.clone());

            let Some(deps) = self.dependents.get(&node) else {
                continue;
            };
            for dependent in deps {
                let Some(degree) = in_degree.get_mut(dependent) else {
                    continue;
                };
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(dependent.clone());
                }
            }
        }

        // If we haven't sorted all nodes, there's a cycle
        if sorted.len() != all_nodes.len()
            && let Some(cycle) = self.detect_cycle()
        {
            return Err(Error::CircularDependency {
                cycle: cycle.join(" -> "),
            });
        }

        Ok(sorted)
    }

    /// Get the initialization order for a specific resource and its dependencies.
    ///
    /// # Errors
    /// Returns [`Error::CircularDependency`] if a cycle is detected in the
    /// subgraph reachable from `resource`.
    pub fn get_init_order(&self, resource: &str) -> Result<Vec<String>> {
        let mut visited = HashSet::new();
        let mut visiting = HashSet::new();
        let mut order = Vec::new();

        self.build_init_order(resource, &mut visited, &mut visiting, &mut order)?;

        Ok(order)
    }

    /// Recursively build initialization order using DFS with cycle detection.
    ///
    /// `visiting` tracks the current recursion stack — if we encounter a node
    /// already in `visiting`, we have found a back-edge (cycle).
    fn build_init_order(
        &self,
        resource: &str,
        visited: &mut HashSet<String>,
        visiting: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<()> {
        if visited.contains(resource) {
            return Ok(());
        }

        if !visiting.insert(resource.to_string()) {
            return Err(Error::CircularDependency {
                cycle: resource.to_string(),
            });
        }

        // Visit dependencies first
        if let Some(deps) = self.dependencies.get(resource) {
            for dep in deps {
                self.build_init_order(dep, visited, visiting, order)?;
            }
        }

        visiting.remove(resource);
        visited.insert(resource.to_string());

        // Add this resource after its dependencies
        order.push(resource.to_string());

        Ok(())
    }

    /// Get all transitive dependencies of a resource
    #[must_use]
    pub fn get_all_dependencies(&self, resource: &str) -> HashSet<String> {
        let mut all_deps = HashSet::new();
        self.collect_dependencies(resource, &mut all_deps);
        all_deps
    }

    /// Recursively collect all dependencies
    fn collect_dependencies(&self, resource: &str, collected: &mut HashSet<String>) {
        if let Some(deps) = self.dependencies.get(resource) {
            for dep in deps {
                if collected.insert(dep.clone()) {
                    self.collect_dependencies(dep, collected);
                }
            }
        }
    }

    /// Check if resource A depends on resource B (directly or transitively)
    #[must_use]
    pub fn depends_on(&self, resource: &str, depends_on: &str) -> bool {
        let all_deps = self.get_all_dependencies(resource);
        all_deps.contains(depends_on)
    }
}

// ---------------------------------------------------------------------------
// Type-erased guard
// ---------------------------------------------------------------------------

/// Trait for type-erased resource guards.
///
/// Provides `&dyn Any` access to the inner instance while the concrete
/// `TypedGuard<R>` holds the real `Guard` that returns the instance
/// to the pool on drop.
pub trait AnyGuardTrait: Send {
    /// Access the inner instance as `&dyn Any` for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// Access the inner instance as `&mut dyn Any` for downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Type-erased guard returned by [`Manager::acquire`].
///
/// When dropped, the underlying `Guard` returns the instance
/// to the pool. Use [`as_any`](AnyGuardTrait::as_any) and
/// `downcast_ref` to access the concrete instance.
pub type AnyGuard = Box<dyn AnyGuardTrait>;

impl std::fmt::Debug for dyn AnyGuardTrait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyGuard").finish_non_exhaustive()
    }
}

/// Opaque handle wrapping a type-erased resource guard.
///
/// Returned by the engine's `ResourceProvider` adapter (via
/// `ActionContext::resource()`). When dropped, the underlying guard
/// returns the instance to its pool.
pub struct ResourceHandle {
    guard: AnyGuard,
}

impl ResourceHandle {
    /// Wrap an [`AnyGuard`] in a handle.
    pub fn new(guard: AnyGuard) -> Self {
        Self { guard }
    }

    /// Access the inner resource instance by type.
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.guard.as_any().downcast_ref()
    }

    /// Mutably access the inner resource instance by type.
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.guard.as_any_mut().downcast_mut()
    }
}

/// Concrete guard wrapping a typed `Guard`.
struct TypedGuard<R: Resource> {
    guard: Guard<R::Instance>,
}

impl<R: Resource> AnyGuardTrait for TypedGuard<R>
where
    R::Instance: Any,
{
    fn as_any(&self) -> &dyn Any {
        &*self.guard
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        &mut *self.guard
    }
}

// ---------------------------------------------------------------------------
// Type-erased pool wrapper
// ---------------------------------------------------------------------------

/// Type-erased pool interface so the manager can store pools of different
/// resource types in a single map.
trait AnyPool: Send + Sync {
    /// Acquire a type-erased instance.
    fn acquire_any<'a>(
        &'a self,
        ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = Result<AnyGuard>> + Send + 'a>>;

    /// Shut down the pool.
    fn shutdown(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
}

/// Concrete adapter from `Pool<R>` to `AnyPool`.
struct TypedPool<R: Resource> {
    pool: Pool<R>,
}

impl<R: Resource> AnyPool for TypedPool<R>
where
    R::Instance: Any,
{
    fn acquire_any<'a>(
        &'a self,
        ctx: &'a Context,
    ) -> Pin<Box<dyn Future<Output = Result<AnyGuard>> + Send + 'a>> {
        Box::pin(async move {
            let guard = self.pool.acquire(ctx).await?;
            Ok(Box::new(TypedGuard::<R> { guard }) as AnyGuard)
        })
    }

    fn shutdown(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move { self.pool.shutdown().await })
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

/// A pool together with the scope it was registered under.
struct PoolEntry {
    pool: Arc<dyn AnyPool>,
    scope: Scope,
}

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

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout: Duration::from_secs(30),
            cleanup_timeout: Duration::from_secs(10),
            terminate_timeout: Duration::from_secs(5),
        }
    }
}

/// Central manager for resource pools and dependency ordering.
///
/// Pools are keyed by the resource's string ID (`Resource::id()`), allowing
/// multiple pools of the same resource type with different IDs.
pub struct Manager {
    /// Pools indexed by resource ID string, with associated scope.
    pools: DashMap<String, PoolEntry>,
    /// Dependency graph for initialization ordering.
    deps: parking_lot::RwLock<DependencyGraph>,
    /// Background health checker.
    health_checker: Arc<crate::health::HealthChecker>,
    /// Event bus for lifecycle events.
    event_bus: Arc<EventBus>,
    /// Quarantine manager for unhealthy resources.
    quarantine: QuarantineManager,
    /// Per-resource health states (set externally or by health checker events).
    health_states: DashMap<String, HealthState>,
    /// Hook registry for lifecycle hooks.
    hooks: HookRegistry,
}

impl Default for Manager {
    fn default() -> Self {
        Self {
            pools: DashMap::new(),
            deps: parking_lot::RwLock::new(DependencyGraph::default()),
            health_checker: Arc::new(crate::health::HealthChecker::new(
                crate::health::HealthCheckConfig::default(),
            )),
            event_bus: Arc::new(EventBus::default()),
            quarantine: QuarantineManager::default(),
            health_states: DashMap::new(),
            hooks: HookRegistry::default(),
        }
    }
}

impl Manager {
    /// Create a new empty resource manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a manager with a custom health check configuration.
    #[must_use]
    pub fn with_health_config(config: crate::health::HealthCheckConfig) -> Self {
        Self {
            health_checker: Arc::new(crate::health::HealthChecker::new(config)),
            ..Self::default()
        }
    }

    /// Create a manager with a custom event bus.
    #[must_use]
    pub fn with_event_bus(event_bus: Arc<EventBus>) -> Self {
        Self {
            event_bus,
            ..Self::default()
        }
    }

    /// Create a manager with a custom quarantine configuration.
    #[must_use]
    pub fn with_quarantine_config(quarantine_config: QuarantineConfig) -> Self {
        Self {
            quarantine: QuarantineManager::new(quarantine_config),
            ..Self::default()
        }
    }

    /// Create a manager with both a custom event bus and quarantine config.
    #[must_use]
    pub fn with_event_bus_and_quarantine(
        event_bus: Arc<EventBus>,
        quarantine_config: QuarantineConfig,
    ) -> Self {
        Self {
            event_bus,
            quarantine: QuarantineManager::new(quarantine_config),
            ..Self::default()
        }
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
        let id = resource.id().to_string();
        let new_deps: Vec<String> = resource
            .dependencies()
            .into_iter()
            .map(Into::into)
            .collect();

        // Create pool first -- if this fails, nothing is modified.
        let pool = Pool::new(resource, config, pool_config)?;

        // Validate all new deps on a clone before touching the real graph.
        // This ensures the mutation is all-or-nothing.
        {
            let mut deps = self.deps.write();
            let mut candidate = deps.clone();
            candidate.remove_all_for(&id);
            for dep in &new_deps {
                candidate.add_dependency(&id, dep)?;
            }
            // Validation passed -- swap in the new graph.
            *deps = candidate;
        }

        let any_pool: Arc<dyn AnyPool> = Arc::new(TypedPool { pool });
        self.pools.insert(
            id.clone(),
            PoolEntry {
                pool: any_pool,
                scope: scope.clone(),
            },
        );

        self.event_bus.emit(ResourceEvent::Created {
            resource_id: id.clone(),
            scope,
        });

        #[cfg(feature = "tracing")]
        tracing::debug!(resource_id = %id, "Registered resource");

        Ok(())
    }

    /// Acquire a resource instance by resource ID.
    ///
    /// Returns an [`AnyGuard`] that provides `&dyn Any` access to the
    /// instance. When the guard is dropped, the instance is returned to
    /// the pool.
    ///
    /// Checks quarantine status, health state, and scope compatibility
    /// before delegating to the pool.
    pub async fn acquire(&self, resource_id: &str, ctx: &Context) -> Result<AnyGuard> {
        // Check quarantine -- quarantined resources cannot be acquired.
        if self.quarantine.is_quarantined(resource_id) {
            return Err(Error::Unavailable {
                resource_id: resource_id.to_string(),
                reason: "Resource is quarantined".to_string(),
                retryable: true,
            });
        }

        // Check health state -- block on Unhealthy, warn on Degraded.
        if let Some(state) = self.health_states.get(resource_id) {
            match state.value() {
                HealthState::Unhealthy { recoverable, .. } => {
                    self.event_bus.emit(ResourceEvent::Error {
                        resource_id: resource_id.to_string(),
                        error: "Resource is unhealthy".to_string(),
                    });
                    return Err(Error::Unavailable {
                        resource_id: resource_id.to_string(),
                        reason: "Resource is unhealthy".to_string(),
                        retryable: *recoverable,
                    });
                }
                HealthState::Degraded { reason, .. } => {
                    #[cfg(feature = "tracing")]
                    tracing::warn!(
                        resource_id,
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
        let (pool, resource_scope) = match self.pools.get(resource_id) {
            Some(entry) => (Arc::clone(&entry.pool), entry.scope.clone()),
            None => {
                let err = Error::Unavailable {
                    resource_id: resource_id.to_string(),
                    reason: "Resource not registered".to_string(),
                    retryable: false,
                };
                self.event_bus.emit(ResourceEvent::Error {
                    resource_id: resource_id.to_string(),
                    error: err.to_string(),
                });
                return Err(err);
            }
        };

        // Validate scope: the resource scope must contain the caller's scope
        // under the Hierarchical strategy.
        if !Strategy::Hierarchical.is_compatible(&resource_scope, &ctx.scope) {
            return Err(Error::Unavailable {
                resource_id: resource_id.to_string(),
                reason: format!(
                    "Scope mismatch: resource scope {} does not contain requested scope {}",
                    resource_scope, ctx.scope
                ),
                retryable: false,
            });
        }

        // Run before-hooks; if any hook cancels, abort the acquire.
        self.hooks
            .run_before(&HookEvent::Acquire, resource_id, ctx)
            .await?;

        match pool.acquire_any(ctx).await {
            Ok(guard) => {
                #[cfg(feature = "tracing")]
                tracing::debug!(resource_id, "Acquired resource instance");

                self.event_bus.emit(ResourceEvent::Acquired {
                    resource_id: resource_id.to_string(),
                });

                // Run after-hooks; errors are logged but never propagated.
                self.hooks
                    .run_after(&HookEvent::Acquire, resource_id, ctx, true)
                    .await;

                Ok(guard)
            }
            Err(err) => {
                // Run after-hooks for the failure path too.
                self.hooks
                    .run_after(&HookEvent::Acquire, resource_id, ctx, false)
                    .await;

                self.event_bus.emit(ResourceEvent::Error {
                    resource_id: resource_id.to_string(),
                    error: err.to_string(),
                });
                Err(err)
            }
        }
    }

    /// Shut down all pools whose scope is contained by `scope`.
    ///
    /// Pools are shut down in reverse topological order to respect
    /// dependency relationships.
    pub async fn shutdown_scope(&self, scope: &Scope) -> Result<()> {
        // Collect pool IDs whose scope is contained by the target scope.
        let affected_ids: Vec<String> = self
            .pools
            .iter()
            .filter(|entry| scope.contains(&entry.value().scope))
            .map(|entry| entry.key().clone())
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
        for id in &affected_ids {
            if !ordered.contains(id) {
                ordered.push(id.clone());
            }
        }

        for id in &ordered {
            if let Some((_, entry)) = self.pools.remove(id) {
                let _ = entry.pool.shutdown().await;
                self.event_bus.emit(ResourceEvent::CleanedUp {
                    resource_id: id.clone(),
                    reason: crate::events::CleanupReason::Shutdown,
                });
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

    /// Get a reference to the health checker.
    #[must_use]
    pub fn health_checker(&self) -> &Arc<crate::health::HealthChecker> {
        &self.health_checker
    }

    /// Get a reference to the hook registry.
    #[must_use]
    pub fn hooks(&self) -> &HookRegistry {
        &self.hooks
    }

    /// Get a reference to the quarantine manager.
    #[must_use]
    pub fn quarantine(&self) -> &QuarantineManager {
        &self.quarantine
    }

    /// Get the current health state of a resource, if set.
    #[must_use]
    pub fn get_health_state(&self, resource_id: &str) -> Option<HealthState> {
        self.health_states.get(resource_id).map(|r| r.clone())
    }

    /// Set a resource's health state and propagate the effect to dependents.
    ///
    /// When a resource becomes `Unhealthy`, all its direct dependents are
    /// marked `Degraded` (with reason referencing the unhealthy dependency).
    /// When a resource becomes `Healthy`, any dependent whose degraded
    /// reason mentions this resource has its degraded state cleared.
    pub fn set_health_state(&self, resource_id: &str, state: HealthState) {
        self.propagate_health(resource_id, state);
    }

    /// Internal implementation of health state propagation.
    fn propagate_health(&self, resource_id: &str, state: HealthState) {
        self.health_states
            .insert(resource_id.to_string(), state.clone());

        let dependents = self.deps.read().get_dependents(resource_id);

        match &state {
            HealthState::Unhealthy { .. } => {
                let reason = format!("Dependency {resource_id} is unhealthy");
                for dep in &dependents {
                    // Only degrade if the dependent is not already unhealthy
                    // (unhealthy is worse than degraded, don't overwrite it).
                    let dominated = self
                        .health_states
                        .get(dep)
                        .is_some_and(|s| matches!(s.value(), HealthState::Unhealthy { .. }));

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
                        matches!(s.value(), HealthState::Degraded { reason, .. }
                            if reason.contains(resource_id))
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

    /// Shut down all registered pools (simple, non-phased).
    pub async fn shutdown(&self) -> Result<()> {
        let pools: Vec<(String, Arc<dyn AnyPool>)> = self
            .pools
            .iter()
            .map(|entry| (entry.key().clone(), Arc::clone(&entry.value().pool)))
            .collect();

        for (id, pool) in pools {
            pool.shutdown().await?;
            self.event_bus.emit(ResourceEvent::CleanedUp {
                resource_id: id,
                reason: crate::events::CleanupReason::Shutdown,
            });
        }

        self.pools.clear();
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
        let registered: HashSet<String> = self.pools.iter().map(|e| e.key().clone()).collect();
        let mut ordered: Vec<String> = full_order
            .into_iter()
            .filter(|id| registered.contains(id))
            .collect();
        ordered.reverse();

        // Include any pools not in the dependency graph (no declared deps).
        for id in &registered {
            if !ordered.contains(id) {
                ordered.push(id.clone());
            }
        }

        // Phase 1: Drain -- give in-flight operations time to complete.
        tokio::time::sleep(config.drain_timeout).await;

        // Phase 2: Cleanup each pool in dependency order with per-pool timeout.
        for id in &ordered {
            if let Some((_, entry)) = self.pools.remove(id) {
                let _ = tokio::time::timeout(config.cleanup_timeout, entry.pool.shutdown()).await;
                self.event_bus.emit(ResourceEvent::CleanedUp {
                    resource_id: id.clone(),
                    reason: crate::events::CleanupReason::Shutdown,
                });
            }
        }

        // Phase 3: Terminate -- force-clear anything remaining.
        if !self.pools.is_empty() {
            tokio::time::sleep(config.terminate_timeout).await;
            self.pools.clear();
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
        let id = resource.id().to_string();

        // Build the new pool before touching the registry.
        let new_pool = Pool::with_event_bus(
            resource,
            config,
            pool_config,
            Some(Arc::clone(&self.event_bus)),
        )?;

        // Shut down the old pool (if any), preserving the existing scope.
        let existing_scope = if let Some((_, entry)) = self.pools.remove(&id) {
            let scope = entry.scope.clone();
            let _ = entry.pool.shutdown().await;
            scope
        } else {
            Scope::Global
        };

        let any_pool: Arc<dyn AnyPool> = Arc::new(TypedPool { pool: new_pool });
        self.pools.insert(
            id.clone(),
            PoolEntry {
                pool: any_pool,
                scope: existing_scope.clone(),
            },
        );

        self.event_bus.emit(ResourceEvent::Created {
            resource_id: id,
            scope: existing_scope,
        });

        Ok(())
    }
}

impl std::fmt::Debug for Manager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Manager")
            .field("pool_count", &self.pools.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod dependency_tests {
    use super::*;

    #[test]
    fn test_add_simple_dependency() {
        let mut graph = DependencyGraph::new();

        graph.add_dependency("a", "b").unwrap();

        assert_eq!(graph.get_dependencies("a"), vec!["b".to_string()]);
        assert_eq!(graph.get_dependents("b"), vec!["a".to_string()]);
    }

    #[test]
    fn test_self_dependency_rejected() {
        let mut graph = DependencyGraph::new();

        let result = graph.add_dependency("a", "a");
        assert!(result.is_err());
    }

    #[test]
    fn test_circular_dependency_detected() {
        let mut graph = DependencyGraph::new();

        // a -> b -> c is fine
        graph.add_dependency("a", "b").unwrap();
        graph.add_dependency("b", "c").unwrap();

        // c -> a creates a cycle
        let result = graph.add_dependency("c", "a");
        assert!(result.is_err());
    }

    #[test]
    fn test_topological_sort() {
        let mut graph = DependencyGraph::new();

        // a depends on b and c
        // b depends on d
        // c depends on d
        // Expected order: d, then b and c (in any order), then a
        graph.add_dependency("a", "b").unwrap();
        graph.add_dependency("a", "c").unwrap();
        graph.add_dependency("b", "d").unwrap();
        graph.add_dependency("c", "d").unwrap();

        let sorted = graph.topological_sort().unwrap();

        let d_pos = sorted.iter().position(|r| r == "d").unwrap();
        let b_pos = sorted.iter().position(|r| r == "b").unwrap();
        let c_pos = sorted.iter().position(|r| r == "c").unwrap();
        let a_pos = sorted.iter().position(|r| r == "a").unwrap();

        assert!(d_pos < b_pos);
        assert!(d_pos < c_pos);
        assert!(b_pos < a_pos);
        assert!(c_pos < a_pos);
    }

    #[test]
    fn test_get_init_order() {
        let mut graph = DependencyGraph::new();

        graph.add_dependency("a", "b").unwrap();
        graph.add_dependency("b", "c").unwrap();

        let order = graph.get_init_order("a").unwrap();

        // Should be: c, b, a
        assert_eq!(order.len(), 3);
        assert_eq!(order[0], "c");
        assert_eq!(order[1], "b");
        assert_eq!(order[2], "a");
    }

    #[test]
    fn test_transitive_dependencies() {
        let mut graph = DependencyGraph::new();

        graph.add_dependency("a", "b").unwrap();
        graph.add_dependency("b", "c").unwrap();

        let all_deps = graph.get_all_dependencies("a");
        assert!(all_deps.contains("b"));
        assert!(all_deps.contains("c"));
        assert_eq!(all_deps.len(), 2);
    }

    #[test]
    fn test_depends_on() {
        let mut graph = DependencyGraph::new();

        graph.add_dependency("a", "b").unwrap();
        graph.add_dependency("b", "c").unwrap();

        assert!(graph.depends_on("a", "b"));
        assert!(graph.depends_on("a", "c")); // transitive
        assert!(!graph.depends_on("b", "a"));
    }

    #[test]
    fn test_self_dependency_returns_circular_dependency_error() {
        let mut graph = DependencyGraph::new();
        let err = graph.add_dependency("x", "x").unwrap_err();
        assert!(
            matches!(err, Error::CircularDependency { .. }),
            "self-dependency should be CircularDependency, got: {err:?}"
        );
    }

    #[test]
    fn test_duplicate_edge_is_idempotent() {
        let mut graph = DependencyGraph::new();

        graph.add_dependency("a", "b").unwrap();
        graph.add_dependency("a", "b").unwrap(); // duplicate — should be no-op

        assert_eq!(graph.get_dependencies("a"), vec!["b".to_string()]);
        assert_eq!(graph.get_dependents("b"), vec!["a".to_string()]);
    }

    #[test]
    fn test_remove_all_for() {
        let mut graph = DependencyGraph::new();

        graph.add_dependency("a", "b").unwrap();
        graph.add_dependency("a", "c").unwrap();
        graph.add_dependency("d", "a").unwrap();

        graph.remove_all_for("a");

        assert!(graph.get_dependencies("a").is_empty());
        assert!(!graph.get_dependents("b").contains(&"a".to_string()));
        assert!(!graph.get_dependents("c").contains(&"a".to_string()));
        assert!(!graph.get_dependencies("d").contains(&"a".to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::Config;
    use crate::scope::Scope;

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

        fn id(&self) -> &str {
            "test"
        }

        async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
            Ok(format!("instance-{}", config.value))
        }
    }

    fn ctx() -> Context {
        Context::new(Scope::Global, "wf", "ex")
    }

    #[tokio::test]
    async fn register_and_acquire() {
        let mgr = Manager::new();
        let config = TestConfig {
            value: "hello".into(),
        };
        mgr.register(TestResource, config, PoolConfig::default())
            .unwrap();

        let guard = mgr.acquire("test", &ctx()).await.unwrap();
        let instance = guard
            .as_any()
            .downcast_ref::<String>()
            .expect("should downcast to String");
        assert_eq!(instance, "instance-hello");
    }

    #[tokio::test]
    async fn acquire_unregistered_fails() {
        let mgr = Manager::new();
        let result = mgr.acquire("test", &ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_clears_pools() {
        let mgr = Manager::new();
        let config = TestConfig { value: "x".into() };
        mgr.register(TestResource, config, PoolConfig::default())
            .unwrap();
        mgr.shutdown().await.unwrap();
        assert!(mgr.pools.is_empty());
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

        // Acquire and drop — should return to pool
        {
            let _guard = mgr.acquire("test", &ctx()).await.unwrap();
        }
        // Give the spawn a moment to return the instance
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Should be able to acquire again (pool recycled)
        let guard = mgr.acquire("test", &ctx()).await.unwrap();
        let instance = guard
            .as_any()
            .downcast_ref::<String>()
            .expect("should downcast");
        assert_eq!(instance, "instance-pooled");
    }

    #[test]
    fn register_with_invalid_pool_config_leaves_no_dirty_deps() {
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
    fn re_register_replaces_dependencies() {
        struct DepResource {
            deps: Vec<&'static str>,
        }

        impl Resource for DepResource {
            type Config = TestConfig;
            type Instance = String;

            fn id(&self) -> &str {
                "with-deps"
            }

            async fn create(
                &self,
                config: &Self::Config,
                _ctx: &Context,
            ) -> Result<Self::Instance> {
                Ok(config.value.clone())
            }

            fn dependencies(&self) -> Vec<&str> {
                self.deps.clone()
            }
        }

        let mgr = Manager::new();

        // First registration: depends on "a"
        mgr.register(
            DepResource { deps: vec!["a"] },
            TestConfig { value: "v1".into() },
            PoolConfig::default(),
        )
        .unwrap();
        assert!(mgr.deps.read().depends_on("with-deps", "a"));

        // Re-register: depends on "b" instead of "a"
        mgr.register(
            DepResource { deps: vec!["b"] },
            TestConfig { value: "v2".into() },
            PoolConfig::default(),
        )
        .unwrap();
        assert!(
            mgr.deps.read().depends_on("with-deps", "b"),
            "should have new dependency"
        );
        assert!(
            !mgr.deps.read().depends_on("with-deps", "a"),
            "old dependency should be cleaned up"
        );
    }

    // -----------------------------------------------------------------------
    // Shared test hook structs (extracted to module level to avoid nesting)
    // -----------------------------------------------------------------------

    struct CountingHook {
        before_count: std::sync::atomic::AtomicU32,
        after_count: std::sync::atomic::AtomicU32,
    }

    impl crate::hooks::ResourceHook for CountingHook {
        fn name(&self) -> &str {
            "counter"
        }
        fn events(&self) -> Vec<crate::hooks::HookEvent> {
            vec![crate::hooks::HookEvent::Acquire]
        }
        fn before<'a>(
            &'a self,
            _event: &'a crate::hooks::HookEvent,
            _resource_id: &'a str,
            _ctx: &'a Context,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = crate::hooks::HookResult> + Send + 'a>,
        > {
            Box::pin(async {
                self.before_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                crate::hooks::HookResult::Continue
            })
        }
        fn after<'a>(
            &'a self,
            _event: &'a crate::hooks::HookEvent,
            _resource_id: &'a str,
            _ctx: &'a Context,
            _success: bool,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
            Box::pin(async {
                self.after_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })
        }
    }

    struct BlockerHook;

    impl crate::hooks::ResourceHook for BlockerHook {
        fn name(&self) -> &str {
            "blocker"
        }
        fn events(&self) -> Vec<crate::hooks::HookEvent> {
            vec![crate::hooks::HookEvent::Acquire]
        }
        fn before<'a>(
            &'a self,
            _event: &'a crate::hooks::HookEvent,
            _resource_id: &'a str,
            _ctx: &'a Context,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = crate::hooks::HookResult> + Send + 'a>,
        > {
            Box::pin(async {
                crate::hooks::HookResult::Cancel(Error::Unavailable {
                    resource_id: "test".to_string(),
                    reason: "blocked by hook".to_string(),
                    retryable: false,
                })
            })
        }
        fn after<'a>(
            &'a self,
            _event: &'a crate::hooks::HookEvent,
            _resource_id: &'a str,
            _ctx: &'a Context,
            _success: bool,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
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

        let _guard = mgr.acquire("test", &ctx()).await.unwrap();

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

        let result = mgr.acquire("test", &ctx()).await;
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
        mgr.set_health_state(
            "db",
            HealthState::Unhealthy {
                reason: "down".into(),
                recoverable: true,
            },
        );
        let state = mgr.health_states.get("db").unwrap();
        assert!(
            matches!(state.value(), HealthState::Unhealthy { .. }),
            "expected Unhealthy, got: {:?}",
            state.value()
        );
    }

    #[test]
    fn unhealthy_propagates_degraded_to_dependents() {
        let mgr = Manager::new();
        // Set up dependency: "app" depends on "db"
        mgr.deps.write().add_dependency("app", "db").unwrap();

        mgr.set_health_state(
            "db",
            HealthState::Unhealthy {
                reason: "connection refused".into(),
                recoverable: true,
            },
        );

        let app_state = mgr.health_states.get("app").unwrap();
        match app_state.value() {
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

        // First mark db unhealthy (cascades to app)
        mgr.set_health_state(
            "db",
            HealthState::Unhealthy {
                reason: "down".into(),
                recoverable: true,
            },
        );
        assert!(matches!(
            mgr.health_states.get("app").unwrap().value(),
            HealthState::Degraded { .. }
        ));

        // Now mark db healthy (should clear app)
        mgr.set_health_state("db", HealthState::Healthy);

        let app_state = mgr.health_states.get("app").unwrap();
        assert_eq!(
            *app_state.value(),
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

        // Mark cache unhealthy (degrades app)
        mgr.set_health_state(
            "cache",
            HealthState::Unhealthy {
                reason: "evicted".into(),
                recoverable: true,
            },
        );

        // Now mark db healthy -- should NOT clear the degraded state
        // caused by cache
        mgr.set_health_state("db", HealthState::Healthy);

        let app_state = mgr.health_states.get("app").unwrap();
        assert!(
            matches!(app_state.value(), HealthState::Degraded { reason, .. } if reason.contains("cache")),
            "degraded state from cache should remain, got: {:?}",
            app_state.value()
        );
    }

    #[test]
    fn unhealthy_does_not_downgrade_already_unhealthy_dependent() {
        let mgr = Manager::new();
        mgr.deps.write().add_dependency("app", "db").unwrap();

        // Mark app itself as unhealthy (independent of db)
        mgr.set_health_state(
            "app",
            HealthState::Unhealthy {
                reason: "crashed".into(),
                recoverable: false,
            },
        );

        // Mark db unhealthy -- should NOT overwrite app's Unhealthy with Degraded
        mgr.set_health_state(
            "db",
            HealthState::Unhealthy {
                reason: "timeout".into(),
                recoverable: true,
            },
        );

        let app_state = mgr.health_states.get("app").unwrap();
        match app_state.value() {
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

        mgr.set_health_state(
            "test",
            HealthState::Unhealthy {
                reason: "down".into(),
                recoverable: true,
            },
        );

        let result = mgr.acquire("test", &ctx()).await;
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

        // Acquire from pool A
        let guard = mgr.acquire("test", &ctx()).await.unwrap();
        let inst = guard.as_any().downcast_ref::<String>().expect("downcast");
        assert_eq!(inst, "instance-A");
        drop(guard);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Reload with config B
        mgr.reload_config(TestResource, config_b, PoolConfig::default())
            .await
            .unwrap();

        // Acquire from pool B
        let guard = mgr.acquire("test", &ctx()).await.unwrap();
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

        // Hold a guard from the old pool
        let old_guard = mgr.acquire("test", &ctx()).await.unwrap();
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
        let new_guard = mgr.acquire("test", &ctx()).await.unwrap();
        let new_inst = new_guard
            .as_any()
            .downcast_ref::<String>()
            .expect("downcast");
        assert_eq!(new_inst, "instance-new");
    }
}
