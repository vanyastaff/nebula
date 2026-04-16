//! Fluent builder for constructing and validating workflow definitions.

use std::{collections::HashMap, time::Duration};

use chrono::Utc;
use nebula_core::{NodeKey, WorkflowId};

use crate::{
    Version,
    connection::{Connection, EdgeCondition},
    definition::{CURRENT_SCHEMA_VERSION, UiMetadata, WorkflowConfig, WorkflowDefinition},
    error::WorkflowError,
    graph::DependencyGraph,
    node::NodeDefinition,
};

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
    owner_id: Option<String>,
    ui_metadata: Option<UiMetadata>,
}

impl WorkflowBuilder {
    /// Start building a workflow with the given name.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: WorkflowId::new(),
            name: name.into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: Vec::new(),
            connections: Vec::new(),
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            tags: Vec::new(),
            owner_id: None,
            ui_metadata: None,
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
    pub fn connect(mut self, from: NodeKey, to: NodeKey) -> Self {
        self.connections.push(Connection::new(from, to));
        self
    }

    /// Add a conditional connection between two nodes.
    #[must_use]
    pub fn connect_with_condition(
        mut self,
        from: NodeKey,
        to: NodeKey,
        condition: EdgeCondition,
    ) -> Self {
        self.connections
            .push(Connection::new(from, to).with_condition(condition));
        self
    }

    /// Add a connection with explicit source and target ports.
    #[must_use]
    pub fn connect_port(
        mut self,
        from: NodeKey,
        from_port: impl Into<String>,
        to: NodeKey,
        to_port: impl Into<String>,
    ) -> Self {
        self.connections
            .push(Connection::new(from, to).with_ports(from_port, to_port));
        self
    }

    /// Add a pre-built connection directly.
    #[must_use]
    pub fn add_connection(mut self, connection: Connection) -> Self {
        self.connections.push(connection);
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

    /// Set the owner ID for multi-tenant workflows.
    ///
    /// # Errors
    ///
    /// Returns [`WorkflowError::EmptyName`] if `owner_id` is empty or blank.
    pub fn owner(mut self, owner_id: impl Into<String>) -> Result<Self, WorkflowError> {
        let id = owner_id.into();
        if id.trim().is_empty() {
            return Err(WorkflowError::InvalidOwnerId);
        }
        self.owner_id = Some(id);
        Ok(self)
    }

    /// Set UI metadata (node positions, viewport, annotations).
    #[must_use = "builder methods must be chained or built"]
    pub fn ui_metadata(mut self, metadata: UiMetadata) -> Self {
        self.ui_metadata = Some(metadata);
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
            if !seen.insert(node.id.clone()) {
                return Err(WorkflowError::DuplicateNodeId(node.id.clone()));
            }
        }

        // Check self-loops
        for conn in &self.connections {
            if conn.is_self_loop() {
                return Err(WorkflowError::SelfLoop(conn.from_node.clone()));
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
            trigger: None,
            tags: self.tags,
            created_at: now,
            updated_at: now,
            owner_id: self.owner_id,
            ui_metadata: self.ui_metadata,
            schema_version: CURRENT_SCHEMA_VERSION,
        };

        // Validate graph structure
        let graph = DependencyGraph::from_definition(&definition)?;
        graph.validate()?;

        Ok(definition)
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::{NodeKey, node_key};

    use super::*;

    fn node(id: NodeKey) -> NodeDefinition {
        NodeDefinition::new(id, "n", "n").unwrap()
    }

    #[test]
    fn build_linear_workflow() {
        let a = node_key!("a");
        let b = node_key!("b");
        let c = node_key!("c");

        let def = WorkflowBuilder::new("linear")
            .add_node(node(a.clone()))
            .add_node(node(b.clone()))
            .add_node(node(c.clone()))
            .connect(a, b.clone())
            .connect(b, c)
            .build()
            .unwrap();

        assert_eq!(def.name, "linear");
        assert_eq!(def.nodes.len(), 3);
        assert_eq!(def.connections.len(), 2);
    }

    #[test]
    fn build_diamond_workflow() {
        let a = node_key!("a");
        let b = node_key!("b");
        let c = node_key!("c");
        let d = node_key!("d");

        let def = WorkflowBuilder::new("diamond")
            .add_node(node(a.clone()))
            .add_node(node(b.clone()))
            .add_node(node(c.clone()))
            .add_node(node(d.clone()))
            .connect(a.clone(), b.clone())
            .connect(a, c.clone())
            .connect(b, d.clone())
            .connect(c, d)
            .build()
            .unwrap();

        assert_eq!(def.nodes.len(), 4);
        assert_eq!(def.connections.len(), 4);
    }

    #[test]
    fn build_empty_name_fails() {
        let a = node_key!("a");
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
        let a = node_key!("a");
        let err = WorkflowBuilder::new("dup")
            .add_node(node(a.clone()))
            .add_node(node(a))
            .build()
            .unwrap_err();
        assert!(matches!(err, WorkflowError::DuplicateNodeId(_)));
    }

    #[test]
    fn build_self_loop_fails() {
        let a = node_key!("a");
        let err = WorkflowBuilder::new("loop")
            .add_node(node(a.clone()))
            .connect(a.clone(), a)
            .build()
            .unwrap_err();
        assert!(matches!(err, WorkflowError::SelfLoop(_)));
    }

    #[test]
    fn build_cycle_detected() {
        let a = node_key!("a");
        let b = node_key!("b");
        let err = WorkflowBuilder::new("cycle")
            .add_node(node(a.clone()))
            .add_node(node(b.clone()))
            .connect(a.clone(), b.clone())
            .connect(b, a)
            .build()
            .unwrap_err();
        assert!(matches!(err, WorkflowError::CycleDetected));
    }

    #[test]
    fn build_with_variables_tags_config() {
        let a = node_key!("a");
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

    #[test]
    fn owner_rejects_empty_string() {
        let result = WorkflowBuilder::new("test").owner("");
        assert!(matches!(result, Err(WorkflowError::InvalidOwnerId)));
    }

    #[test]
    fn owner_rejects_blank_string() {
        let result = WorkflowBuilder::new("test").owner("   ");
        assert!(matches!(result, Err(WorkflowError::InvalidOwnerId)));
    }

    #[test]
    fn owner_accepts_valid_string() {
        let a = node_key!("a");
        let def = WorkflowBuilder::new("owned")
            .owner("user_123")
            .unwrap()
            .add_node(node(a))
            .build()
            .unwrap();
        assert_eq!(def.owner_id.as_deref(), Some("user_123"));
    }
}
