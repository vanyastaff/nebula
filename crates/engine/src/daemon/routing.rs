//! Trigger-dispatch routing seam.
//!
//! [`RoutingResolver`] is a port (DIP seam) the [`DurableExecutionEmitter`]
//! calls to determine which `required_plugin_key` (and flavour SHA) should be
//! attached to each enqueued [`JobDispatchMsg`].  The engine's orchestrator
//! pull-loop claims only rows whose `required_plugin_key` is in the worker's
//! advertised capability set, so the resolver is the single place that maps a
//! `(validated_workflow, trigger_id)` pair onto a dispatch route.
//!
//! ## D1 implementation
//!
//! This module ships [`DefinitionRoutingResolver`] — a registry-free resolver
//! that reads `plugin_key` directly from the [`ValidatedWorkflow`]'s inner
//! definition.  No external lookup is required: the definition is
//! self-describing.  Taking [`ValidatedWorkflow`] rather than a raw
//! `WorkflowDefinition` closes the duplicate-trigger-id hole
//! by construction: validation rejects duplicate `trigger_binding` ids before
//! the resolver is ever reached, so `.find()` always picks a unique match.
//!
//! [`DurableExecutionEmitter`]: super::durable_emitter::DurableExecutionEmitter
//! [`JobDispatchMsg`]: nebula_storage_port::dto::JobDispatchMsg

use std::collections::BTreeSet;

use nebula_core::{NodeKey, PluginKey};
use nebula_workflow::ValidatedWorkflow;

// ── types ─────────────────────────────────────────────────────────────────────

/// A resolved dispatch route for one `(validated_workflow, fired_trigger_id)`
/// pair.
///
/// Returned by [`RoutingResolver::resolve`].  The `required_plugin_key` is
/// written into the `JobDispatchMsg::required_plugin_key` field; the
/// orchestrator claims rows whose `required_plugin_key` is a member of the
/// worker's `available_plugins`.
///
/// `required_plugins` carries the full set of plugin keys the workflow needs
/// (trigger + enabled nodes, deduplicated and sorted).  The claim predicate
/// is the superset check `job.required_plugins ⊆ worker.available_plugins`;
/// `required_plugin_key` is kept as an index-friendly pre-filter (sound
/// because `required_plugins ⊇ {required_plugin_key}` by construction).
///
/// `target_flavor_sha` is a version-pin guard written into the message but
/// not yet used for routing.
#[must_use = "a DispatchRoute must be written into the JobDispatchMsg; dropping it yields an un-claimable job"]
#[derive(Debug, Clone)]
pub struct DispatchRoute {
    /// The primary required plugin (the trigger's plugin); an element of
    /// `required_plugins`; used as the index pre-filter.
    pub required_plugin_key: PluginKey,
    /// Full set of plugin keys the workflow needs (trigger binding + enabled
    /// nodes, deduplicated and sorted).  Superset of `{required_plugin_key}`.
    pub required_plugins: Vec<PluginKey>,
    /// SHA of the plugin flavor this message targets (version-pin guard; not
    /// yet used for routing).
    pub target_flavor_sha: String,
}

/// Errors returned by [`RoutingResolver::resolve`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RoutingError {
    /// The fired trigger id is not present in the workflow's `trigger_bindings`.
    ///
    /// Fail-closed: a trigger absent from the validated definition can never
    /// produce a valid dispatch route, so the job is rejected rather than
    /// routing to an arbitrary worker.
    #[error("trigger `{trigger_id}` not found in trigger_bindings of workflow `{workflow_id}`")]
    TriggerNotOnWorkflow {
        /// The workflow whose definition was inspected.
        workflow_id: String,
        /// The trigger node key that was not present.
        trigger_id: String,
    },
}

// ── trait ─────────────────────────────────────────────────────────────────────

/// Port for mapping `(validated_workflow, fired_trigger_id)` → [`DispatchRoute`].
///
/// Taking [`ValidatedWorkflow`] rather than a raw `WorkflowDefinition` makes
/// "must validate before dispatch" a compile-time obligation: callers cannot
/// reach this seam with an unvalidated definition.
///
/// The [`DurableExecutionEmitter`] holds an `Arc<dyn RoutingResolver>` so the
/// routing strategy can be swapped without touching the emitter.  The
/// production implementation, [`DefinitionRoutingResolver`], reads `plugin_key`
/// directly from the definition — no registry needed.
///
/// [`DurableExecutionEmitter`]: super::durable_emitter::DurableExecutionEmitter
pub trait RoutingResolver: Send + Sync + std::fmt::Debug {
    /// Resolve the dispatch route for a fired trigger.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::TriggerNotOnWorkflow`] when `fired_trigger_id`
    /// is not present in `workflow.definition().trigger_bindings`.
    fn resolve(
        &self,
        workflow: &ValidatedWorkflow,
        fired_trigger_id: &NodeKey,
    ) -> Result<DispatchRoute, RoutingError>;
}

// ── DefinitionRoutingResolver ─────────────────────────────────────────────────

