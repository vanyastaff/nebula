//! Workflow building utilities.
//!
//! This module provides helpers for constructing workflows programmatically.
//!
//! # Examples
//!
//! ```rust,no_run
//! use nebula_sdk::workflow::WorkflowBuilder;
//! use nebula_sdk::prelude::*;
//!
//! let workflow = WorkflowBuilder::new("my_workflow")
//!     .with_description("Processes data")
//!     .add_node("start", "echo")
//!     .add_node("process", "http_request")
//!     .connect("start", "process")
//!     .build();
//! ```

use nebula_core::{ActionKey, NodeId, Version, WorkflowId};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, ParamValue, WorkflowConfig, WorkflowDefinition,
    connection::{Connection, EdgeCondition},
    node::NodeDefinition,
};
use std::collections::{HashMap, HashSet};

/// Builder for constructing workflows.
///
/// # Examples
///
/// ```ignore
/// use nebula_sdk::workflow::WorkflowBuilder;
///
/// let workflow = WorkflowBuilder::new("data_pipeline")
///     .with_description("ETL pipeline")
///     .add_node("extract", "550e8400-e29b-41d4-a716-446655440000")
///     .add_node("transform", "550e8400-e29b-41d4-a716-446655440001")
///     .add_node("load", "550e8400-e29b-41d4-a716-446655440002")
///     .connect("extract", "transform")
///     .connect("transform", "load")
///     .build()
///     .expect("valid workflow");
/// ```
pub struct WorkflowBuilder {
    id: WorkflowId,
    name: String,
    description: Option<String>,
    nodes: Vec<NodeConfig>,
    connections: Vec<(String, String)>,
    variables: HashMap<String, serde_json::Value>,
    version: Version,
}

#[derive(Debug, Clone)]
struct NodeConfig {
    id: String,
    name: String,
    action_key: String,
    parameters: HashMap<String, ParamValue>,
}

impl WorkflowBuilder {
    /// Create a new workflow builder.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique workflow identifier
    pub fn new(id: impl Into<String>) -> Self {
        let id_str = id.into();
        Self {
            name: id_str.clone(),
            id: WorkflowId::new(),
            description: None,
            nodes: Vec::new(),
            connections: Vec::new(),
            variables: HashMap::new(),
            version: Version::new(1, 0, 0),
        }
    }

    /// Set the workflow display name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the workflow description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the workflow version.
    pub fn with_version(mut self, major: u32, minor: u32, patch: u32) -> Self {
        self.version = Version::new(major, minor, patch);
        self
    }

    /// Add a node to the workflow.
    ///
    /// # Arguments
    ///
    /// * `id` - Node identifier within the workflow
    /// * `action_key` - Plugin/action key (e.g. `"echo"`, `"http_request"`)
    pub fn add_node(mut self, id: impl Into<String>, action_key: impl Into<String>) -> Self {
        let id_str = id.into();
        self.nodes.push(NodeConfig {
            id: id_str.clone(),
            name: id_str,
            action_key: action_key.into(),
            parameters: HashMap::new(),
        });
        self
    }

    /// Add a node with initial parameters.
    pub fn add_node_with_params(
        mut self,
        id: impl Into<String>,
        action_key: impl Into<String>,
        parameters: HashMap<String, ParamValue>,
    ) -> Self {
        let id_str = id.into();
        self.nodes.push(NodeConfig {
            id: id_str.clone(),
            name: id_str,
            action_key: action_key.into(),
            parameters,
        });
        self
    }

    /// Connect two nodes.
    ///
    /// # Arguments
    ///
    /// * `from` - Source node ID
    /// * `to` - Target node ID
    pub fn connect(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.connections.push((from.into(), to.into()));
        self
    }

    /// Add a workflow variable.
    pub fn with_variable(
        mut self,
        name: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        self.variables.insert(name.into(), value.into());
        self
    }

