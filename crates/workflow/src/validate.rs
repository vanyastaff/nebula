//! Comprehensive workflow validation that collects all errors.

use std::collections::HashSet;

use nebula_schema::{
    Assignability, FieldKey, PathWalk, explain_assignable, explain_field_assignable,
    is_opaque_field_node,
};

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

    // 5. Check parameter references.
    //
    // A `Reference` must (a) point at a node that exists and (b) have a coincident
    // connection edge from that node to the consumer. The dependency graph is built
    // from connections only (`graph.rs` never reads `parameters`), so a reference
    // with no connection leaves the data dependency invisible to the scheduler — the
    // producer may be ordered after the consumer, which then reads stale or absent
    // output. Making the connection mandatory keeps every data dependency visible on
    // the graph (and type-checkable). Build the (from, to) endpoint set once for O(1)
    // coincidence checks (O(E) build, O(params) loop — not O(params·E)).
    let connected_pairs: HashSet<_> = definition
        .connections
        .iter()
        .map(|conn| (&conn.from_node, &conn.to_node))
        .collect();
    for node in &definition.nodes {
        for param in node.parameters.values() {
            let ParamValue::Reference { node_key, .. } = param else {
                continue;
            };
            if !seen_ids.contains(node_key) {
                errors.push(WorkflowError::InvalidParameterReference {
                    node_key: node.id.clone(),
                    source_node_key: node_key.clone(),
                });
            } else if !connected_pairs.contains(&(node_key, &node.id)) {
                errors.push(WorkflowError::ReferenceWithoutConnection {
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

/// Policy for how the TypeDAG per-edge schema check treats an **undecidable**
/// assignability verdict ([`nebula_schema::Assignability::Unknown`] — a
/// loader-backed `Dynamic` field, an opaque `Any` producer, `Mode` sum-type
/// variance, or a float→int narrowing).
///
/// A provable incompatibility ([`No`](nebula_schema::Assignability::No)) is
/// always reported regardless of mode; this only governs the `Unknown` middle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SchemaCheckMode {
    /// Undecidable edges **pass** (warn-and-pass). The default — preserves
    /// untyped `serde_json::Value` / `Dynamic` workflows. Behaves exactly like
    /// the binary [`nebula_schema::is_assignable_schema`].
    #[default]
    Gradual,
    /// Undecidable edges are **blocked** with
    /// [`WorkflowError::PortSchemaUndecidable`] carrying the reasons. Use when a
    /// workflow must be provably well-typed before activation.
    Strict,
}

/// Validate a workflow definition and run the TypeDAG per-edge schema check in
/// [`SchemaCheckMode::Gradual`] (the back-compatible default — undecidable edges
/// pass). See [`validate_workflow_with_resolver_mode`] to choose the mode.
#[must_use]
pub fn validate_workflow_with_resolver(
    definition: &WorkflowDefinition,
    resolver: &dyn NodeSchemaResolver,
) -> Vec<WorkflowError> {
    validate_workflow_with_resolver_mode(definition, resolver, SchemaCheckMode::Gradual)
}

/// Validate a workflow definition and run the TypeDAG per-edge schema check.
///
/// Runs every structural check that [`validate_workflow`] performs, then for
/// each [`Connection`](crate::Connection) whose **both** endpoints can be
/// resolved by `resolver`, computes `nebula_schema::explain_assignable` over the
/// polarity-typed `producer.output` ([`OutputSchema`](nebula_schema::OutputSchema))
/// and `consumer.input` ([`InputSchema`](nebula_schema::InputSchema)) — the
/// newtypes make transposing the two a compile error — and reports per `mode`:
/// a [`No`](nebula_schema::Assignability::No) verdict is
/// always a [`WorkflowError::PortSchemaIncompatible`]; an
/// [`Unknown`](nebula_schema::Assignability::Unknown) verdict is a
/// [`WorkflowError::PortSchemaUndecidable`] only in [`SchemaCheckMode::Strict`].
///
/// An edge is **skipped** (fail-open, ADR-0100 T3.2) when **any** of the
/// following hold:
/// - the edge is not a main-flow edge: `from_port` resolves to something other
///   than `"out"` (e.g. `"error"`, `"true"`, a dynamic / support port key), or
///   `to_port` is a named port (support / supply input). Only default
///   main-flow edges — `from_port: None` (effective `"out"`) **and**
///   `to_port: None` — carry the typed `A::Output` / `A::Input` payload that
///   `output_schema` / `base.schema` describe. Named ports carry different
///   payloads and must not be validated against the success-output schema.
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
/// - `mode` — how undecidable edges are treated (see [`SchemaCheckMode`]).
///
/// # Returns
///
/// All [`WorkflowError`]s collected (structural + schema), in encounter order:
/// structural errors come first (from [`validate_workflow`]), followed by any
/// [`WorkflowError::PortSchemaIncompatible`] / [`WorkflowError::PortSchemaUndecidable`]
/// variants in connection order.
#[must_use]
pub fn validate_workflow_with_resolver_mode(
    definition: &WorkflowDefinition,
    resolver: &dyn NodeSchemaResolver,
    mode: SchemaCheckMode,
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

        // Port-scope guard: only type-check main-flow edges.
        //
        // `output_schema` / `base.schema` describe the typed A::Output / A::Input
        // on the SUCCESS path — the default main-flow edge. Named `from_port` values
        // (e.g. `"error"` for recovery routing, `"true"`/`"false"` for control
        // branches, dynamic port keys) carry a *different* payload shape and must
        // not be validated against the success output schema, or legitimate
        // error/recovery edges would be falsely rejected at `/activate`/`/validate`.
        //
        // `effective_from_port()` normalises `None → "out"` (the engine's
        // canonical main-flow sentinel). `to_port: None` is the engine's default
        // flow input; a named `to_port` indicates a support or supply input whose
        // schema is not captured by `base.schema`.
        if conn.effective_from_port().as_str() != "out" || conn.to_port.is_some() {
            continue;
        }

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

        // Run the directional, kind-aware assignability check: producer output
        // ⊆ consumer input. `explain_assignable` honors the `SchemaKind`
        // Top/Bottom split (an `Output = ()` empty record does not satisfy a
        // consumer that hard-requires a field; an untyped `Any` producer is
        // `Unknown`, not a hard error).
        match explain_assignable(&producer_schemas.output, &consumer_schemas.input) {
            // Provably incompatible: always a hard error, carrying every finding.
            Assignability::No(incompatibilities) => {
                errors.push(WorkflowError::PortSchemaIncompatible(Box::new(
                    crate::error::PortSchemaIncompatDetails {
                        from_node: conn.from_node.clone(),
                        to_node: conn.to_node.clone(),
                        from_port: conn.from_port.clone(),
                        to_port: conn.to_port.clone(),
                        incompatibilities,
                    },
                )));
            },
            // Undecidable: blocked only in Strict mode; Gradual warns-and-passes.
            Assignability::Unknown(reasons) if mode == SchemaCheckMode::Strict => {
                errors.push(WorkflowError::PortSchemaUndecidable(Box::new(
                    crate::error::PortSchemaUndecidableDetails {
                        from_node: conn.from_node.clone(),
                        to_node: conn.to_node.clone(),
                        from_port: conn.from_port.clone(),
                        to_port: conn.to_port.clone(),
                        reasons,
                    },
                )));
            },
            // `Yes`, an `Unknown` in Gradual mode, or any future verdict variant
            // (the enum is `#[non_exhaustive]`): pass the edge.
            _ => {},
        }
    }

    check_reference_edges(definition, resolver, mode, &node_by_id, &mut errors);

    errors
}

/// Type-check each node's per-field `ParamValue::Reference` edges against the
/// producer's output schema (ADR-0100 TypeDAG, W0 U5 — **correctness only**,
/// see the crate's W0 U5 plan; this does *not* close any secret-exfiltration
/// surface — `Expression`/`Template` parameters already read every prior
/// node's raw output through the identical runtime path).
///
/// Complements the main-flow port check above: that loop only type-checks the
/// *default* `"out"` connection edge, whereas a `Reference` parameter can pull
/// from **any** node's output through an arbitrary authored dotted path,
/// entirely outside the main-flow port shape.
///
/// Fail-open (no error pushed) when:
/// - the referenced producer node is missing from `node_by_id`, or either
///   endpoint's schema does not resolve (`resolver.io_schemas` returns
///   `None`) — mirrors the main-flow edge check's fail-open contract;
/// - `ValidSchema::walk_authored_path` returns [`PathWalk::Opaque`] — an
///   opaque node, a missing `Object` key, or an untyped `List` item anywhere
///   along the walk (never provably wrong, see the schema crate's opacity
///   classification);
/// - the consumer's expected field is undeterminable — `param_key` is not a
///   valid [`FieldKey`], no such field is declared on the consumer's input
///   schema, or the declared field is itself opaque. Only the *type* check is
///   skipped in this case; a hard error from the walk above still stands.
///
/// Hard errors (both [`SchemaCheckMode`]s) when the walk returns
/// [`PathWalk::Unresolved`] — a non-numeric `List` index, or a segment past a
/// scalar leaf — via [`WorkflowError::ReferencePathUnresolved`]. When the walk
/// resolves and the leaf is provably not assignable to the consumer's expected
/// field ([`Assignability::No`]), pushes
/// [`WorkflowError::ReferenceTypeIncompatible`] (both modes);
/// [`Assignability::Unknown`] pushes [`WorkflowError::ReferenceTypeUndecidable`]
/// only under [`SchemaCheckMode::Strict`].
fn check_reference_edges(
    definition: &WorkflowDefinition,
    resolver: &dyn NodeSchemaResolver,
    mode: SchemaCheckMode,
    node_by_id: &std::collections::HashMap<&nebula_core::NodeKey, &crate::node::NodeDefinition>,
    errors: &mut Vec<WorkflowError>,
) {
    for consumer_node in &definition.nodes {
        for (param_key, param_value) in &consumer_node.parameters {
            let ParamValue::Reference {
                node_key: producer_key,
                output_path,
            } = param_value
            else {
                continue;
            };

            // Fail-open: unknown producer (already reported structurally), or
            // either endpoint's schema does not resolve from the catalog.
            let Some(producer_node) = node_by_id.get(producer_key) else {
                continue;
            };
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

            // Opacity-gated path walk: `Opaque` fails open; `Unresolved` is a
            // provable mistake on an otherwise fully-closed path (hard error in
            // both modes); `Resolved` hands back the leaf field to type-check.
            let producer_leaf = match producer_schemas
                .output
                .as_schema()
                .walk_authored_path(output_path)
            {
                PathWalk::Opaque => continue,
                PathWalk::Unresolved(reason) => {
                    errors.push(WorkflowError::ReferencePathUnresolved(Box::new(
                        crate::error::ReferencePathUnresolvedDetails {
                            consumer_node: consumer_node.id.clone(),
                            param_key: param_key.clone(),
                            producer_node: producer_key.clone(),
                            output_path: output_path.clone(),
                            reason: reason.to_string(),
                        },
                    )));
                    continue;
                },
                PathWalk::Resolved(leaf) => leaf.clone(),
                // `PathWalk` is `#[non_exhaustive]`: a future verdict variant
                // defaults to fail-open, the same posture as `Opaque`, never a
                // silent hard error.
                _ => continue,
            };

            // Resolve the consumer's expected field for this parameter. The
            // parameter key equals the consumer `InputSchema` field key by
            // construction (schema keys mirror serde wire keys, including
            // `#[serde(rename)]`ed fields — see the derive macro's own test
            // suite). Undeterminable (invalid key, no such field, or the field
            // is itself opaque per the same classification the walk above
            // uses) → fail-open, skip only the type check; the walk's
            // `Resolved` verdict above already stands on its own.
            let Ok(consumer_key) = FieldKey::new(param_key.as_str()) else {
                continue;
            };
            let Some(consumer_field) = consumer_schemas.input.as_schema().find(&consumer_key)
            else {
                continue;
            };
            if is_opaque_field_node(consumer_field) {
                continue;
            }

            match explain_field_assignable(&producer_leaf, consumer_field) {
                Assignability::No(incompatibilities) => {
                    errors.push(WorkflowError::ReferenceTypeIncompatible(Box::new(
                        crate::error::ReferenceTypeIncompatDetails {
                            consumer_node: consumer_node.id.clone(),
                            param_key: param_key.clone(),
                            producer_node: producer_key.clone(),
                            output_path: output_path.clone(),
                            incompatibilities,
                        },
                    )));
                },
                Assignability::Unknown(reasons) if mode == SchemaCheckMode::Strict => {
                    errors.push(WorkflowError::ReferenceTypeUndecidable(Box::new(
                        crate::error::ReferenceTypeUndecidableDetails {
                            consumer_node: consumer_node.id.clone(),
                            param_key: param_key.clone(),
                            producer_node: producer_key.clone(),
                            output_path: output_path.clone(),
                            reasons,
                        },
                    )));
                },
                // `Yes`, an `Unknown` in Gradual mode, or any future verdict
                // variant (the enum is `#[non_exhaustive]`): pass the reference.
                _ => {},
            }
        }
    }
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
    /// per-edge schema check in [`SchemaCheckMode::Gradual`], then wrap it as a
    /// dispatch witness on success.
    ///
    /// A `Gradual`-hardcoded convenience over
    /// [`Self::validate_with_resolver_mode`]: collects all structural errors
    /// (identical to [`Self::validate`]) plus per-edge
    /// [`WorkflowError::PortSchemaIncompatible`] errors when both endpoint
    /// schemas resolve. Undecidable edges pass; use
    /// [`Self::validate_with_resolver_mode`] with [`SchemaCheckMode::Strict`] to
    /// reject them ([`WorkflowError::PortSchemaUndecidable`]).
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
        Self::validate_with_resolver_mode(definition, resolver, SchemaCheckMode::Gradual)
    }

    /// Like [`Self::validate_with_resolver`] but with an explicit
    /// [`SchemaCheckMode`]. [`SchemaCheckMode::Strict`] additionally rejects
    /// undecidable edges ([`WorkflowError::PortSchemaUndecidable`]), so the
    /// resulting witness is provably well-typed, not merely not-refuted.
    ///
    /// # Errors
    ///
    /// Returns every [`WorkflowError`] collected (structural + schema).
    pub fn validate_with_resolver_mode(
        definition: WorkflowDefinition,
        resolver: &dyn NodeSchemaResolver,
        mode: SchemaCheckMode,
    ) -> Result<Self, Vec<WorkflowError>> {
        let errors = validate_workflow_with_resolver_mode(&definition, resolver, mode);
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
    use nebula_core::{ActionKey, NodeKey, WorkflowId, node_key, port_key};
    use nebula_schema::{Field, FieldKey, Schema, ValidSchema, schema_of};

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
    fn reference_without_connection_is_rejected() {
        // `a` pulls `b`'s output via a parameter Reference, but there is no
        // connection edge b -> a, so the data dependency is invisible to the
        // scheduler (graph.rs builds edges from connections only).
        let a = node_key!("a");
        let b = node_key!("b");
        let mut consumer = node(a);
        consumer
            .parameters
            .insert("input".into(), ParamValue::reference(b.clone(), "$.data"));
        let def = make_definition("ref-no-conn", vec![consumer, node(b)], vec![]);
        let errors = validate_workflow(&def);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::ReferenceWithoutConnection { .. })),
            "a Reference with no coincident connection must be rejected; got: {errors:?}"
        );
    }

    #[test]
    fn reference_with_coincident_connection_passes() {
        // Same Reference, now backed by a real b -> a connection edge: the
        // dependency is visible to the scheduler, so it must not be flagged.
        let a = node_key!("a");
        let b = node_key!("b");
        let mut consumer = node(a.clone());
        consumer
            .parameters
            .insert("input".into(), ParamValue::reference(b.clone(), "$.data"));
        let def = make_definition(
            "ref-with-conn",
            vec![consumer, node(b.clone())],
            vec![Connection::new(b, a)],
        );
        let errors = validate_workflow(&def);
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::ReferenceWithoutConnection { .. })),
            "a Reference WITH a coincident connection must not be flagged; got: {errors:?}"
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
                Connection::new(a, b).with_from_port(port_key!("alt")),
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
                input: ValidSchema::empty().into(),
                output: producer_output.into(),
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: consumer_input.into(),
                output: ValidSchema::empty().into(),
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
                input: ValidSchema::empty().into(),
                output: producer_output.into(),
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: consumer_input.into(),
                output: ValidSchema::empty().into(),
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

    /// Top/Bottom split enforced on a real edge: a producer whose `Output = ()`
    /// (an empty *record*, not the gradual `Any`) does NOT satisfy a consumer
    /// that hard-requires a field. This is exactly the scenario the old
    /// kind-blind slice check missed — it treated the empty output as `Any`.
    #[test]
    fn empty_record_output_does_not_satisfy_required_input() {
        let (def, a, b) = two_node_def();

        // Producer emits nothing (`()` → empty record); consumer requires `needed`.
        let producer_output = ValidSchema::empty();
        let consumer_input = single_field_schema("needed", true);

        let mut schemas = HashMap::new();
        schemas.insert(
            "producer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty().into(),
                output: producer_output.into(),
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: consumer_input.into(),
                output: ValidSchema::empty().into(),
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
            "an empty-record output feeding a required input must be rejected; got: {errors:?}"
        );
        let Some(WorkflowError::PortSchemaIncompatible(details)) = schema_errors.first().copied()
        else {
            panic!("expected PortSchemaIncompatible; got: {errors:?}");
        };
        assert_eq!(details.from_node, a);
        assert_eq!(details.to_node, b);
    }

    /// Contrast with the empty-record case: an untyped `Any` output
    /// (`serde_json::Value` → [`ValidSchema::any`]) still satisfies a required
    /// consumer — gradual typing's escape hatch is preserved, so untyped flows
    /// are not falsely rejected.
    #[test]
    fn any_output_satisfies_required_input() {
        let (def, _, _) = two_node_def();

        let producer_output = ValidSchema::any();
        let consumer_input = single_field_schema("needed", true);

        let mut schemas = HashMap::new();
        schemas.insert(
            "producer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty().into(),
                output: producer_output.into(),
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: consumer_input.into(),
                output: ValidSchema::empty().into(),
            },
        );

        let resolver = MapResolver(schemas);
        let errors = validate_workflow_with_resolver(&def, &resolver);

        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::PortSchemaIncompatible(_))),
            "an `Any` output must still satisfy a required input (gradual escape); got: {errors:?}"
        );
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
                input: consumer_input.into(),
                output: ValidSchema::empty().into(),
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

    /// Non-main-flow edges are skipped regardless of schema compatibility.
    ///
    /// `output_schema` / `base.schema` describe the typed `A::Output` / `A::Input`
    /// on the SUCCESS path only. A named `from_port` (e.g. `"error"` for recovery
    /// routing) carries a *different* payload shape and must not be compared against
    /// the success output schema, or legitimate error/recovery edges would be
    /// falsely rejected at `/activate`/`/validate`.
    ///
    /// Non-vacuous contract: the SAME incompatible schemas on the default
    /// main-flow edge (from the existing `incompatible_schemas_produce_port_schema_incompatible_error`
    /// test) DO produce `PortSchemaIncompatible`. The guard is what makes this
    /// test pass — removing it would cause the error-port edge to be wrongly
    /// rejected (RED).
    #[test]
    fn non_main_port_edge_is_skipped() {
        let a = node_key!("a");
        let b = node_key!("b");

        // Producer output {x: required} and consumer input {y: required} are
        // structurally incompatible (missing field `y`). On the main-flow edge
        // this would produce `PortSchemaIncompatible` — but this edge routes
        // through the `"error"` port, which carries a different payload shape.
        let producer_output = single_field_schema("x", true);
        let consumer_input = single_field_schema("y", true);

        let def = make_definition(
            "non-main-port-test",
            vec![
                NodeDefinition::new(a.clone(), "Producer", "core", "producer.action").unwrap(),
                NodeDefinition::new(b.clone(), "Consumer", "core", "consumer.action").unwrap(),
            ],
            // Error-port edge: producer's "error" output → consumer's default input.
            vec![Connection::new(a, b).with_from_port(port_key!("error"))],
        );

        let mut schemas = HashMap::new();
        schemas.insert(
            "producer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty().into(),
                output: producer_output.into(),
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: consumer_input.into(),
                output: ValidSchema::empty().into(),
            },
        );

        let resolver = MapResolver(schemas);
        let errors = validate_workflow_with_resolver(&def, &resolver);

        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::PortSchemaIncompatible(_))),
            "named from_port (\"error\") must be skipped — schema check applies only to \
             main-flow edges; got: {errors:?}"
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
                input: ValidSchema::empty().into(),
                output: output.into(),
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: input.into(),
                output: ValidSchema::empty().into(),
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
                input: ValidSchema::empty().into(),
                output: output.into(),
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: input.into(),
                output: ValidSchema::empty().into(),
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

    // ── SchemaCheckMode: Gradual vs Strict on undecidable edges ───────────────

    /// Build a producer schema whose single field is `Dynamic` (loader-backed) —
    /// an output that yields an `Unknown` verdict against a typed consumer.
    fn dynamic_field_schema(key: &str) -> ValidSchema {
        Schema::builder()
            .add(Field::dynamic(FieldKey::new(key).unwrap()))
            .build()
            .unwrap()
    }

    /// Build the producer-dynamic / consumer-required edge used by the mode tests.
    fn undecidable_edge_schemas() -> HashMap<String, NodeIoSchemas> {
        let mut schemas = HashMap::new();
        schemas.insert(
            "producer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty().into(),
                output: dynamic_field_schema("data").into(),
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: single_field_schema("data", true).into(),
                output: ValidSchema::empty().into(),
            },
        );
        schemas
    }

    /// Gradual mode (the default) warns-and-passes an undecidable edge: a
    /// `Dynamic` producer output feeding a required consumer input yields no
    /// schema error — untyped/dynamic workflows keep working.
    #[test]
    fn gradual_mode_passes_undecidable_dynamic_edge() {
        let (def, _, _) = two_node_def();
        let resolver = MapResolver(undecidable_edge_schemas());

        let errors = validate_workflow_with_resolver(&def, &resolver);
        assert!(
            !errors.iter().any(|e| matches!(
                e,
                WorkflowError::PortSchemaUndecidable(_) | WorkflowError::PortSchemaIncompatible(_)
            )),
            "gradual mode must pass an undecidable (Dynamic) edge; got: {errors:?}"
        );
    }

    /// Strict mode blocks the same undecidable edge with exactly one
    /// `PortSchemaUndecidable` (and no `PortSchemaIncompatible`, since it is not
    /// a provable conflict).
    #[test]
    fn strict_mode_blocks_undecidable_dynamic_edge() {
        let (def, a, b) = two_node_def();
        let resolver = MapResolver(undecidable_edge_schemas());

        let errors = validate_workflow_with_resolver_mode(&def, &resolver, SchemaCheckMode::Strict);

        let undecidable: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, WorkflowError::PortSchemaUndecidable(_)))
            .collect();
        assert_eq!(
            undecidable.len(),
            1,
            "strict mode must block the undecidable edge; got: {errors:?}"
        );
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::PortSchemaIncompatible(_))),
            "an undecidable edge is not a hard incompatibility; got: {errors:?}"
        );
        let Some(WorkflowError::PortSchemaUndecidable(details)) = undecidable.first().copied()
        else {
            panic!("expected PortSchemaUndecidable; got: {errors:?}");
        };
        assert_eq!(details.from_node, a);
        assert_eq!(details.to_node, b);
        // The structured reason is the producer's `Dynamic` output field `data`.
        assert_eq!(
            details.reasons,
            vec![nebula_schema::UnknownReason::DynamicLoaderBacked {
                key: FieldKey::new("data").unwrap()
            }],
            "reasons carries the structured UnknownReason, not just a string"
        );
        assert!(
            details.to_string().contains("dynamic"),
            "Display renders the reason; got: {details}"
        );
    }

    /// A provably-incompatible edge with TWO missing required fields carries
    /// BOTH in `incompatibilities` (the collect-all behavior) — guards against a
    /// regression to first-error-only.
    #[test]
    fn incompatible_edge_carries_all_findings() {
        let (def, _, _) = two_node_def();
        let consumer_input = Schema::builder()
            .add(Field::string(FieldKey::new("a").unwrap()).required())
            .add(Field::string(FieldKey::new("b").unwrap()).required())
            .build()
            .unwrap();
        let mut schemas = HashMap::new();
        schemas.insert(
            "producer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty().into(),
                output: ValidSchema::empty().into(), // empty record — supplies neither `a` nor `b`
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: consumer_input.into(),
                output: ValidSchema::empty().into(),
            },
        );
        let resolver = MapResolver(schemas);

        let errors = validate_workflow_with_resolver(&def, &resolver);
        let incompat: Vec<_> = errors
            .iter()
            .filter_map(|e| match e {
                WorkflowError::PortSchemaIncompatible(d) => Some(d),
                _ => None,
            })
            .collect();
        assert_eq!(incompat.len(), 1, "one edge, one error; got: {errors:?}");
        assert_eq!(
            incompat[0].incompatibilities.len(),
            2,
            "both missing required fields are reported, not just the first"
        );
        let rendered = incompat[0].to_string();
        assert!(
            rendered.contains('a') && rendered.contains('b'),
            "Display joins all findings; got: {rendered}"
        );
    }

    /// Strict mode does NOT reclassify a provable conflict: a genuinely
    /// incompatible edge is still `PortSchemaIncompatible`, never
    /// `PortSchemaUndecidable`.
    #[test]
    fn strict_mode_still_reports_hard_incompatible_as_incompatible() {
        let (def, _, _) = two_node_def();
        let mut schemas = HashMap::new();
        schemas.insert(
            "producer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty().into(),
                output: single_field_schema("a", true).into(),
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: single_field_schema("b", true).into(), // requires `b`, absent
                output: ValidSchema::empty().into(),
            },
        );
        let resolver = MapResolver(schemas);

        let errors = validate_workflow_with_resolver_mode(&def, &resolver, SchemaCheckMode::Strict);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::PortSchemaIncompatible(_))),
            "a provable conflict stays PortSchemaIncompatible in strict mode; got: {errors:?}"
        );
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, WorkflowError::PortSchemaUndecidable(_))),
            "a provable conflict must not be reported as undecidable; got: {errors:?}"
        );
    }

    /// `ValidatedWorkflow::validate_with_resolver_mode(Strict)` rejects an
    /// undecidable graph, while the gradual witness accepts it.
    #[test]
    fn validated_workflow_strict_rejects_undecidable_gradual_accepts() {
        let (def, _, _) = two_node_def();
        let resolver = MapResolver(undecidable_edge_schemas());

        assert!(
            ValidatedWorkflow::validate_with_resolver(def.clone(), &resolver).is_ok(),
            "gradual witness accepts an undecidable edge"
        );

        let errors =
            ValidatedWorkflow::validate_with_resolver_mode(def, &resolver, SchemaCheckMode::Strict)
                .expect_err("strict witness must reject an undecidable edge");
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, WorkflowError::PortSchemaUndecidable(_))),
            "expected PortSchemaUndecidable; got: {errors:?}"
        );
    }

    // ── check_reference_edges: per-field Reference type check (W0 U5) ────────
    //
    // Tests 1/2/6 in the plan MUST use real `#[derive(Schema)]`-produced
    // structs, not hand-built `ValidSchema` fixtures — these are the exact
    // regressions the four adversarial review rounds were about (a hand-built
    // fixture can't prove what the derive macro actually emits for
    // `serde_json::Value` / an enum).

    /// Build a two-node workflow where node `b`'s `param_key` parameter is a
    /// `Reference` to node `a`'s output at `output_path`, with a coincident
    /// connection (so the structural `ReferenceWithoutConnection` check does
    /// not also fire and muddy the assertions).
    ///
    /// The connection is wired to a named, non-default `to_port` ("params")
    /// rather than the default main-flow `to_port: None` — deliberately so
    /// the separate main-flow port-schema check (`validate_workflow_with_resolver_mode`'s
    /// own loop, which only type-checks `to_port: None` edges) never fires on
    /// it. `ReferenceWithoutConnection` only requires the (from, to) node
    /// pair to be connected, port-agnostic, so this does not weaken that
    /// guard. Without this, a test asserting only `assert_no_reference_errors`
    /// could stay spuriously green off an unrelated `PortSchemaIncompatible`
    /// masking a broken reference check, or a producer/consumer pairing
    /// crafted to make the reference check pass could incidentally also trip
    /// the main-flow check — either way conflating two independent checks
    /// under one assertion.
    fn two_node_reference_def(
        param_key: &str,
        output_path: &str,
    ) -> (WorkflowDefinition, NodeKey, NodeKey) {
        let a = node_key!("a");
        let b = node_key!("b");
        let mut consumer =
            NodeDefinition::new(b.clone(), "Consumer", "core", "consumer.action").unwrap();
        consumer.parameters.insert(
            param_key.to_owned(),
            ParamValue::Reference {
                node_key: a.clone(),
                output_path: output_path.to_owned(),
            },
        );
        let def = make_definition(
            "reference-edge-test",
            vec![
                NodeDefinition::new(a.clone(), "Producer", "core", "producer.action").unwrap(),
                consumer,
            ],
            vec![Connection::new(a.clone(), b.clone()).with_to_port(port_key!("params"))],
        );
        (def, a, b)
    }

    /// A `MapResolver` with `producer.action`'s output and `consumer.action`'s
    /// input set to the given schemas (the other polarity on each side is
    /// `ValidSchema::empty()` — irrelevant to `check_reference_edges`).
    fn resolver_with(producer_output: ValidSchema, consumer_input: ValidSchema) -> MapResolver {
        let mut schemas = HashMap::new();
        schemas.insert(
            "producer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty().into(),
                output: producer_output.into(),
            },
        );
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: consumer_input.into(),
                output: ValidSchema::empty().into(),
            },
        );
        MapResolver(schemas)
    }

    fn assert_no_reference_errors(errors: &[WorkflowError], context: &str) {
        assert!(
            !errors.iter().any(|e| matches!(
                e,
                WorkflowError::ReferencePathUnresolved(_)
                    | WorkflowError::ReferenceTypeIncompatible(_)
                    | WorkflowError::ReferenceTypeUndecidable(_)
            )),
            "{context}: expected zero reference errors; got: {errors:?}"
        );
    }

    /// The round-2 regression, pinned against real derive output: a nested
    /// `serde_json::Value` field derives to an EMPTY `Field::Object`, not
    /// `Any` — the outer producer is still "concretely typed", so a naive
    /// root-only check would never fire. `$.data.foo` must fail open in both
    /// modes.
    #[test]
    fn nested_value_field_reference_fails_open() {
        #[derive(nebula_schema::Schema)]
        #[expect(dead_code, reason = "fields exercised via derive")]
        struct Resp {
            status: String,
            data: serde_json::Value,
        }

        let (def, _, _) = two_node_reference_def("value", "$.data.foo");
        let resolver = resolver_with(schema_of::<Resp>(), ValidSchema::empty());

        for mode in [SchemaCheckMode::Gradual, SchemaCheckMode::Strict] {
            let errors = validate_workflow_with_resolver_mode(&def, &resolver, mode);
            assert_no_reference_errors(&errors, &format!("{mode:?}"));
        }
    }

    /// An Adjacent-tagged enum producer field is a `SchemaKind::Union` root —
    /// opaque outright (step 1 of the walk), regardless of the authored path.
    #[test]
    fn adjacent_tagged_enum_reference_fails_open() {
        #[derive(nebula_schema::Schema, serde::Serialize)]
        #[serde(tag = "type", content = "data")]
        #[expect(dead_code, reason = "variants exercised via derive")]
        enum Event {
            Click { x: i64 },
            Noop,
        }

        let (def, _, _) = two_node_reference_def("value", "$.x");
        let resolver = resolver_with(schema_of::<Event>(), ValidSchema::empty());

        for mode in [SchemaCheckMode::Gradual, SchemaCheckMode::Strict] {
            let errors = validate_workflow_with_resolver_mode(&def, &resolver, mode);
            assert_no_reference_errors(&errors, &format!("{mode:?}"));
        }
    }

    /// Even serde's default (External) tagging fails open — the conservative
    /// `Mode` rule applies to every union regardless of tagging, since
    /// `ModeField` itself carries no tagging discriminant to reason about.
    #[test]
    fn external_tagged_enum_reference_fails_open() {
        #[derive(nebula_schema::Schema, serde::Serialize)]
        #[expect(dead_code, reason = "variants exercised via derive")]
        enum Auth {
            ApiKey { key: String },
            None,
        }

        let (def, _, _) = two_node_reference_def("value", "$.key");
        let resolver = resolver_with(schema_of::<Auth>(), ValidSchema::empty());

        let errors = validate_workflow_with_resolver(&def, &resolver);
        assert_no_reference_errors(&errors, "Gradual");
    }

    /// A reference to a key the producer's real (non-empty) `Object` does not
    /// declare fails open in both modes — never a hard error. `HasSchema` is
    /// unsealed, so a non-empty `Object` cannot be trusted as exhaustive.
    #[test]
    fn missing_object_key_fails_open() {
        #[derive(nebula_schema::Schema)]
        #[expect(dead_code, reason = "fields exercised via derive")]
        struct Contact {
            email: String,
        }
        #[derive(nebula_schema::Schema)]
        #[expect(dead_code, reason = "fields exercised via derive")]
        struct Resp {
            contact: Contact,
        }

        let (def, _, _) = two_node_reference_def("value", "$.contact.phone");
        let resolver = resolver_with(schema_of::<Resp>(), ValidSchema::empty());

        for mode in [SchemaCheckMode::Gradual, SchemaCheckMode::Strict] {
            let errors = validate_workflow_with_resolver_mode(&def, &resolver, mode);
            assert_no_reference_errors(&errors, &format!("{mode:?}"));
        }
    }

    /// End-to-end wiring proof (not just a fail-open assertion): a non-numeric
    /// `List` index on an otherwise fully-closed path is a hard
    /// `ReferencePathUnresolved` in both modes.
    #[test]
    fn non_index_on_list_reference_hard_rejected() {
        #[derive(nebula_schema::Schema)]
        #[expect(dead_code, reason = "fields exercised via derive")]
        struct Resp {
            items: Vec<String>,
        }

        let (def, a, b) = two_node_reference_def("value", "$.items.first");
        let resolver = resolver_with(schema_of::<Resp>(), ValidSchema::empty());

        for mode in [SchemaCheckMode::Gradual, SchemaCheckMode::Strict] {
            let errors = validate_workflow_with_resolver_mode(&def, &resolver, mode);
            let unresolved: Vec<_> = errors
                .iter()
                .filter_map(|e| match e {
                    WorkflowError::ReferencePathUnresolved(details) => Some(details),
                    _ => None,
                })
                .collect();
            assert_eq!(
                unresolved.len(),
                1,
                "{mode:?}: expected exactly one ReferencePathUnresolved; got: {errors:?}"
            );
            assert_eq!(unresolved[0].consumer_node, b, "{mode:?}");
            assert_eq!(unresolved[0].producer_node, a, "{mode:?}");
        }
    }

    /// A reference through only closed nodes, assignable at the leaf → zero
    /// errors (the happy path the whole check exists to allow through).
    #[test]
    fn valid_reference_typechecks_and_passes() {
        #[derive(nebula_schema::Schema)]
        #[expect(dead_code, reason = "fields exercised via derive")]
        struct Contact {
            email: String,
        }
        #[derive(nebula_schema::Schema)]
        #[expect(dead_code, reason = "fields exercised via derive")]
        struct Resp {
            contact: Contact,
        }

        let (def, _, _) = two_node_reference_def("recipient_email", "$.contact.email");
        let resolver = resolver_with(
            schema_of::<Resp>(),
            single_field_schema("recipient_email", true),
        );

        for mode in [SchemaCheckMode::Gradual, SchemaCheckMode::Strict] {
            let errors = validate_workflow_with_resolver_mode(&def, &resolver, mode);
            // Assert the WHOLE validation succeeds (not merely that no
            // reference-specific error fired): `two_node_reference_def`'s
            // connection is wired to a named `to_port`, so the separate
            // main-flow port-schema check never runs on it, and this test's
            // producer/consumer schemas carry no other error source — an
            // `assert_no_reference_errors`-only check here would stay green
            // even if the reference check were broken, as long as nothing
            // else happened to fail too.
            assert!(
                errors.is_empty(),
                "{mode:?}: expected zero errors for a fully-resolved, assignable reference; \
                 got: {errors:?}"
            );
        }
    }

    /// A fully-resolved path whose leaf is providably NOT assignable to the
    /// consumer's expected field is a hard `ReferenceTypeIncompatible` in
    /// both modes (it is never merely undecidable).
    #[test]
    fn reference_type_incompatible_rejected() {
        #[derive(nebula_schema::Schema)]
        #[expect(dead_code, reason = "fields exercised via derive")]
        struct Resp {
            name: String,
        }

        let consumer_input = Schema::builder()
            .add(Field::boolean(FieldKey::new("greeting").unwrap()).required())
            .build()
            .unwrap();

        let (def, a, b) = two_node_reference_def("greeting", "$.name");
        let resolver = resolver_with(schema_of::<Resp>(), consumer_input);

        for mode in [SchemaCheckMode::Gradual, SchemaCheckMode::Strict] {
            let errors = validate_workflow_with_resolver_mode(&def, &resolver, mode);
            let incompatible: Vec<_> = errors
                .iter()
                .filter_map(|e| match e {
                    WorkflowError::ReferenceTypeIncompatible(details) => Some(details),
                    _ => None,
                })
                .collect();
            assert_eq!(
                incompatible.len(),
                1,
                "{mode:?}: expected exactly one ReferenceTypeIncompatible; got: {errors:?}"
            );
            assert_eq!(incompatible[0].consumer_node, b, "{mode:?}");
            assert_eq!(incompatible[0].producer_node, a, "{mode:?}");
        }
    }

    /// A float→int narrowing at a fully-resolved leaf is `Unknown`, not `No`:
    /// Gradual warns-and-passes, Strict blocks with `ReferenceTypeUndecidable`.
    #[test]
    fn reference_type_undecidable_gradual_vs_strict() {
        #[derive(nebula_schema::Schema)]
        #[expect(dead_code, reason = "fields exercised via derive")]
        struct Resp {
            amount: f64,
        }

        let consumer_input = Schema::builder()
            .add(Field::integer(FieldKey::new("qty").unwrap()).required())
            .build()
            .unwrap();

        let (def, _, _) = two_node_reference_def("qty", "$.amount");
        let resolver = resolver_with(schema_of::<Resp>(), consumer_input);

        let gradual_errors = validate_workflow_with_resolver(&def, &resolver);
        assert_no_reference_errors(&gradual_errors, "Gradual");

        let strict_errors =
            validate_workflow_with_resolver_mode(&def, &resolver, SchemaCheckMode::Strict);
        assert!(
            strict_errors
                .iter()
                .any(|e| matches!(e, WorkflowError::ReferenceTypeUndecidable(_))),
            "strict mode must block the undecidable (float\u{2192}int) reference; got: {strict_errors:?}"
        );
        assert!(
            !strict_errors
                .iter()
                .any(|e| matches!(e, WorkflowError::ReferenceTypeIncompatible(_))),
            "a float\u{2192}int narrowing is undecidable, not a provable conflict; got: {strict_errors:?}"
        );
    }

    /// T3.2 parity: when the referenced producer's action is unregistered
    /// (`resolver.io_schemas` returns `None`), the reference is skipped —
    /// fail-open, same posture as the main-flow port check.
    #[test]
    fn reference_producer_unregistered_fails_open() {
        let (def, _, _) = two_node_reference_def("value", "$.anything");
        // Only the consumer resolves; the producer action is absent from the map.
        let mut schemas = HashMap::new();
        schemas.insert(
            "consumer.action".to_owned(),
            NodeIoSchemas {
                input: ValidSchema::empty().into(),
                output: ValidSchema::empty().into(),
            },
        );
        let resolver = MapResolver(schemas);

        let errors = validate_workflow_with_resolver(&def, &resolver);
        assert_no_reference_errors(&errors, "unresolvable producer");
    }

    /// The consumer field is bound by its serde WIRE key (`"to"`), not the
    /// Rust field identifier (`recipient`) — guards the Q2 binding
    /// (`consumer_schemas.input.as_schema().find(&FieldKey::new(param_key))`)
    /// against regression. The consumer field's type is deliberately wrong
    /// (`bool` vs. the producer's `String`) so a successful wire-key lookup
    /// PRODUCES a hard error: if the binding ever regressed to look up the
    /// Rust field name instead, the lookup would silently miss and this test
    /// would go quiet (fail-open) rather than red.
    #[test]
    fn renamed_consumer_field_binds_by_wire_key() {
        #[derive(nebula_schema::Schema)]
        #[expect(dead_code, reason = "fields exercised via derive")]
        struct Resp {
            name: String,
        }

        #[derive(nebula_schema::Schema, serde::Deserialize)]
        #[expect(dead_code, reason = "fields exercised via derive")]
        struct ConsumerParams {
            #[serde(rename = "to")]
            recipient: bool,
        }

        let (def, _, _) = two_node_reference_def("to", "$.name");
        let resolver = resolver_with(schema_of::<Resp>(), schema_of::<ConsumerParams>());

        for mode in [SchemaCheckMode::Gradual, SchemaCheckMode::Strict] {
            let errors = validate_workflow_with_resolver_mode(&def, &resolver, mode);
            let incompatible = errors
                .iter()
                .filter(|e| matches!(e, WorkflowError::ReferenceTypeIncompatible(_)))
                .count();
            assert_eq!(
                incompatible, 1,
                "{mode:?}: the reference must bind the consumer field by its wire key `to`, not \
                 the Rust field name `recipient` — got: {errors:?}"
            );
        }
    }
}
