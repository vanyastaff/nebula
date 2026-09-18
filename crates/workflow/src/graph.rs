//! DAG dependency graph built on `petgraph`.

use std::collections::HashMap;

use nebula_core::NodeKey;
use petgraph::{
    Direction, algo,
    graph::{DiGraph, NodeIndex},
};

use crate::{
    NodeDefinition, connection::Connection, definition::WorkflowDefinition, error::WorkflowError,
};

/// A directed acyclic graph representing the execution dependencies between workflow nodes.
#[derive(Debug)]
pub struct DependencyGraph {
    graph: DiGraph<NodeKey, Connection>,
    index_map: HashMap<NodeKey, NodeIndex>,
}

impl DependencyGraph {
    /// Build a [`DependencyGraph`] from a [`WorkflowDefinition`].
    ///
    /// Returns an error if a connection references an unknown node or creates a self-loop.
    pub fn from_definition(definition: &WorkflowDefinition) -> Result<Self, WorkflowError> {
        Self::from_parts(&definition.nodes, &definition.connections)
    }

    /// Build dependency topology directly from scheduler nodes and connections.
    ///
    /// # Errors
    ///
    /// Rejects duplicate node keys, unknown endpoints, and self-loops. As with
    /// [`Self::from_definition`], cycle detection remains in [`Self::validate`]
    /// and the topological traversal methods.
    pub fn from_parts(
        nodes: &[NodeDefinition],
        connections: &[Connection],
    ) -> Result<Self, WorkflowError> {
        let mut graph = DiGraph::new();
        let mut index_map = HashMap::new();

        for node in nodes {
            let id = node.id.clone();
            let idx = graph.add_node(id.clone());
            // `validate_workflow` also checks duplicates, but plan builders
            // call this constructor directly without that pass, so an
            // unchecked `insert` would orphan the first of two duplicate
            // nodes inside `petgraph` while silently losing it from the
            // index map. Fail loudly here.
            if index_map.insert(id.clone(), idx).is_some() {
                return Err(WorkflowError::DuplicateNodeKey(id));
            }
        }

        for conn in connections {
            let from_idx = index_map
                .get(&conn.from_node)
                .ok_or_else(|| WorkflowError::UnknownNode(conn.from_node.clone()))?;
            let to_idx = index_map
                .get(&conn.to_node)
                .ok_or_else(|| WorkflowError::UnknownNode(conn.to_node.clone()))?;
            if conn.is_self_loop() {
                return Err(WorkflowError::SelfLoop(conn.from_node.clone()));
            }
            graph.add_edge(*from_idx, *to_idx, conn.clone());
        }

        Ok(Self { graph, index_map })
    }

    /// Returns `true` if the graph contains at least one cycle.
    #[must_use]
    pub fn has_cycle(&self) -> bool {
        algo::is_cyclic_directed(&self.graph)
    }

    /// Topological sort of the graph. Returns an error if a cycle exists.
    pub fn topological_sort(&self) -> Result<Vec<NodeKey>, WorkflowError> {
        let sorted = algo::toposort(&self.graph, None).map_err(|_| WorkflowError::CycleDetected)?;
        Ok(self.collect_node_keys(sorted))
    }

    /// Collect [`NodeKey`]s from a sequence of node indices.
    fn collect_node_keys(&self, indices: impl IntoIterator<Item = NodeIndex>) -> Vec<NodeKey> {
        indices
            .into_iter()
            .map(|idx| self.graph[idx].clone())
            .collect()
    }

    /// Compute parallel execution levels using Kahn's algorithm.
    ///
    /// Each level contains nodes whose predecessors all appear in earlier levels,
    /// meaning the nodes within a single level can execute concurrently.
    pub fn compute_levels(&self) -> Result<Vec<Vec<NodeKey>>, WorkflowError> {
        let mut in_degree = self.compute_in_degrees();
        let mut remaining: Vec<NodeIndex> = self.graph.node_indices().collect();
        let mut levels = Vec::new();

        while !remaining.is_empty() {
            let current_level = Self::next_level(&remaining, &in_degree)?;
            self.decrement_successor_degrees(&current_level, &mut in_degree);
            remaining.retain(|idx| !current_level.contains(idx));
            levels.push(self.collect_node_keys(current_level));
        }

        Ok(levels)
    }

    /// Build the initial in-degree map for Kahn's algorithm.
    fn compute_in_degrees(&self) -> HashMap<NodeIndex, usize> {
        self.graph
            .node_indices()
            .map(|idx| (idx, self.in_degree(idx)))
            .collect()
    }

    /// Count incoming edges for a single node.
    fn in_degree(&self, idx: NodeIndex) -> usize {
        self.graph
            .neighbors_directed(idx, Direction::Incoming)
            .count()
    }

