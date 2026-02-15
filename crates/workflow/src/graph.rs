//! DAG dependency graph built on `petgraph`.

use std::collections::HashMap;

use nebula_core::NodeId;
use petgraph::Direction;
use petgraph::algo;
use petgraph::graph::{DiGraph, NodeIndex};

use crate::connection::Connection;
use crate::definition::WorkflowDefinition;
use crate::error::WorkflowError;

/// A directed acyclic graph representing the execution dependencies between workflow nodes.
#[derive(Debug)]
pub struct DependencyGraph {
    graph: DiGraph<NodeId, Connection>,
    index_map: HashMap<NodeId, NodeIndex>,
}

impl DependencyGraph {
    /// Build a [`DependencyGraph`] from a [`WorkflowDefinition`].
    ///
    /// Returns an error if a connection references an unknown node or creates a self-loop.
    pub fn from_definition(definition: &WorkflowDefinition) -> Result<Self, WorkflowError> {
        let mut graph = DiGraph::new();
        let mut index_map = HashMap::new();

        for node in &definition.nodes {
            let idx = graph.add_node(node.id);
            index_map.insert(node.id, idx);
        }

        for conn in &definition.connections {
            let from_idx = index_map
                .get(&conn.from_node)
                .ok_or(WorkflowError::UnknownNode(conn.from_node))?;
            let to_idx = index_map
                .get(&conn.to_node)
                .ok_or(WorkflowError::UnknownNode(conn.to_node))?;
            if conn.from_node == conn.to_node {
                return Err(WorkflowError::SelfLoop(conn.from_node));
            }
            graph.add_edge(*from_idx, *to_idx, conn.clone());
        }

        Ok(Self { graph, index_map })
    }

    /// Returns `true` if the graph contains at least one cycle.
    #[must_use]
    pub fn has_cycle(&self) -> bool {
        petgraph::algo::is_cyclic_directed(&self.graph)
    }

    /// Topological sort of the graph. Returns an error if a cycle exists.
    pub fn topological_sort(&self) -> Result<Vec<NodeId>, WorkflowError> {
        let sorted = algo::toposort(&self.graph, None).map_err(|_| WorkflowError::CycleDetected)?;
        Ok(sorted.into_iter().map(|idx| self.graph[idx]).collect())
    }

    /// Compute parallel execution levels using Kahn's algorithm.
    ///
    /// Each level contains nodes whose predecessors all appear in earlier levels,
    /// meaning the nodes within a single level can execute concurrently.
    pub fn compute_levels(&self) -> Result<Vec<Vec<NodeId>>, WorkflowError> {
        let mut in_degree: HashMap<NodeIndex, usize> = HashMap::new();
        for idx in self.graph.node_indices() {
            in_degree.insert(
                idx,
                self.graph
                    .neighbors_directed(idx, Direction::Incoming)
                    .count(),
            );
        }

        let mut levels = Vec::new();
        let mut remaining: Vec<NodeIndex> = self.graph.node_indices().collect();

        while !remaining.is_empty() {
            let current_level: Vec<NodeIndex> = remaining
                .iter()
                .filter(|idx| in_degree[idx] == 0)
                .copied()
                .collect();

            if current_level.is_empty() {
                return Err(WorkflowError::CycleDetected);
            }

            for &idx in &current_level {
                for neighbor in self.graph.neighbors_directed(idx, Direction::Outgoing) {
                    in_degree.entry(neighbor).and_modify(|deg| *deg -= 1);
                }
            }

            remaining.retain(|idx| !current_level.contains(idx));
            for idx in &current_level {
                in_degree.remove(idx);
            }

            levels.push(
                current_level
                    .into_iter()
                    .map(|idx| self.graph[idx])
                    .collect(),
            );
        }

        Ok(levels)
    }

    /// Get all incoming connections (edges pointing TO this node).
    #[must_use]
    pub fn incoming_connections(&self, id: NodeId) -> Vec<&Connection> {
        let Some(&idx) = self.index_map.get(&id) else {
            return Vec::new();
        };
        self.graph
            .edges_directed(idx, Direction::Incoming)
            .map(|e| e.weight())
            .collect()
    }

    /// Get all outgoing connections (edges leaving FROM this node).
    #[must_use]
    pub fn outgoing_connections(&self, id: NodeId) -> Vec<&Connection> {
        let Some(&idx) = self.index_map.get(&id) else {
            return Vec::new();
        };
        self.graph
            .edges_directed(idx, Direction::Outgoing)
            .map(|e| e.weight())
            .collect()
    }

