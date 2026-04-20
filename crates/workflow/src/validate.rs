//! Comprehensive workflow validation that collects all errors.

use std::collections::HashSet;

use crate::{
    definition::{CURRENT_SCHEMA_VERSION, TriggerDefinition, WorkflowDefinition},
    error::WorkflowError,
    graph::DependencyGraph,
    node::ParamValue,
};

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

    // 3. Check duplicate node keys
    let mut seen_ids = HashSet::new();
    for node in &definition.nodes {
        if !seen_ids.insert(node.id.clone()) {
            errors.push(WorkflowError::DuplicateNodeKey(node.id.clone()));
        }
    }

    // 4. Check connections reference valid nodes and detect self-loops
    for conn in &definition.connections {
        if !seen_ids.contains(&conn.from_node) {
            errors.push(WorkflowError::UnknownNode(conn.from_node.clone()));
        }
        if !seen_ids.contains(&conn.to_node) {
            errors.push(WorkflowError::UnknownNode(conn.to_node.clone()));
        }
        if conn.is_self_loop() {
            errors.push(WorkflowError::SelfLoop(conn.from_node.clone()));
        }
    }

    // 4b. Detect duplicate connections (identical source, target, ports, and condition).
    // Duplicate connections are always redundant and confuse edge-resolution bookkeeping.
    //
    // `Connection` cannot derive `Hash` (because `serde_json::Value` doesn't implement it),
    // so we serialize each connection to a canonical JSON string and use a HashSet<String>
    // for O(n) average-case detection.
    let mut seen_connections: HashSet<String> = HashSet::new();
    for conn in &definition.connections {
        let key = serde_json::to_string(conn).unwrap_or_default();
        if !seen_connections.insert(key) {
            errors.push(WorkflowError::DuplicateConnection {
                from: conn.from_node.clone(),
                to: conn.to_node.clone(),
            });
        }
    }

    // 5. Check parameter references
    for node in &definition.nodes {
        for param in node.parameters.values() {
            if let ParamValue::Reference { node_key, .. } = param
                && !seen_ids.contains(node_key)
            {
                errors.push(WorkflowError::InvalidParameterReference {
                    node_key: node.id.clone(),
                    source_node_key: node_key.clone(),
                });
            }
        }
    }

    // 6. Check schema version
    if !definition.is_schema_supported() {
        errors.push(WorkflowError::UnsupportedSchema {
            version: definition.schema_version,
            max: CURRENT_SCHEMA_VERSION,
        });
    }

    // 7. Check trigger configuration
    if let Some(trigger) = &definition.trigger {
        match trigger {
            TriggerDefinition::Cron { expression } if expression.is_empty() => {
                errors.push(WorkflowError::InvalidTrigger {
                    reason: "cron expression must not be empty".into(),
                });
            },
            TriggerDefinition::Webhook { path, .. } if !path.starts_with('/') => {
                errors.push(WorkflowError::InvalidTrigger {
                    reason: format!("webhook path must start with '/', got: {path:?}"),
                });
            },
            _ => {},
        }
    }

    // 8. Check graph structure
    match DependencyGraph::from_definition(definition) {
        Ok(graph) => {
            if graph.has_cycle() {
                errors.push(WorkflowError::CycleDetected);
            }
            if graph.entry_nodes().is_empty() {
                errors.push(WorkflowError::NoEntryNodes);
            }
        },
        Err(e) => errors.push(e),
    }

    errors
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
        definition::{WorkflowConfig, WorkflowDefinition},
        node::{NodeDefinition, ParamValue},
    };

    fn make_definition(
        name: &str,
        nodes: Vec<NodeDefinition>,
        connections: Vec<Connection>,
    ) -> WorkflowDefinition {
        let now = Utc::now();
        WorkflowDefinition {
            id: WorkflowId::new(),
            name: name.into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes,
            connections,
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            trigger: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: 1,
        }
    }

    fn node(id: NodeKey) -> NodeDefinition {
        NodeDefinition::new(id, "n", "n").unwrap()
    }

    #[test]
    fn valid_workflow_returns_empty() {
        let a = node_key!("a");
        let b = node_key!("b");
        let def = make_definition(
            "ok",
            vec![node(a.clone()), node(b.clone())],
            vec![Connection::new(a, b)],
        );
        let errors = validate_workflow(&def);
        assert!(errors.is_empty(), "expected no errors, got: {errors:?}");
    }

    #[test]
    fn detects_empty_name() {
        let a = node_key!("a");
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
        let a = node_key!("a");
        let unknown = node_key!("unknown");
        let def = make_definition(
            "bad",
            vec![node(a.clone())],
            vec![Connection::new(a, unknown)],
        );
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::UnknownNode(_)))
        );
    }

    #[test]
    fn detects_self_loop() {
        let a = node_key!("a");
        let def = make_definition(
            "loop",
            vec![node(a.clone())],
            vec![Connection::new(a.clone(), a)],
        );
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::SelfLoop(_)))
        );
    }

    #[test]
    fn detects_invalid_parameter_reference() {
        let a = node_key!("a");
        let ghost = node_key!("ghost");
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
        let a = node_key!("a");
        let unknown = node_key!("unknown");
        let def = make_definition(
            "",
            vec![node(a.clone())],
            vec![
                Connection::new(a.clone(), a.clone()),
                Connection::new(a, unknown),
            ],
        );
        let errors = validate_workflow(&def);
        // Should have at least: EmptyName, SelfLoop, UnknownNode
        assert!(errors.len() >= 3, "expected >= 3 errors, got: {errors:?}");
    }

    #[test]
    fn detects_cycle() {
        let a = node_key!("a");
        let b = node_key!("b");
        let def = make_definition(
            "cycle",
            vec![node(a.clone()), node(b.clone())],
            vec![Connection::new(a.clone(), b.clone()), Connection::new(b, a)],
        );
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::CycleDetected))
        );
    }

    #[test]
    fn trigger_validation_rejects_invalid_webhook_path() {
        let a = node_key!("a");
        let mut def = make_definition("webhook-test", vec![node(a)], vec![]);
        def.trigger = Some(TriggerDefinition::Webhook {
            method: "POST".into(),
            path: "no-leading-slash".into(),
        });
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::InvalidTrigger { .. })),
            "expected InvalidTrigger, got: {errors:?}"
        );
    }

    #[test]
    fn trigger_validation_rejects_empty_cron_expression() {
        let a = node_key!("a");
        let mut def = make_definition("cron-test", vec![node(a)], vec![]);
        def.trigger = Some(TriggerDefinition::Cron {
            expression: String::new(),
        });
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::InvalidTrigger { .. })),
            "expected InvalidTrigger, got: {errors:?}"
        );
    }

    #[test]
    fn trigger_validation_accepts_valid_webhook() {
        let a = node_key!("a");
        let mut def = make_definition("webhook-ok", vec![node(a)], vec![]);
        def.trigger = Some(TriggerDefinition::Webhook {
            method: "POST".into(),
            path: "/hooks/incoming".into(),
        });
        let errors = validate_workflow(&def);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::InvalidTrigger { .. })),
            "expected no InvalidTrigger, got: {errors:?}"
        );
    }

    #[test]
    fn trigger_validation_accepts_valid_cron() {
        let a = node_key!("a");
        let mut def = make_definition("cron-ok", vec![node(a)], vec![]);
        def.trigger = Some(TriggerDefinition::Cron {
            expression: "0 */5 * * *".into(),
        });
        let errors = validate_workflow(&def);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::InvalidTrigger { .. })),
            "expected no InvalidTrigger, got: {errors:?}"
        );
    }

    #[test]
    fn detects_unsupported_schema_version() {
        let a = node_key!("a");
        let mut def = make_definition("schema-test", vec![node(a)], vec![]);
        def.schema_version = 99;
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::UnsupportedSchema { .. })),
            "expected UnsupportedSchema, got: {errors:?}"
        );
    }

    #[test]
    fn detects_duplicate_connection() {
        let a = node_key!("a");
        let b = node_key!("b");
        // Two identical connections: same from/to, same condition, same ports.
        let conn = Connection::new(a.clone(), b.clone());
        let def = make_definition("dup-conn", vec![node(a), node(b)], vec![conn.clone(), conn]);
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::DuplicateConnection { .. })),
            "expected DuplicateConnection error, got: {errors:?}"
        );
    }

    #[test]
    fn distinct_multi_edges_are_not_duplicate() {
        use crate::connection::EdgeCondition;
        let a = node_key!("a");
        let b = node_key!("b");
        // Two edges from A to B but with different conditions — these are
        // distinct (not duplicates) and must not trigger a validation error.
        let def = make_definition(
            "multi-edge",
            vec![node(a.clone()), node(b.clone())],
            vec![
                Connection::new(a.clone(), b.clone()).with_condition(EdgeCondition::Always),
                Connection::new(a, b).with_from_port("alt"),
            ],
        );
        let errors = validate_workflow(&def);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::DuplicateConnection { .. })),
            "distinct multi-edges must not be flagged as duplicates; got: {errors:?}"
        );
    }
}