    /// Pick all remaining nodes whose in-degree has dropped to zero.
    ///
    /// Returns an error if no zero-in-degree nodes remain — that means the
    /// graph contains a cycle.
    fn next_level(
        remaining: &[NodeIndex],
        in_degree: &HashMap<NodeIndex, usize>,
    ) -> Result<Vec<NodeIndex>, WorkflowError> {
        let level: Vec<NodeIndex> = remaining
            .iter()
            .copied()
            .filter(|idx| in_degree.get(idx).copied().unwrap_or(0) == 0)
            .collect();

        if level.is_empty() {
            return Err(WorkflowError::CycleDetected);
        }
        Ok(level)
    }

    /// Decrement in-degrees for the direct successors of every node in the
    /// just-completed level and remove those nodes from the degree map.
    fn decrement_successor_degrees(
        &self,
        level: &[NodeIndex],
        in_degree: &mut HashMap<NodeIndex, usize>,
    ) {
        for &idx in level {
            for neighbor in self.graph.neighbors_directed(idx, Direction::Outgoing) {
                in_degree.entry(neighbor).and_modify(|deg| *deg -= 1);
            }
            in_degree.remove(&idx);
        }
    }

    /// Get all incoming connections (edges pointing TO this node).
    #[must_use]
    pub fn incoming_connections(&self, id: NodeKey) -> Vec<&Connection> {
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
    pub fn outgoing_connections(&self, id: NodeKey) -> Vec<&Connection> {
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
    pub fn entry_nodes(&self) -> Vec<NodeKey> {
        self.collect_node_keys(
            self.graph
                .node_indices()
                .filter(|&idx| self.in_degree(idx) == 0),
        )
    }

    /// Nodes with no outgoing edges (end points of the DAG).
    #[must_use]
    pub fn exit_nodes(&self) -> Vec<NodeKey> {
        self.collect_node_keys(
            self.graph
                .node_indices()
                .filter(|&idx| self.out_degree(idx) == 0),
        )
    }

    /// Count outgoing edges for a single node.
    fn out_degree(&self, idx: NodeIndex) -> usize {
        self.graph
            .neighbors_directed(idx, Direction::Outgoing)
            .count()
    }

    /// Look up a node index and collect its neighbors in one direction.
    fn neighbors(&self, id: NodeKey, direction: Direction) -> Vec<NodeKey> {
        let Some(&idx) = self.index_map.get(&id) else {
            return Vec::new();
        };
        self.collect_node_keys(self.graph.neighbors_directed(idx, direction))
    }

    /// Get the predecessor (upstream) node IDs of a given node.
    #[must_use]
    pub fn predecessors(&self, id: NodeKey) -> Vec<NodeKey> {
        self.neighbors(id, Direction::Incoming)
    }

    /// Get the successor (downstream) node IDs of a given node.
    #[must_use]
    pub fn successors(&self, id: NodeKey) -> Vec<NodeKey> {
        self.neighbors(id, Direction::Outgoing)
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
    use std::collections::HashMap;

    use chrono::Utc;
    use nebula_core::{NodeKey, WorkflowId, node_key};

    use super::*;
    use crate::{
        Version,
        connection::Connection,
        definition::{CURRENT_SCHEMA_VERSION, WorkflowConfig, WorkflowDefinition},
        node::NodeDefinition,
    };

    /// Helper: build a minimal `WorkflowDefinition` from nodes and connections.
    fn make_definition(
        nodes: Vec<NodeDefinition>,
        connections: Vec<Connection>,
    ) -> WorkflowDefinition {
        let now = Utc::now();
        WorkflowDefinition {
            id: WorkflowId::new(),
            name: "test".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes,
            connections,
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            trigger_bindings: Vec::new(),
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: CURRENT_SCHEMA_VERSION,
        }
    }

    fn node(id: NodeKey) -> NodeDefinition {
        NodeDefinition::new(id, "n", "core", "n").unwrap()
    }

    // --- linear graph: A -> B -> C ---

    fn linear_ids() -> (NodeKey, NodeKey, NodeKey) {
        (node_key!("a"), node_key!("b"), node_key!("c"))
    }

    fn linear_definition(a: NodeKey, b: NodeKey, c: NodeKey) -> WorkflowDefinition {
        make_definition(
            vec![node(a.clone()), node(b.clone()), node(c.clone())],
            vec![Connection::new(a, b.clone()), Connection::new(b, c)],
        )
    }

    // --- diamond graph: A -> B, A -> C, B -> D, C -> D ---

    fn diamond_ids() -> (NodeKey, NodeKey, NodeKey, NodeKey) {
        (
            node_key!("a"),
            node_key!("b"),
            node_key!("c"),
            node_key!("d"),
        )
    }

    fn diamond_definition(a: NodeKey, b: NodeKey, c: NodeKey, d: NodeKey) -> WorkflowDefinition {
        make_definition(
            vec![
                node(a.clone()),
                node(b.clone()),
                node(c.clone()),
                node(d.clone()),
            ],
            vec![
                Connection::new(a.clone(), b.clone()),
                Connection::new(a, c.clone()),
                Connection::new(b, d.clone()),
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
        let a = node_key!("a");
        let unknown = node_key!("unknown");
        let def = make_definition(vec![node(a.clone())], vec![Connection::new(a, unknown)]);
        let err = DependencyGraph::from_definition(&def).unwrap_err();
        assert!(matches!(err, WorkflowError::UnknownNode(_)));
    }

    #[test]
    fn from_definition_rejects_self_loop() {
        let a = node_key!("a");
        let def = make_definition(vec![node(a.clone())], vec![Connection::new(a.clone(), a)]);
        let err = DependencyGraph::from_definition(&def).unwrap_err();
        assert!(matches!(err, WorkflowError::SelfLoop(_)));
    }

    #[test]
    fn has_cycle_detects_cycle() {
        let a = node_key!("a");
        let b = node_key!("b");
        let def = make_definition(
            vec![node(a.clone()), node(b.clone())],
            vec![Connection::new(a.clone(), b.clone()), Connection::new(b, a)],
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
        let def = linear_definition(a.clone(), b.clone(), c.clone());
        let graph = DependencyGraph::from_definition(&def).unwrap();
        let sorted = graph.topological_sort().unwrap();
        assert_eq!(sorted, vec![a, b, c]);
    }

    #[test]
    fn topological_sort_diamond() {
        let (a, b, c, d) = diamond_ids();
        let def = diamond_definition(a.clone(), b.clone(), c.clone(), d.clone());
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
        let def = linear_definition(a.clone(), b.clone(), c.clone());
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
        let def = diamond_definition(a.clone(), b.clone(), c.clone(), d.clone());
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
        let def = diamond_definition(a.clone(), b, c, d.clone());
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
        let def = diamond_definition(a.clone(), b.clone(), c.clone(), d.clone());
        let graph = DependencyGraph::from_definition(&def).unwrap();

        // a has no predecessors, two successors
        assert!(graph.predecessors(a.clone()).is_empty());
        let a_succ = graph.successors(a);
        assert_eq!(a_succ.len(), 2);
        assert!(a_succ.contains(&b));
        assert!(a_succ.contains(&c));

        // d has two predecessors, no successors
        let d_pred = graph.predecessors(d.clone());
        assert_eq!(d_pred.len(), 2);
        assert!(d_pred.contains(&b));
        assert!(d_pred.contains(&c));
        assert!(graph.successors(d).is_empty());
    }

    #[test]
    fn predecessors_unknown_node_returns_empty() {
        let a = node_key!("a");
        let def = make_definition(vec![node(a)], vec![]);
        let graph = DependencyGraph::from_definition(&def).unwrap();
        assert!(graph.predecessors(node_key!("test")).is_empty());
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
        let a = node_key!("a");
        let b = node_key!("b");
        let def = make_definition(
            vec![node(a.clone()), node(b.clone())],
            vec![Connection::new(a.clone(), b.clone()), Connection::new(b, a)],
        );
        let graph = DependencyGraph::from_definition(&def).unwrap();
        let err = graph.validate().unwrap_err();
        assert!(matches!(err, WorkflowError::CycleDetected));
    }
}

#[cfg(test)]
mod parts_tests {
    use super::*;
    use crate::NodeDefinition;
    use nebula_core::node_key;

    #[test]
    fn from_parts_preserves_edges_and_rejects_invalid_structure() {
        let a = NodeDefinition::new(node_key!("a"), "A", "core", "echo").unwrap();
        let b = NodeDefinition::new(node_key!("b"), "B", "core", "echo").unwrap();
        let edge = Connection::new(a.id.clone(), b.id.clone());
        let graph =
            DependencyGraph::from_parts(&[a.clone(), b.clone()], std::slice::from_ref(&edge))
                .unwrap();
        assert_eq!(
            graph.topological_sort().unwrap(),
            vec![a.id.clone(), b.id.clone()]
        );
        assert_eq!(graph.outgoing_connections(a.id.clone()), vec![&edge]);
        assert!(matches!(
            DependencyGraph::from_parts(&[a.clone(), a.clone()], &[]),
            Err(WorkflowError::DuplicateNodeKey(_))
        ));
        assert!(matches!(
            DependencyGraph::from_parts(std::slice::from_ref(&a), std::slice::from_ref(&edge)),
            Err(WorkflowError::UnknownNode(_))
        ));
        assert!(matches!(
            DependencyGraph::from_parts(
                std::slice::from_ref(&a),
                &[Connection::new(a.id.clone(), a.id.clone())]
            ),
            Err(WorkflowError::SelfLoop(_))
        ));
        let cycle = DependencyGraph::from_parts(
            &[a.clone(), b.clone()],
            &[edge, Connection::new(b.id, a.id)],
        )
        .unwrap();
        assert!(matches!(
            cycle.validate(),
            Err(WorkflowError::CycleDetected)
        ));
    }
}
