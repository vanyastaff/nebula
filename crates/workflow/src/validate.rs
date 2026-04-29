//! Comprehensive workflow validation that collects all errors.

use std::collections::HashSet;

use crate::{
    definition::{CURRENT_SCHEMA_VERSION, RetryConfig, TriggerDefinition, WorkflowDefinition},
    error::WorkflowError,
    graph::DependencyGraph,
    node::ParamValue,
};

/// Validate a single `RetryConfig` against the engine's invariants.
///
/// Returns a list of human-readable rejection reasons; an empty list means the
/// config is acceptable. Per ADR-0042 (layered retry):
/// - `max_attempts == 0` is rejected — the field should be `None` to disable retries; zero retries
///   means the field is dead and indicates a modelling bug.
/// - `initial_delay_ms == 0` with `max_attempts > 1` is rejected — burst retry with no backoff is
///   an abuse vector against the action's downstream service.
/// - `max_delay_ms < initial_delay_ms` is logically incoherent (cap below start).
/// - `backoff_multiplier <= 0.0` or non-finite (NaN/Inf) breaks the engine's delay formula
///   `min(initial * mult^attempt, max)`.
fn validate_retry_config(config: &RetryConfig) -> Vec<String> {
    let mut reasons = Vec::new();
    if config.max_attempts == 0 {
        reasons
            .push("max_attempts must be >= 1 (use retry_policy = None to disable retries)".into());
    }
    if config.initial_delay_ms == 0 && config.max_attempts > 1 {
        reasons.push(
            "initial_delay_ms == 0 with max_attempts > 1 is a burst retry without backoff; \
             use a non-zero delay or set max_attempts == 1"
                .into(),
        );
    }
    if config.max_delay_ms < config.initial_delay_ms {
        reasons.push("max_delay_ms must be >= initial_delay_ms".into());
    }
    if !config.backoff_multiplier.is_finite() || config.backoff_multiplier <= 0.0 {
        reasons.push("backoff_multiplier must be a finite positive number".into());
    }
    reasons
}

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

    // 1b. Check workflow-default retry_policy. Independent of nodes / graph,
    // so it runs BEFORE the empty-nodes early return — otherwise a malformed
    // `WorkflowConfig.retry_policy` would be silently dropped when the
    // workflow also has zero nodes (CodeRabbit on PR #627).
    if let Some(ref retry) = definition.config.retry_policy {
        for reason in validate_retry_config(retry) {
            errors.push(WorkflowError::InvalidRetryConfig { node: None, reason });
        }
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

    // 4b. Detect duplicate connections (identical source, target, and ports).
    // Duplicate connections are always redundant and confuse edge-resolution bookkeeping.
    // Two edges that wire the same node pair but on different `from_port` values are
    // distinct — e.g. main vs error routing from the same upstream node.
    //
    // Serialising to JSON gives us a canonical `Hash`-free comparison key without hand-rolling
    // a discriminator over every `Connection` field.
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

    // 9. Check per-node retry_policy validity. The workflow-default retry
    // policy was validated as step 1b (before the empty-nodes early return);
    // here we only iterate the actual nodes. Per ADR-0042 the engine consumes
    // both surfaces (`NodeDefinition.retry_policy` overriding
    // `WorkflowConfig.retry_policy`) — rejecting bad configs at this
    // shift-left point prevents them from reaching the runtime scheduler
    // (canon §10).
    for node in &definition.nodes {
        if let Some(ref retry) = node.retry_policy {
            for reason in validate_retry_config(retry) {
                errors.push(WorkflowError::InvalidRetryConfig {
                    node: Some(node.id.clone()),
                    reason,
                });
            }
        }
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
        definition::{RetryConfig, WorkflowConfig, WorkflowDefinition},
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
        let a = node_key!("a");
        let b = node_key!("b");
        // Two edges from A to B but on different source ports — these are
        // distinct (not duplicates) and must not trigger a validation error.
        let def = make_definition(
            "multi-edge",
            vec![node(a.clone()), node(b.clone())],
            vec![
                Connection::new(a.clone(), b.clone()),
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

    // ── retry_policy validation tests (ROADMAP §M2.1 / ADR-0042) ────────────

    /// Construct a workflow with a single node carrying the given retry config.
    fn def_with_node_retry(retry: RetryConfig) -> WorkflowDefinition {
        let a = node_key!("a");
        let mut n = node(a);
        n.retry_policy = Some(retry);
        make_definition("retry-test", vec![n], vec![])
    }

    /// Construct a workflow with a workflow-default retry config and one node.
    fn def_with_workflow_retry(retry: RetryConfig) -> WorkflowDefinition {
        let a = node_key!("a");
        let mut def = make_definition("retry-test", vec![node(a)], vec![]);
        def.config.retry_policy = Some(retry);
        def
    }

    #[test]
    fn valid_fixed_retry_passes_validation() {
        let def = def_with_node_retry(RetryConfig::fixed(3, 100));
        let errors = validate_workflow(&def);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::InvalidRetryConfig { .. })),
            "valid fixed retry config should not produce InvalidRetryConfig; got: {errors:?}"
        );
    }

    #[test]
    fn valid_exponential_retry_passes_validation() {
        let def = def_with_node_retry(RetryConfig {
            max_attempts: 5,
            initial_delay_ms: 100,
            max_delay_ms: 5_000,
            backoff_multiplier: 2.0,
        });
        let errors = validate_workflow(&def);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::InvalidRetryConfig { .. })),
            "valid exponential retry config should not produce InvalidRetryConfig; got: {errors:?}"
        );
    }

    #[test]
    fn rejects_zero_max_attempts() {
        let def = def_with_node_retry(RetryConfig {
            max_attempts: 0,
            initial_delay_ms: 100,
            max_delay_ms: 1_000,
            backoff_multiplier: 1.0,
        });
        let errors = validate_workflow(&def);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                WorkflowError::InvalidRetryConfig { reason, .. } if reason.contains("max_attempts must be >= 1")
            )),
            "expected max_attempts == 0 to be rejected; got: {errors:?}"
        );
    }

    #[test]
    fn rejects_zero_initial_delay_with_multiple_attempts() {
        let def = def_with_node_retry(RetryConfig {
            max_attempts: 3,
            initial_delay_ms: 0,
            max_delay_ms: 0,
            backoff_multiplier: 2.0,
        });
        let errors = validate_workflow(&def);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                WorkflowError::InvalidRetryConfig { reason, .. } if reason.contains("burst retry")
            )),
            "expected zero-initial-delay + multi-attempt to be rejected; got: {errors:?}"
        );
    }

    #[test]
    fn rejects_max_delay_below_initial_delay() {
        let def = def_with_node_retry(RetryConfig {
            max_attempts: 3,
            initial_delay_ms: 1_000,
            max_delay_ms: 500, // < initial_delay_ms
            backoff_multiplier: 1.0,
        });
        let errors = validate_workflow(&def);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                WorkflowError::InvalidRetryConfig { reason, .. } if reason.contains("max_delay_ms must be >= initial_delay_ms")
            )),
            "expected max_delay_ms < initial_delay_ms to be rejected; got: {errors:?}"
        );
    }

    #[test]
    fn rejects_zero_backoff_multiplier() {
        let def = def_with_node_retry(RetryConfig {
            max_attempts: 3,
            initial_delay_ms: 100,
            max_delay_ms: 1_000,
            backoff_multiplier: 0.0,
        });
        let errors = validate_workflow(&def);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                WorkflowError::InvalidRetryConfig { reason, .. } if reason.contains("backoff_multiplier must be a finite positive number")
            )),
            "expected backoff_multiplier == 0 to be rejected; got: {errors:?}"
        );
    }

    #[test]
    fn rejects_negative_backoff_multiplier() {
        let def = def_with_node_retry(RetryConfig {
            max_attempts: 3,
            initial_delay_ms: 100,
            max_delay_ms: 1_000,
            backoff_multiplier: -2.0,
        });
        let errors = validate_workflow(&def);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                WorkflowError::InvalidRetryConfig { reason, .. } if reason.contains("backoff_multiplier must be a finite positive number")
            )),
            "expected negative backoff_multiplier to be rejected; got: {errors:?}"
        );
    }

    #[test]
    fn rejects_non_finite_backoff_multiplier() {
        let def = def_with_node_retry(RetryConfig {
            max_attempts: 3,
            initial_delay_ms: 100,
            max_delay_ms: 1_000,
            backoff_multiplier: f64::NAN,
        });
        let errors = validate_workflow(&def);
        assert!(
            errors.iter().any(|e| matches!(
                e,
                WorkflowError::InvalidRetryConfig { reason, .. } if reason.contains("backoff_multiplier must be a finite positive number")
            )),
            "expected NaN backoff_multiplier to be rejected; got: {errors:?}"
        );
    }

    #[test]
    fn node_level_retry_error_carries_node_id() {
        let a = node_key!("a");
        let mut n = node(a.clone());
        n.retry_policy = Some(RetryConfig {
            max_attempts: 0,
            initial_delay_ms: 100,
            max_delay_ms: 100,
            backoff_multiplier: 1.0,
        });
        let def = make_definition("node-retry", vec![n], vec![]);
        let errors = validate_workflow(&def);
        let node_err = errors.iter().find_map(|e| match e {
            WorkflowError::InvalidRetryConfig {
                node: Some(key), ..
            } => Some(key),
            _ => None,
        });
        assert_eq!(
            node_err,
            Some(&a),
            "node-level invalid retry config must carry the node id; got: {errors:?}"
        );
    }

    #[test]
    fn workflow_default_retry_error_carries_no_node() {
        let def = def_with_workflow_retry(RetryConfig {
            max_attempts: 0,
            initial_delay_ms: 100,
            max_delay_ms: 100,
            backoff_multiplier: 1.0,
        });
        let errors = validate_workflow(&def);
        let workflow_err = errors
            .iter()
            .any(|e| matches!(e, WorkflowError::InvalidRetryConfig { node: None, .. }));
        assert!(
            workflow_err,
            "workflow-default invalid retry config must have node = None; got: {errors:?}"
        );
    }

    #[test]
    fn workflow_default_retry_validated_even_when_nodes_empty() {
        // Regression test for CodeRabbit PR #627 finding: workflow-default
        // retry policy must be validated independently of the empty-nodes
        // early return. Otherwise a malformed `WorkflowConfig.retry_policy`
        // would be silently dropped when the workflow also has zero nodes.
        let mut def = make_definition("empty-with-bad-retry", vec![], vec![]);
        def.config.retry_policy = Some(RetryConfig {
            max_attempts: 0, // invalid
            initial_delay_ms: 100,
            max_delay_ms: 100,
            backoff_multiplier: 1.0,
        });
        let errors = validate_workflow(&def);
        assert!(
            errors.iter().any(|e| matches!(e, WorkflowError::NoNodes)),
            "expected NoNodes error; got: {errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::InvalidRetryConfig { node: None, .. })),
            "workflow-default retry error must surface even when nodes is empty; got: {errors:?}"
        );
    }
}