    /// Build the workflow definition.
    ///
    /// # Errors
    ///
    /// Returns an error if the workflow is invalid (e.g., references to non-existent nodes).
    pub fn build(self) -> crate::Result<WorkflowDefinition> {
        use chrono::Utc;

        let mut seen = HashSet::new();
        for node in &self.nodes {
            if !seen.insert(node.id.clone()) {
                return Err(crate::Error::workflow(format!(
                    "Duplicate node id: {}",
                    node.id
                )));
            }
        }

        // Stable mapping between user-facing node ids and typed ids.
        let node_id_by_name: HashMap<String, NodeId> = self
            .nodes
            .iter()
            .map(|node| (node.id.clone(), NodeId::new()))
            .collect();

        // Validate all edge references.
        for (from, to) in &self.connections {
            if !node_id_by_name.contains_key(from) {
                return Err(crate::Error::workflow(format!(
                    "Connection references unknown source node: {}",
                    from
                )));
            }
            if !node_id_by_name.contains_key(to) {
                return Err(crate::Error::workflow(format!(
                    "Connection references unknown target node: {}",
                    to
                )));
            }
        }

        // Build nodes.
        let nodes: Vec<NodeDefinition> = self
            .nodes
            .into_iter()
            .map(|config| -> crate::Result<NodeDefinition> {
                let node_id = node_id_by_name.get(&config.id).copied().ok_or_else(|| {
                    crate::Error::workflow(format!("Node id not found in mapping: {}", config.id))
                })?;

                let action_key: ActionKey = config.action_key.parse().map_err(|e| {
                    crate::Error::action(format!(
                        "Invalid action_key for node `{}`: `{}` ({})",
                        config.id, config.action_key, e
                    ))
                })?;

                Ok(NodeDefinition {
                    id: node_id,
                    name: config.name,
                    action_key,
                    interface_version: None,
                    parameters: config.parameters,
                    retry_policy: None,
                    timeout: None,
                    description: None,
                    enabled: true,
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;

        // Build connections.
        let connections: Vec<Connection> = self
            .connections
            .into_iter()
            .map(|(from, to)| -> crate::Result<Connection> {
                let from_node = node_id_by_name.get(&from).copied().ok_or_else(|| {
                    crate::Error::workflow(format!("Unknown source node in connection: {}", from))
                })?;
                let to_node = node_id_by_name.get(&to).copied().ok_or_else(|| {
                    crate::Error::workflow(format!("Unknown target node in connection: {}", to))
                })?;

                Ok(Connection {
                    from_node,
                    to_node,
                    condition: EdgeCondition::Always,
                    branch_key: None,
                    from_port: None,
                    to_port: None,
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;

        let now = Utc::now();

        Ok(WorkflowDefinition {
            id: self.id,
            name: self.name,
            description: self.description,
            version: self.version,
            nodes,
            connections,
            variables: self.variables,
            config: WorkflowConfig::default(),
            trigger: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: CURRENT_SCHEMA_VERSION,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_builder_basic() {
        let action_a = uuid::Uuid::new_v4().to_string();
        let action_b = uuid::Uuid::new_v4().to_string();
        let workflow = WorkflowBuilder::new("test_workflow")
            .with_description("Test workflow")
            .add_node("start", action_a)
            .add_node("end", action_b)
            .connect("start", "end")
            .build();

        assert!(workflow.is_ok());
        let wf = workflow.unwrap();
        assert_eq!(wf.nodes.len(), 2);
        assert_eq!(wf.connections.len(), 1);
    }

    #[test]
    fn test_workflow_builder_invalid_connection() {
        let action_a = uuid::Uuid::new_v4().to_string();
        let result = WorkflowBuilder::new("test")
            .add_node("start", action_a)
            .connect("start", "nonexistent")
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_workflow_builder_invalid_action_id() {
        let result = WorkflowBuilder::new("test").add_node("start", "").build();

        assert!(result.is_err());
    }
}
