//! Comprehensive workflow validation that collects all errors.

use std::collections::HashSet;

use nebula_schema::is_assignable;

use crate::{
    definition::{CURRENT_SCHEMA_VERSION, RetryConfig, WorkflowDefinition},
    error::WorkflowError,
    graph::DependencyGraph,
    node::ParamValue,
    resolver::NodeSchemaResolver,
};

/// Validate a single `RetryConfig` against the engine's invariants.
///
/// Returns a list of human-readable rejection reasons; an empty list means the
/// config is acceptable. Layered retry rules:
///
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

    // 7. Check trigger bindings for duplicate ids within the workflow.
    // Transport-specific config (cron expression format, webhook path) is the
    // responsibility of the trigger action at load time, not this crate.
    let mut seen_trigger_ids = HashSet::new();
    for binding in &definition.trigger_bindings {
        if !seen_trigger_ids.insert(binding.id.clone()) {
            errors.push(WorkflowError::InvalidTrigger {
                reason: format!("duplicate trigger binding id: {}", binding.id),
            });
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
    // here we only iterate the actual nodes. the engine consumes
    // both surfaces (`NodeDefinition.retry_policy` overriding
    // `WorkflowConfig.retry_policy`) — rejecting bad configs at this
    // shift-left point prevents them from reaching the runtime scheduler
    //.
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

/// Validate a workflow definition and run the TypeDAG per-edge schema check.
///
/// Runs every structural check that [`validate_workflow`] performs, then for
/// each [`Connection`](crate::Connection) whose **both** endpoints can be
/// resolved by `resolver`, calls
/// `nebula_schema::is_assignable(producer.output, consumer.input)`.
///
/// An edge is **skipped** (fail-open, ADR-0100 T3.2) when:
/// - either endpoint's `action_key` is missing from the catalog
///   (`resolver.io_schemas` returns `None`), or
/// - either endpoint's node does not exist in the definition (already
///   reported by the structural pass as [`WorkflowError::UnknownNode`]).
///
/// The fail-open posture means that a workflow with no registered actions
/// (e.g. when `action_registry` is `None`) behaves identically to the
/// structural-only validator — no new hard errors, no new 422s.
///
/// # Arguments
///
/// - `definition` — the workflow to validate.
/// - `resolver` — a `dyn NodeSchemaResolver` supplied by the caller; the
///   workflow crate never imports `ActionRegistry` directly.
///
/// # Returns
///
/// All [`WorkflowError`]s collected (structural + schema), in encounter order:
/// structural errors come first (from [`validate_workflow`]), followed by any
/// [`WorkflowError::PortSchemaIncompatible`] variants in connection order.
#[must_use]
pub fn validate_workflow_with_resolver(
    definition: &WorkflowDefinition,
    resolver: &dyn NodeSchemaResolver,
) -> Vec<WorkflowError> {
    let mut errors = validate_workflow(definition);

    // Build a node-id → &NodeDefinition lookup once: O(n) build, O(1) per
    // edge lookup, avoiding O(n²) per-edge linear scan over `definition.nodes`.
    let node_by_id: std::collections::HashMap<&nebula_core::NodeKey, &crate::node::NodeDefinition> =
        definition.nodes.iter().map(|n| (&n.id, n)).collect();

    for conn in &definition.connections {
        // Skip edges whose nodes don't exist — already reported by the
        // structural pass. Reporting a schema error on a structurally broken
        // edge would be confusing and redundant.
        let Some(producer_node) = node_by_id.get(&conn.from_node) else {
            continue;
        };
        let Some(consumer_node) = node_by_id.get(&conn.to_node) else {
            continue;
        };

        // Resolve both endpoints. Either `None` → fail-open (T3.2).
        let Some(producer_schemas) = resolver.io_schemas(
            &producer_node.action_key,
            producer_node.interface_version.as_ref(),
        ) else {
            continue;
        };
        let Some(consumer_schemas) = resolver.io_schemas(
            &consumer_node.action_key,
            consumer_node.interface_version.as_ref(),
        ) else {
            continue;
        };

        // Run the directional assignability check: producer output ⊆ consumer input.
        if let Err(incompat) = is_assignable(
            producer_schemas.output.fields(),
            consumer_schemas.input.fields(),
        ) {
            errors.push(WorkflowError::PortSchemaIncompatible(Box::new(
                crate::error::PortSchemaIncompatDetails {
                    from_node: conn.from_node.clone(),
                    to_node: conn.to_node.clone(),
                    from_port: conn.from_port.clone(),
                    to_port: conn.to_port.clone(),
                    reason: incompat.to_string(),
                },
            )));
        }
    }

    errors
}

/// A [`WorkflowDefinition`] proven to pass [`validate_workflow`] with zero
/// errors — the **shift-left dispatch witness** (canon §10 / §12.2, ROADMAP
/// M3.6).
///
/// The inner definition is private and there is no `&mut` / `DerefMut`
/// accessor, so the only way to obtain a `ValidatedWorkflow` is
/// [`ValidatedWorkflow::validate`], which runs the full activation-time
/// validator. Dispatch seams that require a `&ValidatedWorkflow` therefore
/// cannot be reached with an unvalidated (or subsequently-mutated) definition:
/// "must validate before dispatch" becomes a compile-time obligation rather
/// than a convention every new handler has to remember.
///
/// The *call* to validation is still owned by the consuming layer
/// (`nebula-api` dispatch handlers); this crate owns only the witness and the
/// validator (see `crates/workflow/CLAUDE.md`).
#[derive(Debug, Clone)]
pub struct ValidatedWorkflow(WorkflowDefinition);

impl ValidatedWorkflow {
    /// Validate `definition` structurally and, on success, wrap it as a
    /// dispatch witness.
    ///
    /// This method runs only **structural** checks (DAG, node references,
    /// schema version, retry config, …). It does not run the TypeDAG
    /// per-edge schema check. To include schema-compatibility errors, use
    /// [`Self::validate_with_resolver`] instead (T3.1 — sibling, non-breaking).
    ///
    /// # Errors
    ///
    /// Returns every [`WorkflowError`] that [`validate_workflow`] collects when
    /// the definition is structurally invalid. On success the definition is
    /// moved into the witness untouched.
    pub fn validate(definition: WorkflowDefinition) -> Result<Self, Vec<WorkflowError>> {
        let errors = validate_workflow(&definition);
        if errors.is_empty() {
            Ok(Self(definition))
        } else {
            Err(errors)
        }
    }

    /// Validate `definition` with both structural checks and the TypeDAG
    /// per-edge schema check, then wrap it as a dispatch witness on success.
    ///
    /// Runs [`validate_workflow_with_resolver`], which collects all structural
    /// errors (identical to [`Self::validate`]) plus per-edge
    /// [`WorkflowError::PortSchemaIncompatible`] errors when both endpoint
    /// schemas can be resolved from `resolver`.
    ///
    /// An edge whose producer or consumer returns `None` from `resolver` is
    /// silently skipped (fail-open — ADR-0100 T3.2). Passing a resolver that
    /// always returns `None` (e.g. when `action_registry` is absent) produces
    /// the same result as [`Self::validate`].
    ///
    /// # Errors
    ///
    /// Returns every [`WorkflowError`] collected (structural + schema).
    pub fn validate_with_resolver(
        definition: WorkflowDefinition,
        resolver: &dyn NodeSchemaResolver,
    ) -> Result<Self, Vec<WorkflowError>> {
        let errors = validate_workflow_with_resolver(&definition, resolver);
        if errors.is_empty() {
            Ok(Self(definition))
        } else {
            Err(errors)
        }
    }

    /// Borrow the validated definition.
    #[must_use]
    pub fn definition(&self) -> &WorkflowDefinition {
        &self.0
    }

    /// Consume the witness, returning the validated definition by value.
    #[must_use]
    pub fn into_inner(self) -> WorkflowDefinition {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use nebula_core::{ActionKey, NodeKey, WorkflowId, node_key};
    use nebula_schema::{Field, FieldKey, Schema, ValidSchema};

    use super::*;
    use crate::{
        Version,
        connection::Connection,
        definition::{CURRENT_SCHEMA_VERSION, RetryConfig, WorkflowConfig, WorkflowDefinition},
        node::{NodeDefinition, ParamValue},
        resolver::{NodeIoSchemas, NodeSchemaResolver},
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
    fn trigger_validation_rejects_duplicate_binding_id() {
        use crate::definition::TriggerBinding;
        let a = node_key!("a");
        let mut def = make_definition("dup-trigger", vec![node(a)], vec![]);
        def.trigger_bindings = vec![
            TriggerBinding::new(node_key!("t1"), "scheduler", "cron.schedule").unwrap(),
            TriggerBinding::new(node_key!("t1"), "scheduler", "cron.schedule").unwrap(), // duplicate id
        ];
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::InvalidTrigger { .. })),
            "expected InvalidTrigger for duplicate binding id, got: {errors:?}"
        );
    }

    #[test]
    fn trigger_validation_accepts_distinct_bindings() {
        use crate::definition::TriggerBinding;
        let a = node_key!("a");
        let mut def = make_definition("distinct-triggers", vec![node(a)], vec![]);
        def.trigger_bindings = vec![
            TriggerBinding::new(node_key!("every-hour"), "scheduler", "cron.schedule")
                .unwrap()
                .with_config(serde_json::json!({"expression": "0 * * * *"})),
            TriggerBinding::new(node_key!("on-push"), "http", "http.webhook")
                .unwrap()
                .with_config(serde_json::json!({"path": "/hooks/push"})),
        ];
        let errors = validate_workflow(&def);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::InvalidTrigger { .. })),
            "expected no InvalidTrigger for distinct binding ids, got: {errors:?}"
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

    // ── retry_policy validation tests (ROADMAP §) ────────────

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
    fn validated_workflow_accepts_valid_definition() {
        let a = node_key!("a");
        let b = node_key!("b");
        let def = make_definition(
            "ok",
            vec![node(a.clone()), node(b.clone())],
            vec![Connection::new(a, b)],
        );
        let validated = ValidatedWorkflow::validate(def.clone()).expect("valid workflow");
        // Witness round-trips the exact definition it was built from.
        assert_eq!(validated.definition().name, "ok");
        assert_eq!(validated.into_inner().nodes.len(), def.nodes.len());
    }

    #[test]
    fn validated_workflow_rejects_invalid_definition() {
        // Empty-nodes workflow fails `validate_workflow` (NoNodes); the witness
        // surfaces every collected error rather than silently constructing.
        let def = make_definition("empty", vec![], vec![]);
        let errors = ValidatedWorkflow::validate(def).expect_err("empty workflow must fail");
        assert!(
            errors.iter().any(|e| matches!(e, WorkflowError::NoNodes)),
            "expected NoNodes; got: {errors:?}"
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

    // ── TypeDAG T3 tests (ADR-0100) ─────────────────────────────────────────

    /// Build a `ValidSchema` with one named field.
    fn single_field_schema(key: &str, required: bool) -> ValidSchema {
        let fk = FieldKey::new(key).unwrap();
        let field = if required {
            Field::string(fk).required()
        } else {
            Field::string(fk)
        };
        Schema::builder().add(field).build().unwrap()
    }

    /// A resolver that maps `ActionKey` string → `NodeIoSchemas`.
    struct MapResolver(HashMap<String, NodeIoSchemas>);

    impl NodeSchemaResolver for MapResolver {
        fn io_schemas(
            &self,
            action_key: &ActionKey,
            _interface_version: Option<&semver::Version>,
        ) -> Option<NodeIoSchemas> {
            self.0.get(action_key.as_str()).cloned()
        }
    }

    /// Build a two-node workflow (a → b) for schema tests.
    fn two_node_def() -> (WorkflowDefinition, NodeKey, NodeKey) {
        let a = node_key!("a");
        let b = node_key!("b");
        let def = make_definition(
            "typedag-test",
            vec![
                NodeDefinition::new(a.clone(), "Producer", "core", "producer.action").unwrap(),
                NodeDefinition::new(b.clone(), "Consumer", "core", "consumer.action").unwrap(),
            ],
            vec![Connection::new(a.clone(), b.clone())],
        );
        (def, a, b)
    }

    /// Compatible schemas → no `PortSchemaIncompatible` errors.
    ///
    /// Guards against false positives: a check that wrongly rejects compatible
    /// schemas turns this RED. The primary red-on-revert guard for the check
    /// existing at all is the sibling
    /// `incompatible_schemas_produce_port_schema_incompatible_error`, which goes
    /// RED if the per-edge check is removed.
    #[test]
    fn compatible_schemas_produce_no_port_schema_errors() {
        let (def, _, _) = two_node_def();

        // Producer emits `{a: required}`, consumer expects `{a: required}`.
        let producer_output = single_field_schema("a", true);
        let consumer_input = single_field_schema("a", true);

        let mut schemas = HashMap::new();
        schemas.insert(
            "producer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty(),
                output: producer_output,
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: consumer_input,
                output: ValidSchema::empty(),
            },
        );

        let resolver = MapResolver(schemas);
        let errors = validate_workflow_with_resolver(&def, &resolver);

        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::PortSchemaIncompatible(_))),
            "compatible schemas must produce no PortSchemaIncompatible; got: {errors:?}"
        );
    }

    /// Incompatible schemas → exactly one `PortSchemaIncompatible` with
    /// the right node keys.
    ///
    /// This test goes RED if the per-edge check is removed from
    /// `validate_workflow_with_resolver`.
    #[test]
    fn incompatible_schemas_produce_port_schema_incompatible_error() {
        let (def, a, b) = two_node_def();

        // Producer emits `{a: required}`, consumer requires `{b}` (absent).
        let producer_output = single_field_schema("a", true);
        let consumer_input = single_field_schema("b", true); // mismatch: `a` not present

        let mut schemas = HashMap::new();
        schemas.insert(
            "producer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty(),
                output: producer_output,
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: consumer_input,
                output: ValidSchema::empty(),
            },
        );

        let resolver = MapResolver(schemas);
        let errors = validate_workflow_with_resolver(&def, &resolver);

        let schema_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, WorkflowError::PortSchemaIncompatible(_)))
            .collect();

        assert_eq!(
            schema_errors.len(),
            1,
            "expected exactly one PortSchemaIncompatible; got: {errors:?}"
        );

        // Assert the variant carries the correct node keys.
        let Some(WorkflowError::PortSchemaIncompatible(details)) = schema_errors.first().copied()
        else {
            panic!("expected PortSchemaIncompatible; got: {errors:?}");
        };
        assert_eq!(details.from_node, a, "from_node must be the producer node");
        assert_eq!(details.to_node, b, "to_node must be the consumer node");
    }

    /// Fail-open: when the resolver returns `None` for one endpoint,
    /// no `PortSchemaIncompatible` is produced, even though the graph
    /// would error if both sides resolved incompatibly.
    #[test]
    fn fail_open_when_one_endpoint_unresolvable() {
        let (def, _, _) = two_node_def();

        // Consumer resolves with a schema that would be incompatible with
        // any non-empty producer, but the producer is unresolvable (`None`).
        let consumer_input = single_field_schema("required_field", true);

        let mut schemas = HashMap::new();
        // Only consumer resolves; producer is absent from the map.
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: consumer_input,
                output: ValidSchema::empty(),
            },
        );

        let resolver = MapResolver(schemas);
        let errors = validate_workflow_with_resolver(&def, &resolver);

        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::PortSchemaIncompatible(_))),
            "unresolvable endpoint must cause the edge to be skipped (fail-open); got: {errors:?}"
        );
    }

    /// The non-resolver `validate_workflow` still returns only structural
    /// errors — schema errors are never injected by the structural pass.
    #[test]
    fn structural_validate_unchanged_no_schema_errors() {
        let (def, _, _) = two_node_def();
        let errors = validate_workflow(&def);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::PortSchemaIncompatible(_))),
            "validate_workflow must never produce PortSchemaIncompatible; got: {errors:?}"
        );
    }

    /// `ValidatedWorkflow::validate_with_resolver` on a compatible graph
    /// returns `Ok`.
    #[test]
    fn validated_workflow_with_resolver_accepts_compatible_graph() {
        let (def, _, _) = two_node_def();

        // Producer emits `{a}`, consumer expects `{a}`.
        let output = single_field_schema("a", true);
        let input = single_field_schema("a", true);

        let mut schemas = HashMap::new();
        schemas.insert(
            "producer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty(),
                output,
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input,
                output: ValidSchema::empty(),
            },
        );

        let resolver = MapResolver(schemas);
        assert!(
            ValidatedWorkflow::validate_with_resolver(def, &resolver).is_ok(),
            "compatible schemas must produce a valid witness"
        );
    }

    /// `ValidatedWorkflow::validate_with_resolver` on an incompatible graph
    /// returns `Err` containing `PortSchemaIncompatible`.
    #[test]
    fn validated_workflow_with_resolver_rejects_incompatible_graph() {
        let (def, _, _) = two_node_def();

        // Producer emits `{a}`, consumer requires `{b}` — incompatible.
        let output = single_field_schema("a", true);
        let input = single_field_schema("b", true);

        let mut schemas = HashMap::new();
        schemas.insert(
            "producer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty(),
                output,
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input,
                output: ValidSchema::empty(),
            },
        );

        let resolver = MapResolver(schemas);
        let errors = ValidatedWorkflow::validate_with_resolver(def, &resolver)
            .expect_err("incompatible graph must be rejected");

        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::PortSchemaIncompatible(_))),
            "expected PortSchemaIncompatible in errors; got: {errors:?}"
        );
    }
}
