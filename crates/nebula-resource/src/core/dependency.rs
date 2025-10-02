//! Dependency resolution and ordering for resources
//!
//! This module provides dependency graph management for resources, including:
//! - Topological sorting for initialization order
//! - Circular dependency detection
//! - Dependency validation

use crate::core::error::{ResourceError, ResourceResult};
use crate::core::resource::ResourceId;
use std::collections::{HashMap, HashSet, VecDeque};

/// Dependency graph for managing resource initialization order
#[derive(Debug, Clone, Default)]
pub struct DependencyGraph {
    /// ResourceId -> list of dependencies (what this resource depends on)
    dependencies: HashMap<ResourceId, Vec<ResourceId>>,
    /// ResourceId -> list of dependents (what depends on this resource)
    dependents: HashMap<ResourceId, Vec<ResourceId>>,
}

impl DependencyGraph {
    /// Create a new empty dependency graph
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
        resource: ResourceId,
        depends_on: ResourceId,
    ) -> ResourceResult<()> {
        // Don't allow self-dependency
        if resource == depends_on {
            return Err(ResourceError::internal(
                resource.to_string(),
                format!("Resource cannot depend on itself: {}", resource),
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
                resource.to_string(),
                format!("Adding dependency would create cycle: {:?}", cycle),
            ));
        }

        Ok(())
    }

    /// Remove a dependency relationship
    fn remove_dependency(&mut self, resource: &ResourceId, depends_on: &ResourceId) {
        if let Some(deps) = self.dependencies.get_mut(resource) {
            deps.retain(|d| d != depends_on);
        }
        if let Some(deps) = self.dependents.get_mut(depends_on) {
            deps.retain(|d| d != resource);
        }
    }

    /// Get all dependencies for a resource
    pub fn get_dependencies(&self, resource: &ResourceId) -> Vec<ResourceId> {
        self.dependencies.get(resource).cloned().unwrap_or_default()
    }

    /// Get all dependents of a resource (what depends on this resource)
    pub fn get_dependents(&self, resource: &ResourceId) -> Vec<ResourceId> {
        self.dependents.get(resource).cloned().unwrap_or_default()
    }

    /// Detect if there's a cycle in the dependency graph
    ///
    /// # Returns
    /// Some(cycle_path) if a cycle is detected, None otherwise
    pub fn detect_cycle(&self) -> Option<Vec<ResourceId>> {
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();
        let mut path = Vec::new();

        for node in self.dependencies.keys() {
            if !visited.contains(node)
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
        node: &ResourceId,
        visited: &mut HashSet<ResourceId>,
        rec_stack: &mut HashSet<ResourceId>,
        path: &mut Vec<ResourceId>,
    ) -> Option<Vec<ResourceId>> {
        visited.insert(node.clone());
        rec_stack.insert(node.clone());
        path.push(node.clone());

        if let Some(deps) = self.dependencies.get(node) {
            for dep in deps {
                if !visited.contains(dep) {
                    if let Some(cycle) = self.detect_cycle_dfs(dep, visited, rec_stack, path) {
                        return Some(cycle);
                    }
                } else if rec_stack.contains(dep) {
                    // Found cycle - build cycle path
                    let cycle_start = path.iter().position(|p| p == dep).unwrap();
                    return Some(path[cycle_start..].to_vec());
                }
            }
        }

        rec_stack.remove(node);
        path.pop();
        None
    }

    /// Perform topological sort to get initialization order
    ///
    /// # Returns
    /// Ordered list of ResourceIds where dependencies come before dependents
    ///
    /// # Errors
    /// Returns error if there's a cycle in the graph
    pub fn topological_sort(&self) -> ResourceResult<Vec<ResourceId>> {
        // Use Kahn's algorithm
        let mut in_degree: HashMap<ResourceId, usize> = HashMap::new();
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
        let mut queue: VecDeque<ResourceId> = in_degree
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

            // For each dependent of this node
            if let Some(deps) = self.dependents.get(&node) {
                for dependent in deps {
                    if let Some(degree) = in_degree.get_mut(dependent) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push_back(dependent.clone());
                        }
                    }
                }
            }
        }

        // If we haven't sorted all nodes, there's a cycle
        if sorted.len() != all_nodes.len()
            && let Some(cycle) = self.detect_cycle() {
                return Err(ResourceError::internal(
                    cycle[0].to_string(),
                    format!("Circular dependency detected: {:?}", cycle),
                ));
            }

        Ok(sorted)
    }

    /// Get the initialization order for a specific resource and its dependencies
    pub fn get_init_order(&self, resource: &ResourceId) -> ResourceResult<Vec<ResourceId>> {
        let mut visited = HashSet::new();
        let mut order = Vec::new();

        self.build_init_order(resource, &mut visited, &mut order)?;

        Ok(order)
    }

    /// Recursively build initialization order using DFS
    fn build_init_order(
        &self,
        resource: &ResourceId,
        visited: &mut HashSet<ResourceId>,
        order: &mut Vec<ResourceId>,
    ) -> ResourceResult<()> {
        if visited.contains(resource) {
            return Ok(());
        }

        visited.insert(resource.clone());

        // Visit dependencies first
        if let Some(deps) = self.dependencies.get(resource) {
            for dep in deps {
                self.build_init_order(dep, visited, order)?;
            }
        }

        // Add this resource after its dependencies
        order.push(resource.clone());

        Ok(())
    }

    /// Get all transitive dependencies of a resource
    pub fn get_all_dependencies(&self, resource: &ResourceId) -> HashSet<ResourceId> {
        let mut all_deps = HashSet::new();
        self.collect_dependencies(resource, &mut all_deps);
        all_deps
    }

    /// Recursively collect all dependencies
    fn collect_dependencies(&self, resource: &ResourceId, collected: &mut HashSet<ResourceId>) {
        if let Some(deps) = self.dependencies.get(resource) {
            for dep in deps {
                if collected.insert(dep.clone()) {
                    self.collect_dependencies(dep, collected);
                }
            }
        }
    }

    /// Check if resource A depends on resource B (directly or transitively)
    pub fn depends_on(&self, resource: &ResourceId, depends_on: &ResourceId) -> bool {
        let all_deps = self.get_all_dependencies(resource);
        all_deps.contains(depends_on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rid(name: &str) -> ResourceId {
        ResourceId::new(name, "1.0")
    }

    #[test]
    fn test_add_simple_dependency() {
        let mut graph = DependencyGraph::new();
        let a = rid("a");
        let b = rid("b");

        graph.add_dependency(a.clone(), b.clone()).unwrap();

        assert_eq!(graph.get_dependencies(&a), vec![b.clone()]);
        assert_eq!(graph.get_dependents(&b), vec![a.clone()]);
    }

    #[test]
    fn test_self_dependency_rejected() {
        let mut graph = DependencyGraph::new();
        let a = rid("a");

        let result = graph.add_dependency(a.clone(), a.clone());
        assert!(result.is_err());
    }

    #[test]
    fn test_circular_dependency_detected() {
        let mut graph = DependencyGraph::new();
        let a = rid("a");
        let b = rid("b");
        let c = rid("c");

        // a -> b -> c is fine
        graph.add_dependency(a.clone(), b.clone()).unwrap();
        graph.add_dependency(b.clone(), c.clone()).unwrap();

        // c -> a creates a cycle
        let result = graph.add_dependency(c.clone(), a.clone());
        assert!(result.is_err());
    }

    #[test]
    fn test_topological_sort() {
        let mut graph = DependencyGraph::new();
        let a = rid("a");
        let b = rid("b");
        let c = rid("c");
        let d = rid("d");

        // a depends on b and c
        // b depends on d
        // c depends on d
        // Expected order: d, then b and c (in any order), then a
        graph.add_dependency(a.clone(), b.clone()).unwrap();
        graph.add_dependency(a.clone(), c.clone()).unwrap();
        graph.add_dependency(b.clone(), d.clone()).unwrap();
        graph.add_dependency(c.clone(), d.clone()).unwrap();

        let sorted = graph.topological_sort().unwrap();

        // d must come before b and c
        let d_pos = sorted.iter().position(|r| r == &d).unwrap();
        let b_pos = sorted.iter().position(|r| r == &b).unwrap();
        let c_pos = sorted.iter().position(|r| r == &c).unwrap();
        let a_pos = sorted.iter().position(|r| r == &a).unwrap();

        assert!(d_pos < b_pos);
        assert!(d_pos < c_pos);
        assert!(b_pos < a_pos);
        assert!(c_pos < a_pos);
    }

    #[test]
    fn test_get_init_order() {
        let mut graph = DependencyGraph::new();
        let a = rid("a");
        let b = rid("b");
        let c = rid("c");

        graph.add_dependency(a.clone(), b.clone()).unwrap();
        graph.add_dependency(b.clone(), c.clone()).unwrap();

        let order = graph.get_init_order(&a).unwrap();

        // Should be: c, b, a
        assert_eq!(order.len(), 3);
        assert_eq!(order[0], c);
        assert_eq!(order[1], b);
        assert_eq!(order[2], a);
    }

    #[test]
    fn test_transitive_dependencies() {
        let mut graph = DependencyGraph::new();
        let a = rid("a");
        let b = rid("b");
        let c = rid("c");

        graph.add_dependency(a.clone(), b.clone()).unwrap();
        graph.add_dependency(b.clone(), c.clone()).unwrap();

        let all_deps = graph.get_all_dependencies(&a);
        assert!(all_deps.contains(&b));
        assert!(all_deps.contains(&c));
        assert_eq!(all_deps.len(), 2);
    }

    #[test]
    fn test_depends_on() {
        let mut graph = DependencyGraph::new();
        let a = rid("a");
        let b = rid("b");
        let c = rid("c");

        graph.add_dependency(a.clone(), b.clone()).unwrap();
        graph.add_dependency(b.clone(), c.clone()).unwrap();

        assert!(graph.depends_on(&a, &b));
        assert!(graph.depends_on(&a, &c)); // transitive
        assert!(!graph.depends_on(&b, &a));
    }
}
