//! Resource manager — central registry, pool orchestration, and dependency ordering.

use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use dashmap::DashMap;

use crate::context::Context;
use crate::error::{Error, Result};
use crate::guard::Guard;
use crate::pool::{Pool, PoolConfig};
use crate::resource::Resource;

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

        // Add nodes that have no dependencies at all
        for node in &all_nodes {
            if !in_degree.contains_key(node) {
                queue.push_back(node.clone());
                in_degree.insert(node.clone(), 0);
            }
        }

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

    /// Get the initialization order for a specific resource and its dependencies
    pub fn get_init_order(&self, resource: &str) -> Result<Vec<String>> {
        let mut visited = HashSet::new();
        let mut order = Vec::new();

        self.build_init_order(resource, &mut visited, &mut order)?;

        Ok(order)
    }

    /// Recursively build initialization order using DFS
    fn build_init_order(
        &self,
        resource: &str,
        visited: &mut HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<()> {
        if visited.contains(resource) {
            return Ok(());
        }

        visited.insert(resource.to_string());

        // Visit dependencies first
        if let Some(deps) = self.dependencies.get(resource) {
            for dep in deps {
                self.build_init_order(dep, visited, order)?;
            }
        }

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

/// Central manager for resource pools and dependency ordering.
///
/// Pools are keyed by the resource's string ID (`Resource::id()`), allowing
/// multiple pools of the same resource type with different IDs.
#[derive(Default)]
pub struct Manager {
    /// Pools indexed by resource ID string.
    pools: DashMap<String, Arc<dyn AnyPool>>,
    /// Dependency graph for initialization ordering.
    deps: parking_lot::RwLock<DependencyGraph>,
}

impl Manager {
    /// Create a new empty resource manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a resource with its config and pool settings.
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
        let id = resource.id().to_string();
        let new_deps: Vec<String> = resource
            .dependencies()
            .into_iter()
            .map(Into::into)
            .collect();

        // Create pool first — if this fails, nothing is modified.
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
            // Validation passed — swap in the new graph.
            *deps = candidate;
        }

        let any_pool: Arc<dyn AnyPool> = Arc::new(TypedPool { pool });
        self.pools.insert(id.clone(), any_pool);

        #[cfg(feature = "tracing")]
        tracing::debug!(resource_id = %id, "Registered resource");

        Ok(())
    }

    /// Acquire a resource instance by resource ID.
    ///
    /// Returns an [`AnyGuard`] that provides `&dyn Any` access to the
    /// instance. When the guard is dropped, the instance is returned to
    /// the pool.
    pub async fn acquire(&self, resource_id: &str, ctx: &Context) -> Result<AnyGuard> {
        // Clone the Arc to release the DashMap shard lock before awaiting.
        // Without this, the Ref<> from get() holds a read lock across the
        // potentially long-running acquire_any().await (up to acquire_timeout).
        let pool = self
            .pools
            .get(resource_id)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| Error::Unavailable {
                resource_id: resource_id.to_string(),
                reason: "Resource not registered".to_string(),
                retryable: false,
            })?;

        pool.acquire_any(ctx).await.inspect(|_guard| {
            #[cfg(feature = "tracing")]
            tracing::debug!(resource_id, "Acquired resource instance");
        })
    }

    /// Get the initialization order based on dependency graph.
    pub fn initialization_order(&self) -> Result<Vec<String>> {
        self.deps.read().topological_sort()
    }

    /// Shut down all registered pools.
    pub async fn shutdown(&self) -> Result<()> {
        let pools: Vec<Arc<dyn AnyPool>> = self
            .pools
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect();

        for pool in pools {
            pool.shutdown().await?;
        }

        self.pools.clear();
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
}
