//! Resource manager — central registry, pool orchestration, and dependency ordering.

use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;

use crate::context::ResourceContext;
use crate::error::{ResourceError, ResourceResult};
use crate::pool::{Pool, PoolConfig};
use crate::resource::{Resource, ResourceGuard};

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
    ) -> ResourceResult<()> {
        let resource = resource.into();
        let depends_on = depends_on.into();

        // Don't allow self-dependency
        if resource == depends_on {
            return Err(ResourceError::internal(
                &resource,
                format!("Resource cannot depend on itself: {resource}"),
            ));
        }

        // Add to dependencies map
        self.dependencies
            .entry(resource.clone())
            .or_default()
            .push(depends_on.clone());

        // Add to dependents map
        self.dependents
            .entry(depends_on.clone())
            .or_default()
            .push(resource.clone());

        // Check for cycles after adding
        if let Some(cycle) = self.detect_cycle() {
            // Rollback the changes
            self.remove_dependency(&resource, &depends_on);
            return Err(ResourceError::internal(
                &resource,
                format!("Adding dependency would create cycle: {cycle:?}"),
            ));
        }

        Ok(())
    }

    /// Remove a dependency relationship
    fn remove_dependency(&mut self, resource: &str, depends_on: &str) {
        if let Some(deps) = self.dependencies.get_mut(resource) {
            deps.retain(|d| d != depends_on);
        }
        if let Some(deps) = self.dependents.get_mut(depends_on) {
            deps.retain(|d| d != resource);
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
                let cycle_start = path.iter().position(|p| p == dep).unwrap();
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
    pub fn topological_sort(&self) -> ResourceResult<Vec<String>> {
        // Use Kahn's algorithm
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut all_nodes = HashSet::new();

        // Collect all nodes and calculate in-degrees
        for (node, deps) in &self.dependencies {
            all_nodes.insert(node.clone());
            in_degree.entry(node.clone()).or_insert(0);

            for dep in deps {
                all_nodes.insert(dep.clone());
                *in_degree.entry(dep.clone()).or_insert(0) += 0; // Ensure it exists
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
            return Err(ResourceError::internal(
                &cycle[0],
                format!("Circular dependency detected: {cycle:?}"),
            ));
        }

        Ok(sorted)
    }

    /// Get the initialization order for a specific resource and its dependencies
    pub fn get_init_order(&self, resource: &str) -> ResourceResult<Vec<String>> {
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
    ) -> ResourceResult<()> {
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
// Type-erased pool wrapper
// ---------------------------------------------------------------------------

/// Type-erased pool interface so the manager can store pools of different
/// resource types in a single map.
#[async_trait]
trait AnyPool: Send + Sync {
    /// Acquire a type-erased instance wrapped in `Arc<dyn Any>`.
    async fn acquire_any(
        &self,
        ctx: &ResourceContext,
    ) -> ResourceResult<ResourceGuard<Arc<dyn Any + Send + Sync>>>;

    /// Shut down the pool.
    async fn shutdown(&self) -> ResourceResult<()>;
}

/// Concrete adapter from `Pool<R>` to `AnyPool`.
struct TypedPool<R: Resource> {
    pool: Pool<R>,
}

#[async_trait]
impl<R: Resource> AnyPool for TypedPool<R> {
    async fn acquire_any(
        &self,
        ctx: &ResourceContext,
    ) -> ResourceResult<ResourceGuard<Arc<dyn Any + Send + Sync>>> {
        let guard = self.pool.acquire(ctx).await?;
        let instance: R::Instance = guard.into_inner();
        let arc_instance: Arc<dyn Any + Send + Sync> = Arc::new(instance);
        // No return-to-pool for the type-erased path — the instance is consumed.
        // For full pool recycling, callers should use Pool<R> directly.
        Ok(ResourceGuard::new(arc_instance, |_| {}))
    }

    async fn shutdown(&self) -> ResourceResult<()> {
        self.pool.shutdown().await
    }
}

// ---------------------------------------------------------------------------
// ResourceManager
// ---------------------------------------------------------------------------

/// Central manager for resource pools and dependency ordering.
pub struct ResourceManager {
    /// Pools indexed by TypeId of the Resource implementation.
    pools: DashMap<TypeId, Arc<dyn AnyPool>>,
    /// Dependency graph for initialization ordering.
    deps: parking_lot::RwLock<DependencyGraph>,
}

impl ResourceManager {
    /// Create a new empty resource manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pools: DashMap::new(),
            deps: parking_lot::RwLock::new(DependencyGraph::new()),
        }
    }

    /// Register a resource type with its config and pool settings.
    ///
    /// The pool is created immediately but no instances are pre-warmed.
    pub fn register<R: Resource>(
        &self,
        resource: R,
        config: R::Config,
        pool_config: PoolConfig,
    ) -> ResourceResult<()> {
        let type_id = TypeId::of::<R>();
        let id = resource.id().to_string();

        // Register dependencies
        {
            let mut deps = self.deps.write();
            for dep in resource.dependencies() {
                deps.add_dependency(&id, dep)?;
            }
        }

        let pool = Pool::new(resource, config, pool_config);
        let any_pool: Arc<dyn AnyPool> = Arc::new(TypedPool { pool });
        self.pools.insert(type_id, any_pool);
        Ok(())
    }

    /// Acquire a resource instance by resource type.
    ///
    /// The caller specifies the Resource implementation type `R` and gets
    /// back a guard wrapping `R::Instance`.
    pub async fn acquire<R: Resource>(
        &self,
        ctx: &ResourceContext,
    ) -> ResourceResult<ResourceGuard<Arc<dyn Any + Send + Sync>>> {
        let type_id = TypeId::of::<R>();
        let pool = self.pools.get(&type_id).ok_or_else(|| {
            ResourceError::unavailable("unknown", "Resource type not registered", false)
        })?;

        pool.acquire_any(ctx).await
    }

    /// Get the initialization order based on dependency graph.
    pub fn initialization_order(&self) -> ResourceResult<Vec<String>> {
        self.deps.read().topological_sort()
    }

    /// Shut down all registered pools.
    pub async fn shutdown(&self) -> ResourceResult<()> {
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

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ResourceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceManager")
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::ResourceConfig;
    use crate::scope::ResourceScope;

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TestConfig {
        value: String,
    }

    impl ResourceConfig for TestConfig {
        fn validate(&self) -> ResourceResult<()> {
            if self.value.is_empty() {
                return Err(ResourceError::configuration("value cannot be empty"));
            }
            Ok(())
        }
    }

    struct TestResource;

    #[async_trait]
    impl Resource for TestResource {
        type Config = TestConfig;
        type Instance = String;

        fn id(&self) -> &str {
            "test"
        }

        async fn create(
            &self,
            config: &Self::Config,
            _ctx: &ResourceContext,
        ) -> ResourceResult<Self::Instance> {
            Ok(format!("instance-{}", config.value))
        }
    }

    fn ctx() -> ResourceContext {
        ResourceContext::new(ResourceScope::Global, "wf", "ex")
    }

    #[tokio::test]
    async fn register_and_acquire() {
        let mgr = ResourceManager::new();
        let config = TestConfig {
            value: "hello".into(),
        };
        mgr.register(TestResource, config, PoolConfig::default())
            .unwrap();

        let guard = mgr.acquire::<TestResource>(&ctx()).await.unwrap();
        let instance = guard
            .downcast_ref::<String>()
            .expect("should downcast to String");
        assert_eq!(instance, "instance-hello");
    }

    #[tokio::test]
    async fn acquire_unregistered_fails() {
        let mgr = ResourceManager::new();
        let result = mgr.acquire::<TestResource>(&ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shutdown_clears_pools() {
        let mgr = ResourceManager::new();
        let config = TestConfig { value: "x".into() };
        mgr.register(TestResource, config, PoolConfig::default())
            .unwrap();
        mgr.shutdown().await.unwrap();
        assert!(mgr.pools.is_empty());
    }
}
