//! Dependency graph for managing resource initialization order.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::{Error, Result};

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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
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