/// Pinned flavor SHA used by the slice harness.
///
/// A single constant avoids a freshly minted random value per call (which
/// would defeat version-pin guards in integration tests).
pub const SLICE_FLAVOR_SHA: &str = "slice-flavor-0000000000000000000000000000000000000001";

/// Registry-free routing resolver that reads plugin routing data directly from
/// the validated workflow definition.
///
/// No external registry is required: the definition carries an explicit
/// `plugin_key` on every [`TriggerBinding`] and enabled [`NodeDefinition`].
///
/// ## Route derivation
///
/// 1. Locate the [`TriggerBinding`] whose `id` matches `fired_trigger_id` —
///    fail closed ([`RoutingError::TriggerNotOnWorkflow`]) if absent.
///    Because [`ValidatedWorkflow`] rejects duplicate trigger-binding ids, the
///    `.find()` result is always unique.
/// 2. The binding's `plugin_key` becomes `required_plugin_key`.
/// 3. `required_plugins` is the deduplicated, sorted union of `plugin_key`
///    values from every trigger binding **and** every **enabled** node.
///    Disabled nodes are excluded — they are skipped at execution time and
///    their plugin is not required on the target worker.
///    The required key is always a member.
///
/// [`TriggerBinding`]: nebula_workflow::TriggerBinding
/// [`NodeDefinition`]: nebula_workflow::NodeDefinition
#[derive(Debug, Clone)]
pub struct DefinitionRoutingResolver {
    flavor_sha: String,
}

impl DefinitionRoutingResolver {
    /// Build a resolver pinned to the given flavor SHA.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when `flavor_sha` is empty (an empty SHA would
    /// make the version-pin guard trivially match any job, defeating its
    /// purpose).
    #[must_use]
    pub fn new(flavor_sha: impl Into<String>) -> Self {
        let sha = flavor_sha.into();
        debug_assert!(!sha.is_empty(), "flavor_sha must not be empty");
        Self { flavor_sha: sha }
    }
}

impl Default for DefinitionRoutingResolver {
    /// Returns a resolver pinned to [`SLICE_FLAVOR_SHA`].
    fn default() -> Self {
        Self::new(SLICE_FLAVOR_SHA)
    }
}

