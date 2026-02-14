//! Comprehensive workflow validation that collects all errors.

use std::collections::HashSet;

use crate::definition::WorkflowDefinition;
use crate::error::WorkflowError;
use crate::graph::DependencyGraph;
use crate::node::ParamValue;

/// Validate a workflow definition comprehensively.
///
/// Unlike [`WorkflowBuilder::build`](crate::WorkflowBuilder::build), which stops at the
/// first error, this function collects every issue it can find so they can all be
/// reported at once.
#[must_use]
pub fn validate_workflow(definition: &WorkflowDefinition) -> Vec<WorkflowError> {
    let mut errors = Vec::new();

    // 1. Check name
    if definition.name.is_empty() {
        errors.push(WorkflowError::EmptyName);
    }

    // 2. Check node count
    if definition.nodes.is_empty() {
        errors.push(WorkflowError::NoNodes);
        return errors; // Cannot check further without nodes
    }

    // 3. Check duplicate node IDs
    let mut seen_ids = HashSet::new();
    for node in &definition.nodes {
        if !seen_ids.insert(node.id) {
            errors.push(WorkflowError::DuplicateNodeId(node.id));
        }
    }

    // 4. Check connections reference valid nodes and detect self-loops
    for conn in &definition.connections {
        if !seen_ids.contains(&conn.from_node) {
            errors.push(WorkflowError::UnknownNode(conn.from_node));
        }
        if !seen_ids.contains(&conn.to_node) {
            errors.push(WorkflowError::UnknownNode(conn.to_node));
        }
        if conn.is_self_loop() {
            errors.push(WorkflowError::SelfLoop(conn.from_node));
        }
    }

    // 5. Check parameter references
    for node in &definition.nodes {
        for param in node.parameters.values() {
            if let ParamValue::Reference { node_id, .. } = param
                && !seen_ids.contains(node_id)
            {
                errors.push(WorkflowError::InvalidParameterReference {
                    node_id: node.id,
                    source_node_id: *node_id,
                });
            }
        }
    }

    // 6. Check graph structure
    match DependencyGraph::from_definition(definition) {
        Ok(graph) => {
            if graph.has_cycle() {
                errors.push(WorkflowError::CycleDetected);
            }
            if graph.entry_nodes().is_empty() {
                errors.push(WorkflowError::NoEntryNodes);
            }
        }
        Err(e) => errors.push(e),
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Connection;
    use crate::definition::{WorkflowConfig, WorkflowDefinition};
    use crate::node::{NodeDefinition, ParamValue};
    use chrono::Utc;
    use nebula_core::{ActionId, NodeId, Version, WorkflowId};
    use std::collections::HashMap;

    fn make_definition(
        name: &str,
        nodes: Vec<NodeDefinition>,
        connections: Vec<Connection>,
    ) -> WorkflowDefinition {
        let now = Utc::now();
        WorkflowDefinition {
            id: WorkflowId::v4(),
            name: name.into(),
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

    #[test]
    fn valid_workflow_returns_empty() {
        let a = NodeId::v4();
        let b = NodeId::v4();
        let def = make_definition("ok", vec![node(a), node(b)], vec![Connection::new(a, b)]);
        let errors = validate_workflow(&def);
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn detects_empty_name() {
        let a = NodeId::v4();
        let def = make_definition("", vec![node(a)], vec![]);
        let errors = validate_workflow(&def);
        assert!(errors.iter().any(|e| matches!(e, WorkflowError::EmptyName)));
    }

    #[test]
    fn detects_no_nodes() {
        let def = make_definition("empty", vec![], vec![]);
        let errors = validate_workflow(&def);
        assert!(errors.iter().any(|e| matches!(e, WorkflowError::NoNodes)));
    }

    #[test]
    fn detects_unknown_node_in_connection() {
        let a = NodeId::v4();
        let unknown = NodeId::v4();
        let def = make_definition("bad", vec![node(a)], vec![Connection::new(a, unknown)]);
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::UnknownNode(_)))
        );
    }

    #[test]
    fn detects_self_loop() {
        let a = NodeId::v4();
        let def = make_definition("loop", vec![node(a)], vec![Connection::new(a, a)]);
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::SelfLoop(_)))
        );
    }

    #[test]
    fn detects_invalid_parameter_reference() {
        let a = NodeId::v4();
        let ghost = NodeId::v4();
        let mut n = node(a);
        n.parameters
            .insert("input".into(), ParamValue::reference(ghost, "$.data"));
        let def = make_definition("ref", vec![n], vec![]);
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::InvalidParameterReference { .. }))
        );
    }

    #[test]
    fn collects_multiple_errors() {
        // empty name + self-loop + unknown node
        let a = NodeId::v4();
        let unknown = NodeId::v4();
        let def = make_definition(
            "",
            vec![node(a)],
            vec![Connection::new(a, a), Connection::new(a, unknown)],
        );
        let errors = validate_workflow(&def);
        // Should have at least: EmptyName, SelfLoop, UnknownNode
        assert!(errors.len() >= 3, "expected >= 3 errors, got: {errors:?}");
    }

    #[test]
    fn detects_cycle() {
        let a = NodeId::v4();
        let b = NodeId::v4();
        let def = make_definition(
            "cycle",
            vec![node(a), node(b)],
            vec![Connection::new(a, b), Connection::new(b, a)],
        );
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::CycleDetected))
        );
    }
}
