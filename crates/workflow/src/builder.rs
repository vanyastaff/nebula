//! Fluent builder for constructing and validating workflow definitions.

use std::collections::HashMap;
use std::time::Duration;

use chrono::Utc;
use nebula_core::{Version, WorkflowId};

use crate::connection::{Connection, EdgeCondition};
use crate::definition::{WorkflowConfig, WorkflowDefinition};
use crate::error::WorkflowError;
use crate::graph::DependencyGraph;
use crate::node::NodeDefinition;
use nebula_core::NodeId;

/// A builder that accumulates nodes, connections, and configuration, then validates
/// and produces a [`WorkflowDefinition`].
pub struct WorkflowBuilder {
    id: WorkflowId,
    name: String,
    description: Option<String>,
    version: Version,
    nodes: Vec<NodeDefinition>,
    connections: Vec<Connection>,
    variables: HashMap<String, serde_json::Value>,
    config: WorkflowConfig,
    tags: Vec<String>,
}

impl WorkflowBuilder {
    /// Start building a workflow with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: WorkflowId::v4(),
            name: name.into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: Vec::new(),
            connections: Vec::new(),
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            tags: Vec::new(),
        }
    }

    /// Override the auto-generated workflow ID.
    #[must_use]
    pub fn id(mut self, id: WorkflowId) -> Self {
        self.id = id;
        self
    }

    /// Set the workflow description.
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Set the workflow version.
    #[must_use]
    pub fn version(mut self, version: Version) -> Self {
        self.version = version;
        self
    }

    /// Add a node to the workflow.
    #[must_use]
    pub fn add_node(mut self, node: NodeDefinition) -> Self {
        self.nodes.push(node);
        self
    }

    /// Add an unconditional connection between two nodes.
    #[must_use]
    pub fn connect(mut self, from: NodeId, to: NodeId) -> Self {
        self.connections.push(Connection::new(from, to));
        self
    }

    /// Add a conditional connection between two nodes.
    #[must_use]
    pub fn connect_with_condition(
        mut self,
        from: NodeId,
        to: NodeId,
        condition: EdgeCondition,
    ) -> Self {
        self.connections
            .push(Connection::new(from, to).with_condition(condition));
        self
    }

    /// Set a workflow-level variable.
    #[must_use]
    pub fn variable(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.variables.insert(key.into(), value);
        self
    }

    /// Set the workflow timeout.
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = Some(timeout);
        self
    }

    /// Set the maximum number of nodes that may execute in parallel.
    #[must_use]
    pub fn max_parallel(mut self, max: usize) -> Self {
        self.config.max_parallel_nodes = max;
        self
    }

    /// Add a tag.
    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Consume the builder, validate the workflow, and return the definition.
    ///
    /// Validation includes: non-empty name, at least one node, no duplicate IDs,
    /// no self-loops, and a valid DAG structure.
    pub fn build(self) -> Result<WorkflowDefinition, WorkflowError> {
        if self.name.is_empty() {
            return Err(WorkflowError::EmptyName);
        }
        if self.nodes.is_empty() {
            return Err(WorkflowError::NoNodes);
        }

        // Check duplicate node IDs
        let mut seen = std::collections::HashSet::new();
        for node in &self.nodes {
            if !seen.insert(node.id) {
                return Err(WorkflowError::DuplicateNodeId(node.id));
            }
        }

        // Check self-loops
        for conn in &self.connections {
            if conn.is_self_loop() {
                return Err(WorkflowError::SelfLoop(conn.from_node));
            }
        }

        let now = Utc::now();
        let definition = WorkflowDefinition {
            id: self.id,
            name: self.name,
            description: self.description,
            version: self.version,
            nodes: self.nodes,
            connections: self.connections,
            variables: self.variables,
            config: self.config,
            tags: self.tags,
            created_at: now,
            updated_at: now,
        };

        // Validate graph structure
        let graph = DependencyGraph::from_definition(&definition)?;
        graph.validate()?;

        Ok(definition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::{ActionId, NodeId};

    fn node(id: NodeId) -> NodeDefinition {
        NodeDefinition::new(id, "n", ActionId::v4())
    }

    #[test]
    fn build_linear_workflow() {
        let a = NodeId::v4();
        let b = NodeId::v4();
        let c = NodeId::v4();

        let def = WorkflowBuilder::new("linear")
            .add_node(node(a))
            .add_node(node(b))
            .add_node(node(c))
            .connect(a, b)
            .connect(b, c)
            .build()
            .unwrap();

        assert_eq!(def.name, "linear");
        assert_eq!(def.nodes.len(), 3);
        assert_eq!(def.connections.len(), 2);
    }

    #[test]
    fn build_diamond_workflow() {
        let a = NodeId::v4();
        let b = NodeId::v4();
        let c = NodeId::v4();
        let d = NodeId::v4();

        let def = WorkflowBuilder::new("diamond")
            .add_node(node(a))
            .add_node(node(b))
            .add_node(node(c))
            .add_node(node(d))
            .connect(a, b)
            .connect(a, c)
            .connect(b, d)
            .connect(c, d)
            .build()
            .unwrap();

        assert_eq!(def.nodes.len(), 4);
        assert_eq!(def.connections.len(), 4);
    }

    #[test]
    fn build_empty_name_fails() {
        let a = NodeId::v4();
        let err = WorkflowBuilder::new("")
            .add_node(node(a))
            .build()
            .unwrap_err();
        assert!(matches!(err, WorkflowError::EmptyName));
    }

    #[test]
    fn build_no_nodes_fails() {
        let err = WorkflowBuilder::new("empty").build().unwrap_err();
        assert!(matches!(err, WorkflowError::NoNodes));
    }

    #[test]
    fn build_duplicate_node_ids_fails() {
        let a = NodeId::v4();
        let err = WorkflowBuilder::new("dup")
            .add_node(node(a))
            .add_node(node(a))
            .build()
            .unwrap_err();
        assert!(matches!(err, WorkflowError::DuplicateNodeId(_)));
    }

    #[test]
    fn build_self_loop_fails() {
        let a = NodeId::v4();
        let err = WorkflowBuilder::new("loop")
            .add_node(node(a))
            .connect(a, a)
            .build()
            .unwrap_err();
        assert!(matches!(err, WorkflowError::SelfLoop(_)));
    }

    #[test]
    fn build_cycle_detected() {
        let a = NodeId::v4();
        let b = NodeId::v4();
        let err = WorkflowBuilder::new("cycle")
            .add_node(node(a))
            .add_node(node(b))
            .connect(a, b)
            .connect(b, a)
            .build()
            .unwrap_err();
        assert!(matches!(err, WorkflowError::CycleDetected));
    }

    #[test]
    fn build_with_variables_tags_config() {
        let a = NodeId::v4();
        let def = WorkflowBuilder::new("configured")
            .description("A test workflow")
            .version(Version::new(1, 0, 0))
            .add_node(node(a))
            .variable("env", serde_json::json!("production"))
            .tag("test")
            .tag("v1")
            .timeout(Duration::from_secs(60))
            .max_parallel(4)
            .build()
            .unwrap();

        assert_eq!(def.description.as_deref(), Some("A test workflow"));
        assert_eq!(def.version, Version::new(1, 0, 0));
        assert_eq!(
            def.variables.get("env"),
            Some(&serde_json::json!("production"))
        );
        assert_eq!(def.tags, vec!["test", "v1"]);
        assert_eq!(def.config.timeout, Some(Duration::from_secs(60)));
        assert_eq!(def.config.max_parallel_nodes, 4);
    }
}