impl RoutingResolver for DefinitionRoutingResolver {
    #[tracing::instrument(
        level = "debug",
        skip(self, workflow),
        fields(
            fired_trigger_id = %fired_trigger_id,
            workflow_id      = %workflow.definition().id,
        )
    )]
    fn resolve(
        &self,
        workflow: &ValidatedWorkflow,
        fired_trigger_id: &NodeKey,
    ) -> Result<DispatchRoute, RoutingError> {
        let def = workflow.definition();

        // Step 1 — locate the trigger binding (fail closed if absent).
        // ValidatedWorkflow guarantees no duplicate trigger-binding ids, so
        // this find always returns the unique match.
        let binding = def
            .trigger_bindings
            .iter()
            .find(|b| b.id == *fired_trigger_id)
            .ok_or_else(|| RoutingError::TriggerNotOnWorkflow {
                workflow_id: def.id.to_string(),
                trigger_id: fired_trigger_id.to_string(),
            })?;

        // Step 2 — the binding's plugin_key is the required routing key.
        // `plugin_key` is already a `PluginKey` — no stringification needed.
        let required_plugin_key = binding.plugin_key.clone();
        tracing::debug!(
            required_plugin_key = %required_plugin_key,
            "resolved required plugin key"
        );

        // Step 3 — required_plugins: deduplicated, sorted union of plugin_key
        // values from all trigger bindings and all ENABLED nodes.
        // Disabled nodes are skipped at execution time; their plugin is not
        // needed on the target worker.
        let mut keys: BTreeSet<&PluginKey> = BTreeSet::new();
        for tb in &def.trigger_bindings {
            keys.insert(&tb.plugin_key);
        }
        for node in &def.nodes {
            if node.enabled {
                keys.insert(&node.plugin_key);
            }
        }
        // The required key is always present: we just resolved it from the
        // trigger bindings, which are unconditionally added above.
        let required_plugins: Vec<PluginKey> = keys.into_iter().cloned().collect();
        tracing::debug!(
            required_plugin_count = required_plugins.len(),
            "resolved required plugins"
        );

        Ok(DispatchRoute {
            required_plugin_key,
            required_plugins,
            target_flavor_sha: self.flavor_sha.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use nebula_core::{PluginKey, WorkflowId, node_key, plugin_key};
    use nebula_workflow::{
        CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, TriggerBinding, ValidatedWorkflow,
        Version, WorkflowConfig, WorkflowDefinition,
    };

    use super::*;

    /// Build a minimal validated workflow.
    ///
    /// `trigger_plugin` is the plugin for the single trigger binding with
    /// `id == trigger_id`.  `nodes` is a slice of `(plugin_key, enabled)` pairs.
    fn make_validated(
        trigger_id: &str,
        trigger_plugin: &str,
        nodes: &[(&str, bool)],
    ) -> ValidatedWorkflow {
        let now = chrono::Utc::now();
        let trigger_key = NodeKey::new(trigger_id).unwrap();
        let trigger = TriggerBinding::new(trigger_key, trigger_plugin, "test.action").unwrap();
        let node_defs: Vec<NodeDefinition> = nodes
            .iter()
            .enumerate()
            .map(|(i, &(pk, enabled))| {
                let mut n = NodeDefinition::new(
                    NodeKey::new(format!("node{i}")).unwrap(),
                    "Node",
                    pk,
                    "test.action",
                )
                .unwrap();
                if !enabled {
                    n = n.disabled();
                }
                n
            })
            .collect();
        let def = WorkflowDefinition {
            id: WorkflowId::new(),
            name: "test-routing".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: node_defs,
            connections: Vec::<Connection>::new(),
            variables: HashMap::new(),
            config: WorkflowConfig::default(),
            trigger_bindings: vec![trigger],
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: CURRENT_SCHEMA_VERSION,
        };
        ValidatedWorkflow::validate(def).expect("test definition must be valid")
    }

    #[test]
    fn fail_closed_when_trigger_not_in_bindings() {
        // Verifies the fail-closed path: a fired trigger id absent from
        // trigger_bindings must return TriggerNotOnWorkflow.
        let workflow = make_validated("real.trigger", "p.trig", &[("p.a", true)]);
        let resolver = DefinitionRoutingResolver::default();
        let absent = node_key!("absent.trigger");
        let err = resolver
            .resolve(&workflow, &absent)
            .expect_err("trigger absent from bindings must fail closed");
        assert!(
            matches!(err, RoutingError::TriggerNotOnWorkflow { .. }),
            "expected TriggerNotOnWorkflow, got {err:?}"
        );
        let RoutingError::TriggerNotOnWorkflow {
            trigger_id,
            workflow_id: _,
        } = err;
        assert_eq!(trigger_id, "absent.trigger");
    }

    #[test]
    fn resolves_required_key_and_sorted_required_plugins() {
        // trigger plugin = "p.trig", node plugins = [("p.a", true), ("p.b", true)]
        // expected required_plugins = sorted { "p.a", "p.b", "p.trig" }
        let workflow = make_validated("test.trigger", "p.trig", &[("p.a", true), ("p.b", true)]);
        let resolver = DefinitionRoutingResolver::new(SLICE_FLAVOR_SHA);
        let fired = node_key!("test.trigger");
        let route = resolver
            .resolve(&workflow, &fired)
            .expect("trigger present in bindings must resolve");

        assert_eq!(route.required_plugin_key, plugin_key!("p.trig"));
        assert_eq!(route.target_flavor_sha, SLICE_FLAVOR_SHA);

        let plugins: Vec<&str> = route
            .required_plugins
            .iter()
            .map(PluginKey::as_str)
            .collect();
        // Sorted BTreeSet order: p.a < p.b < p.trig
        assert_eq!(plugins, vec!["p.a", "p.b", "p.trig"]);

        // Required key is a member of the required_plugins set.
        assert!(
            route.required_plugins.contains(&route.required_plugin_key),
            "required_plugin_key must be in required_plugins"
        );
    }

    #[test]
    fn disabled_node_plugin_excluded_from_required_plugins() {
        // "p.disabled" belongs to a disabled node — it must NOT appear.
        // "p.enabled" belongs to an enabled node — it MUST appear.
        // "p.trig" is the trigger binding — it MUST appear.
        let workflow = make_validated(
            "test.trigger",
            "p.trig",
            &[("p.enabled", true), ("p.disabled", false)],
        );
        let resolver = DefinitionRoutingResolver::default();
        let route = resolver
            .resolve(&workflow, &node_key!("test.trigger"))
            .unwrap();

        let trig = plugin_key!("p.trig");
        let enabled = plugin_key!("p.enabled");
        let disabled = "p.disabled".parse::<PluginKey>().unwrap();
        assert!(
            route.required_plugins.contains(&trig),
            "trigger plugin must be in required_plugins; got {:?}",
            route.required_plugins
        );
        assert!(
            route.required_plugins.contains(&enabled),
            "enabled node plugin must be in required_plugins; got {:?}",
            route.required_plugins
        );
        assert!(
            !route.required_plugins.contains(&disabled),
            "disabled node plugin must NOT be in required_plugins; got {:?}",
            route.required_plugins
        );
    }

    #[test]
    fn required_plugins_deduplicated_when_trigger_and_node_share_plugin() {
        // trigger plugin = "p.shared", node plugin = "p.shared" — only one entry.
        let workflow = make_validated("test.trigger", "p.shared", &[("p.shared", true)]);
        let resolver = DefinitionRoutingResolver::default();
        let fired = node_key!("test.trigger");
        let route = resolver.resolve(&workflow, &fired).unwrap();

        assert_eq!(route.required_plugins.len(), 1);
        assert_eq!(route.required_plugins[0], plugin_key!("p.shared"));
    }

    #[test]
    fn default_uses_slice_flavor_sha() {
        let workflow = make_validated("t", "some.plugin", &[("some.plugin", true)]);
        let resolver = DefinitionRoutingResolver::default();
        let route = resolver.resolve(&workflow, &node_key!("t")).unwrap();
        assert_eq!(route.target_flavor_sha, SLICE_FLAVOR_SHA);
    }
}
