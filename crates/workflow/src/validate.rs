//! Comprehensive workflow validation that collects all errors.

use std::collections::HashSet;

use nebula_schema::{
    Assignability, FieldKey, PathWalk, explain_assignable, explain_field_assignable,
    explain_root_field_assignable, is_opaque_field_node,
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
    /// untyped `serde_json::Value` / `Dynamic` workflows. This is an explicit
    /// workflow admission policy, not evidence of schema assignability.
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
            // Provably compatible: admit in both policy modes.
            Assignability::Yes => {},
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
            // Gradual mode deliberately admits current undecidable outcomes.
            Assignability::Unknown(_) => {},
            // `Assignability` is `#[non_exhaustive]`. A future verdict may pass
            // only under deliberate Gradual policy; Strict rejects until
            // workflow understands the new semantics.
            _ => match mode {
                SchemaCheckMode::Gradual => {},
                SchemaCheckMode::Strict => {
                    errors.push(WorkflowError::PortSchemaUndecidable(Box::new(
                        crate::error::PortSchemaUndecidableDetails {
                            from_node: conn.from_node.clone(),
                            to_node: conn.to_node.clone(),
                            from_port: conn.from_port.clone(),
                            to_port: conn.to_port.clone(),
                            reasons: Vec::new(),
                        },
                    )));
                },
            },
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
/// from **any** node's output through an arbitrary canonical RFC 6901 path,
/// entirely outside the main-flow port shape.
///
/// Fail-open (no error pushed) when:
/// - the referenced producer node is missing from `node_by_id`, or either
///   endpoint's schema does not resolve (`resolver.io_schemas` returns
///   `None`) — mirrors the main-flow edge check's fail-open contract;
/// - `ValidSchema::walk_reference_path` returns [`PathWalk::Opaque`] — an
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
/// resolves and the selected field or complete root is provably not assignable
/// to the consumer's expected field ([`Assignability::No`]), pushes
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
    enum ProducerReference<'a> {
        Root,
        Property(&'a nebula_schema::Property),
    }

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
            // both modes); successful outcomes retain whether the reference
            // selected the complete root or one field.
            let producer_reference = match producer_schemas
                .output
                .as_schema()
                .walk_reference_path(output_path)
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
                PathWalk::ResolvedRoot => ProducerReference::Root,
                PathWalk::Resolved(property) => ProducerReference::Property(property),
                // `PathWalk` is `#[non_exhaustive]`. Only outcomes that the
                // schema walker explicitly classifies as opaque may fail open;
                // workflow must understand any future outcome before admitting
                // references carrying it.
                _ => {
                    errors.push(WorkflowError::ReferencePathUnresolved(Box::new(
                        crate::error::ReferencePathUnresolvedDetails {
                            consumer_node: consumer_node.id.clone(),
                            param_key: param_key.clone(),
                            producer_node: producer_key.clone(),
                            output_path: output_path.clone(),
                            reason: "unsupported schema reference-path outcome".to_owned(),
                        },
                    )));
                    continue;
                },
            };

            // Resolve the consumer's expected field for this parameter. The
            // parameter key equals the consumer `InputSchema` field key by
            // construction (schema keys mirror serde wire keys, including
            // `#[serde(rename)]`ed fields — see the derive macro's own test
            // suite). Undeterminable (invalid key, no such field, or the field
            // is itself opaque per the same classification the walk above
            // uses) → fail-open, skip only the type check; the walk's
            // successful verdict above already stands on its own.
            let Ok(consumer_key) = FieldKey::new(param_key.as_str()) else {
                continue;
            };
            let Some(consumer_field) = consumer_schemas
                .input
                .as_schema()
                .find_property(&consumer_key)
            else {
                continue;
            };
            if is_opaque_field_node(consumer_field) {
                continue;
            }

            let assignability = match producer_reference {
                ProducerReference::Root => {
                    explain_root_field_assignable(&producer_schemas.output, consumer_field)
                },
                ProducerReference::Property(property) => {
                    explain_field_assignable(property, consumer_field)
                },
            };
            match assignability {
                // Provably compatible: admit in both policy modes.
                Assignability::Yes => {},
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
                // Gradual mode deliberately admits current undecidable outcomes.
                Assignability::Unknown(_) => {},
                // Future verdicts remain gradual-only until workflow learns
                // their semantics; Strict rejects with the existing typed
                // undecidable diagnostic and its no-detail sentinel.
                _ => match mode {
                    SchemaCheckMode::Gradual => {},
                    SchemaCheckMode::Strict => {
                        errors.push(WorkflowError::ReferenceTypeUndecidable(Box::new(
                            crate::error::ReferenceTypeUndecidableDetails {
                                consumer_node: consumer_node.id.clone(),
                                param_key: param_key.clone(),
                                producer_node: producer_key.clone(),
                                output_path: output_path.clone(),
                                reasons: Vec::new(),
                            },
                        )));
                    },
                },
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
#[path = "validate_tests.rs"]
mod tests;