    /// Nodes with no incoming edges (start points of the DAG).
    #[must_use]
    pub fn entry_nodes(&self) -> Vec<NodeId> {
        self.graph
            .node_indices()
            .filter(|&idx| {
                self.graph
                    .neighbors_directed(idx, Direction::Incoming)
                    .count()
                    == 0
            })
            .map(|idx| self.graph[idx])
            .collect()
    }

    /// Nodes with no outgoing edges (end points of the DAG).
    #[must_use]
    pub fn exit_nodes(&self) -> Vec<NodeId> {
        self.graph
            .node_indices()
            .filter(|&idx| {
                self.graph
                    .neighbors_directed(idx, Direction::Outgoing)
                    .count()
                    == 0
            })
            .map(|idx| self.graph[idx])
            .collect()
    }

    /// Get the predecessor (upstream) node IDs of a given node.
    #[must_use]
    pub fn predecessors(&self, id: NodeId) -> Vec<NodeId> {
        if let Some(&idx) = self.index_map.get(&id) {
            self.graph
                .neighbors_directed(idx, Direction::Incoming)
                .map(|i| self.graph[i])
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get the successor (downstream) node IDs of a given node.
    #[must_use]
    pub fn successors(&self, id: NodeId) -> Vec<NodeId> {
        if let Some(&idx) = self.index_map.get(&id) {
            self.graph
                .neighbors_directed(idx, Direction::Outgoing)
                .map(|i| self.graph[i])
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Validate the graph structure: no cycles and at least one entry node.
    pub fn validate(&self) -> Result<(), WorkflowError> {
        if self.has_cycle() {
            return Err(WorkflowError::CycleDetected);
        }
        if self.entry_nodes().is_empty() {
            return Err(WorkflowError::NoEntryNodes);
        }
        Ok(())
    }

    /// Number of nodes in the graph.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Connection;
    use crate::definition::{WorkflowConfig, WorkflowDefinition};
    use crate::node::NodeDefinition;
    use chrono::Utc;
    use nebula_core::{ActionId, NodeId, Version, WorkflowId};
    use std::collections::HashMap;

    /// Helper: build a minimal `WorkflowDefinition` from nodes and connections.
    fn make_definition(
        nodes: Vec<NodeDefinition>,
        connections: Vec<Connection>,
    ) -> WorkflowDefinition {
        let now = Utc::now();
        WorkflowDefinition {
            id: WorkflowId::v4(),
            name: "test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes,
            connections,
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn node(id: NodeId) -> NodeDefinition {
        NodeDefinition::new(id, "n", ActionId::v4())
    }

    // --- linear graph: A -> B -> C ---

    fn linear_ids() -> (NodeId, NodeId, NodeId) {
        (NodeId::v4(), NodeId::v4(), NodeId::v4())
    }

    fn linear_definition(a: NodeId, b: NodeId, c: NodeId) -> WorkflowDefinition {
        make_definition(
            vec![node(a), node(b), node(c)],
            vec![Connection::new(a, b), Connection::new(b, c)],
        )
    }

    // --- diamond graph: A -> B, A -> C, B -> D, C -> D ---

    fn diamond_ids() -> (NodeId, NodeId, NodeId, NodeId) {
        (NodeId::v4(), NodeId::v4(), NodeId::v4(), NodeId::v4())
    }

    fn diamond_definition(a: NodeId, b: NodeId, c: NodeId, d: NodeId) -> WorkflowDefinition {
        make_definition(
            vec![node(a), node(b), node(c), node(d)],
            vec![
                Connection::new(a, b),
                Connection::new(a, c),
                Connection::new(b, d),
                Connection::new(c, d),
            ],
        )
    }

    #[test]
    fn from_definition_linear() {
        let (a, b, c) = linear_ids();
        let def = linear_definition(a, b, c);
        let graph = DependencyGraph::from_definition(&def).unwrap();
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);
    }

    #[test]
    fn from_definition_diamond() {
        let (a, b, c, d) = diamond_ids();
        let def = diamond_definition(a, b, c, d);
        let graph = DependencyGraph::from_definition(&def).unwrap();
        assert_eq!(graph.node_count(), 4);
        assert_eq!(graph.edge_count(), 4);
    }

    #[test]
    fn from_definition_rejects_unknown_node() {
        let a = NodeId::v4();
        let unknown = NodeId::v4();
        let def = make_definition(vec![node(a)], vec![Connection::new(a, unknown)]);
        let err = DependencyGraph::from_definition(&def).unwrap_err();
        assert!(matches!(err, WorkflowError::UnknownNode(_)));
    }

    #[test]
    fn from_definition_rejects_self_loop() {
        let a = NodeId::v4();
        let def = make_definition(vec![node(a)], vec![Connection::new(a, a)]);
        let err = DependencyGraph::from_definition(&def).unwrap_err();
        assert!(matches!(err, WorkflowError::SelfLoop(_)));
    }

    #[test]
    fn has_cycle_detects_cycle() {
        let a = NodeId::v4();
        let b = NodeId::v4();
        let def = make_definition(
            vec![node(a), node(b)],
            vec![Connection::new(a, b), Connection::new(b, a)],
        );
        let graph = DependencyGraph::from_definition(&def).unwrap();
        assert!(graph.has_cycle());
    }

    #[test]
    fn has_cycle_false_for_dag() {
        let (a, b, c) = linear_ids();
        let def = linear_definition(a, b, c);
        let graph = DependencyGraph::from_definition(&def).unwrap();
        assert!(!graph.has_cycle());
    }

    #[test]
    fn topological_sort_linear() {
        let (a, b, c) = linear_ids();
        let def = linear_definition(a, b, c);
        let graph = DependencyGraph::from_definition(&def).unwrap();
        let sorted = graph.topological_sort().unwrap();
        assert_eq!(sorted, vec![a, b, c]);
    }

    #[test]
    fn topological_sort_diamond() {
        let (a, b, c, d) = diamond_ids();
        let def = diamond_definition(a, b, c, d);
        let graph = DependencyGraph::from_definition(&def).unwrap();
        let sorted = graph.topological_sort().unwrap();

        // a must come first, d must come last
        assert_eq!(sorted[0], a);
        assert_eq!(sorted[3], d);
        // b and c are in positions 1-2 in some order
        assert!(sorted[1..3].contains(&b));
        assert!(sorted[1..3].contains(&c));
    }

    #[test]
    fn compute_levels_linear() {
        let (a, b, c) = linear_ids();
        let def = linear_definition(a, b, c);
        let graph = DependencyGraph::from_definition(&def).unwrap();
        let levels = graph.compute_levels().unwrap();

        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0], vec![a]);
        assert_eq!(levels[1], vec![b]);
        assert_eq!(levels[2], vec![c]);
    }

    #[test]
    fn compute_levels_diamond() {
        let (a, b, c, d) = diamond_ids();
        let def = diamond_definition(a, b, c, d);
        let graph = DependencyGraph::from_definition(&def).unwrap();
        let levels = graph.compute_levels().unwrap();

        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0], vec![a]);
        // b and c should be in the same level (parallel)
        assert_eq!(levels[1].len(), 2);
        assert!(levels[1].contains(&b));
        assert!(levels[1].contains(&c));
        assert_eq!(levels[2], vec![d]);
    }

    #[test]
    fn entry_and_exit_nodes() {
        let (a, b, c, d) = diamond_ids();
        let def = diamond_definition(a, b, c, d);
        let graph = DependencyGraph::from_definition(&def).unwrap();

        let entries = graph.entry_nodes();
        assert_eq!(entries.len(), 1);
        assert!(entries.contains(&a));

        let exits = graph.exit_nodes();
        assert_eq!(exits.len(), 1);
        assert!(exits.contains(&d));
    }

    #[test]
    fn predecessors_and_successors() {
        let (a, b, c, d) = diamond_ids();
        let def = diamond_definition(a, b, c, d);
        let graph = DependencyGraph::from_definition(&def).unwrap();

        // a has no predecessors, two successors
        assert!(graph.predecessors(a).is_empty());
        let a_succ = graph.successors(a);
        assert_eq!(a_succ.len(), 2);
        assert!(a_succ.contains(&b));
        assert!(a_succ.contains(&c));

        // d has two predecessors, no successors
        let d_pred = graph.predecessors(d);
        assert_eq!(d_pred.len(), 2);
        assert!(d_pred.contains(&b));
        assert!(d_pred.contains(&c));
        assert!(graph.successors(d).is_empty());
    }

    #[test]
    fn predecessors_unknown_node_returns_empty() {
        let a = NodeId::v4();
        let def = make_definition(vec![node(a)], vec![]);
        let graph = DependencyGraph::from_definition(&def).unwrap();
        assert!(graph.predecessors(NodeId::v4()).is_empty());
    }

    #[test]
    fn validate_valid_dag() {
        let (a, b, c) = linear_ids();
        let def = linear_definition(a, b, c);
        let graph = DependencyGraph::from_definition(&def).unwrap();
        assert!(graph.validate().is_ok());
    }

    #[test]
    fn validate_cyclic_graph() {
        let a = NodeId::v4();
        let b = NodeId::v4();
        let def = make_definition(
            vec![node(a), node(b)],
            vec![Connection::new(a, b), Connection::new(b, a)],
        );
        let graph = DependencyGraph::from_definition(&def).unwrap();
        let err = graph.validate().unwrap_err();
        assert!(matches!(err, WorkflowError::CycleDetected));
    }
}
