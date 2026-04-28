use nebula_core::ActionKey;
use nebula_metadata::{BaseMetadata, Metadata};
use nebula_schema::ValidSchema;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::port::{self, InputPort, OutputPort};

/// How isolated this action's execution should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum IsolationLevel {
    /// No isolation. Action runs directly in engine process.
    #[default]
    None,
    /// Capability-gated in-process. SandboxedContext checks declared deps.
    CapabilityGated,
    /// Full isolation — separate process with OS-level hardening
    /// (seccomp/landlock/AppContainer/macOS sandbox) or a microVM.
    /// See `nebula-sandbox`. WASM is an explicit non-goal
    /// (`docs/PRODUCT_CANON.md` §12.6).
    Isolated,
}

/// Broad category of an action — how it behaves in the workflow graph.
///
/// Used by UI editor, workflow validator, and audit log to group and
/// display nodes. Runtime dispatch does **not** depend on this field —
/// it is purely metadata for tooling. The engine routes actions based
/// on which core trait they implement (`StatelessAction`, `StatefulAction`,
/// `TriggerAction`, `ResourceAction`, `AgentAction`), not on `ActionCategory`.
///
/// Categories overlap with but are not identical to core traits: e.g.
/// a `ControlAction` is a `StatelessAction` under the hood, but its
/// category is [`ActionCategory::Control`] so the UI can distinguish
/// it from data-transformation nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActionCategory {
    /// Data-transformation node — takes input, produces output.
    ///
    /// Default for backward compatibility: any metadata serialized
    /// before this field existed deserializes as `Data`.
    #[default]
    Data,
    /// Flow-control node that routes, filters, or gates without
    /// transforming data. Populated automatically by the
    /// `ControlActionAdapter` for members of the `ControlAction` DX
    /// family (If, Switch, Router, Filter, NoOp).
    Control,
    /// Workflow trigger — lives outside the execution graph and
    /// starts new executions.
    Trigger,
    /// Resource provider — supplies scoped capabilities (DB pool,
    /// HTTP client, browser session) to downstream nodes in a branch.
    Resource,
    /// Autonomous agent with an internal reasoning loop and budget.
    Agent,
    /// Terminal control node — ends execution with success or failure
    /// and has no downstream outputs.
    ///
    /// Subcategory of `Control`, distinguished so that the workflow
    /// validator can treat it as a legitimate graph sink even though
    /// its `outputs` list is empty.
    Terminal,
}

/// Compatibility validation errors for action metadata evolution.
///
/// Wraps [`nebula_metadata::BaseCompatError`] (shared catalog-entity rules)
/// and layers the action-specific port-change rule on top.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    /// A generic catalog-citizen rule fired (key / version / schema).
    #[error(transparent)]
    Base(#[from] nebula_metadata::BaseCompatError<ActionKey>),

    /// Input or output ports changed without a major version bump.
    #[error("action ports changed without a major version bump")]
    PortsChangeWithoutMajorBump,
}

/// Static metadata describing an action type.
///
/// Used by the engine for action discovery, capability checks, schema
/// validation, and interface versioning.
///
/// The shared catalog prefix (`key`, `name`, `description`, `schema`, `icon`,
/// `documentation_url`, `tags`, `maturity`, `deprecation`) lives on the
/// composed [`BaseMetadata`]. Entity-specific
/// fields (`version`, ports, `isolation_level`, `category`) stay on this
/// struct.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionMetadata {
    /// Shared catalog prefix — see [`BaseMetadata`]. Carries `version`
    /// (bumped when schema/ports change; exact-match dispatch in the engine).
    #[serde(flatten)]
    pub base: BaseMetadata<ActionKey>,
    /// Input ports this action accepts.
    /// Defaults to a single flow input `"in"`.
    pub inputs: Vec<InputPort>,
    /// Output ports this action produces.
    /// Defaults to a single main flow output `"out"`.
    pub outputs: Vec<OutputPort>,
    /// Isolation level for this action's execution.
    pub isolation_level: IsolationLevel,
    /// Broad category of this action for UI grouping, validator rules,
    /// and audit log filtering. Runtime dispatch does not depend on it.
    ///
    /// Defaults to [`ActionCategory::Data`] for backward compatibility
    /// with metadata serialized before this field existed.
    #[serde(default)]
    pub category: ActionCategory,
    /// Per-action concurrency throttle hint — **persisted hint, not yet
    /// enforced** as of П1.
    ///
    /// The П1 surface lands the field so action authors and registries
    /// can carry it through serialization round-trips, but the engine
    /// scheduler does not currently read it: setting `Some(n)` does
    /// **not** bound in-flight executions today. Enforcement lands in
    /// the engine cluster-mode cascade
    /// (`docs/tracking/cascade-queue.md` slot 2). Until then, treat
    /// this as a stable storage shape, not a runtime guarantee.
    ///
    /// Future contract (when enforced): `None` — engine-global throttle
    /// still applies, but no per-action limit. `Some(n)` — at most `n`
    /// in-flight executions of this action across the engine.
    ///
    /// Per Tech Spec §15.12 F9 + PRODUCT_CANON §11 backpressure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<core::num::NonZeroU32>,
}

impl Metadata for ActionMetadata {
    type Key = ActionKey;
    fn base(&self) -> &BaseMetadata<ActionKey> {
        &self.base
    }
}

impl ActionMetadata {
    /// Create metadata with the minimum required fields.
    #[must_use]
    pub fn new(key: ActionKey, name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            base: BaseMetadata::new(key, name, description, ValidSchema::empty()),
            inputs: port::default_input_ports(),
            outputs: port::default_output_ports(),
            isolation_level: IsolationLevel::None,
            category: ActionCategory::Data,
            max_concurrent: None,
        }
    }

    /// Create metadata whose `parameters` schema is auto-derived from a
    /// [`StatelessAction`](crate::StatelessAction) implementation's `Input`
    /// type.
    ///
    /// Prefer this over [`ActionMetadata::new`] + [`ActionMetadata::with_schema`]
    /// when the author owns a typed `Input` struct — the compiler then
    /// enforces that the declared `Input` type agrees with the schema the
    /// engine, UI, and validator will see.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use nebula_action::{ActionMetadata, StatelessAction};
    /// use nebula_core::action_key;
    ///
    /// struct MyAction;
    /// impl StatelessAction for MyAction {
    ///     type Input = MyInput;  // must impl HasSchema
    ///     type Output = MyOutput;
    ///     // ...
    /// }
    ///
    /// let meta = ActionMetadata::for_stateless::<MyAction>(
    ///     action_key!("my.action"), "My Action", "desc",
    /// );
    /// ```
    #[must_use]
    pub fn for_stateless<A>(
        key: ActionKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self
    where
        A: crate::stateless::StatelessAction,
    {
        Self::new(key, name, description)
            .with_schema(<A::Input as nebula_schema::HasSchema>::schema())
    }

    /// Create metadata whose `parameters` schema is auto-derived from a
    /// [`StatefulAction`](crate::StatefulAction)'s `Input` type.
    #[must_use]
    pub fn for_stateful<A>(
        key: ActionKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self
    where
        A: crate::stateful::StatefulAction,
    {
        Self::new(key, name, description)
            .with_schema(<A::Input as nebula_schema::HasSchema>::schema())
    }

    /// Create metadata whose `parameters` schema is auto-derived from a
    /// [`PaginatedAction`](crate::PaginatedAction)'s `Input` type.
    #[must_use]
    pub fn for_paginated<A>(
        key: ActionKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self
    where
        A: crate::stateful::PaginatedAction,
    {
        Self::new(key, name, description)
            .with_schema(<A::Input as nebula_schema::HasSchema>::schema())
    }

    /// Create metadata whose `parameters` schema is auto-derived from a
    /// [`BatchAction`](crate::BatchAction)'s `Input` type.
    #[must_use]
    pub fn for_batch<A>(
        key: ActionKey,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self
    where
        A: crate::stateful::BatchAction,
    {
        Self::new(key, name, description)
            .with_schema(<A::Input as nebula_schema::HasSchema>::schema())
    }

    /// Set the interface version from `(major, minor)` components.
    ///
    /// Equivalent to `with_version_full(Version::new(major, minor, 0))`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_version(mut self, major: u64, minor: u64) -> Self {
        self.base.version = Version::new(major, minor, 0);
        self
    }

    /// Set the full interface version, including patch and pre-release data.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_version_full(mut self, version: Version) -> Self {
        self.base.version = version;
        self
    }

    /// Set the input port definitions for this action.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_inputs(mut self, inputs: Vec<InputPort>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Append a single input port, preserving already-declared ones.
    #[must_use = "builder methods must be chained or built"]
    pub fn add_input(mut self, port: InputPort) -> Self {
        self.inputs.push(port);
        self
    }

    /// Set the output port definitions for this action.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_outputs(mut self, outputs: Vec<OutputPort>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Append a single output port, preserving already-declared ones.
    #[must_use = "builder methods must be chained or built"]
    pub fn add_output(mut self, port: OutputPort) -> Self {
        self.outputs.push(port);
        self
    }

    /// Bump the major version to `new_major`, zeroing minor and patch.
    #[must_use = "builder methods must be chained or built"]
    pub fn bump_major(mut self, new_major: u64) -> Self {
        self.base.version = Version::new(new_major, 0, 0);
        self
    }

    /// Bump the minor version to `new_minor`, zeroing patch.
    #[must_use = "builder methods must be chained or built"]
    pub fn bump_minor(mut self, new_minor: u64) -> Self {
        self.base.version = Version::new(self.base.version.major, new_minor, 0);
        self
    }

    /// Bump the patch version to `new_patch`.
    #[must_use = "builder methods must be chained or built"]
    pub fn bump_patch(mut self, new_patch: u64) -> Self {
        self.base.version =
            Version::new(self.base.version.major, self.base.version.minor, new_patch);
        self
    }

    /// Set the parameter schema for this action.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_schema(mut self, schema: ValidSchema) -> Self {
        self.base.schema = schema;
        self
    }

    /// Set the isolation level for sandbox routing.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_isolation_level(mut self, level: IsolationLevel) -> Self {
        self.isolation_level = level;
        self
    }

    /// Set the action category (used by UI editor and workflow validator).
    ///
    /// Most authors do not need to call this directly — the
    /// `ControlActionAdapter` and other DX adapters stamp the correct
    /// category when they wrap a typed action.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_category(mut self, category: ActionCategory) -> Self {
        self.category = category;
        self
    }

    /// Set the per-action concurrency throttle hint.
    ///
    /// **Persisted hint, not yet enforced** as of П1 — see
    /// [`max_concurrent`](Self::max_concurrent) field docs for the
    /// enforcement timeline. Builder-style; chainable.
    ///
    /// Per Tech Spec §15.12 F9.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_max_concurrent(mut self, n: core::num::NonZeroU32) -> Self {
        self.max_concurrent = Some(n);
        self
    }

    /// Terminal builder for API consistency with other metadata types.
    #[must_use]
    pub fn build(self) -> Self {
        self
    }

    /// Validate that this metadata update is version-compatible with `previous`.
    ///
    /// Delegates `key immutable / version monotonic / schema-break-requires-
    /// major` to [`nebula_metadata::validate_base_compat`]; layers the action-
    /// specific port-change rule on top.
    ///
    /// The ports rule lives on this type rather than in `nebula-metadata`
    /// because no other catalog citizen exposes a typed input/output port
    /// graph — only actions declare ports that the workflow validator wires
    /// between nodes, so the rule has no meaningful shared form.
    pub fn validate_compatibility(
        &self,
        previous: &Self,
    ) -> Result<(), MetadataCompatibilityError> {
        nebula_metadata::validate_base_compat(&self.base, &previous.base)?;

        let ports_changed = self.inputs != previous.inputs || self.outputs != previous.outputs;
        if ports_changed && self.base.version.major == previous.base.version.major {
            return Err(MetadataCompatibilityError::PortsChangeWithoutMajorBump);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::action_key;
    use nebula_metadata::BaseCompatError;

    use super::*;

    #[test]
    fn metadata_builder() {
        let meta = ActionMetadata::new(
            action_key!("http.request"),
            "HTTP Request",
            "Make HTTP calls",
        )
        .with_version(2, 1);

        assert_eq!(meta.base.key, action_key!("http.request"));
        assert_eq!(meta.base.name, "HTTP Request");
        assert_eq!(meta.base.version, Version::new(2, 1, 0));
    }

    #[test]
    fn metadata_partial_eq_roundtrip() {
        // Two metadata values built identically must compare equal —
        // this is the guarantee downstream registries rely on for
        // deduplication and cache-key checks.
        let a = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc")
            .with_version(1, 2)
            .with_isolation_level(IsolationLevel::CapabilityGated);
        let b = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc")
            .with_version(1, 2)
            .with_isolation_level(IsolationLevel::CapabilityGated);
        assert_eq!(a, b);

        let c = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc").with_version(1, 3);
        assert_ne!(a, c, "different minor version must break equality");
    }

    #[test]
    fn metadata_serde_roundtrip() {
        // Serialize → JSON → deserialize must round-trip cleanly.
        // Engine persistence and cross-process plugin discovery both
        // depend on this contract.
        let original = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc")
            .with_version(2, 1)
            .with_isolation_level(IsolationLevel::Isolated);

        let json = serde_json::to_string(&original).expect("serialization succeeds");
        let decoded: ActionMetadata =
            serde_json::from_str(&json).expect("deserialization succeeds");

        assert_eq!(original, decoded);
    }

    #[test]
    fn category_default_is_data() {
        let meta = ActionMetadata::new(action_key!("test"), "Test", "desc");
        assert_eq!(meta.category, ActionCategory::Data);
    }

    #[test]
    fn category_builder() {
        let meta = ActionMetadata::new(action_key!("if"), "If", "Binary branch")
            .with_category(ActionCategory::Control);
        assert_eq!(meta.category, ActionCategory::Control);
    }

    #[test]
    fn category_serde_roundtrip() {
        let original = ActionMetadata::new(action_key!("stop"), "Stop", "Terminate early")
            .with_category(ActionCategory::Terminal);
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ActionMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.category, ActionCategory::Terminal);
        assert_eq!(original, decoded);
    }

    #[test]
    fn category_backward_compat_without_field() {
        // Metadata serialized before `category` existed must still deserialize.
        // Round-trip an existing metadata through JSON, strip the `category`
        // key, then re-parse — the `#[serde(default)]` attribute must fill
        // the gap with `ActionCategory::Data`.
        let legacy = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc");
        let mut as_value: serde_json::Value = serde_json::to_value(&legacy).unwrap();
        // Simulate pre-field payload by removing the key we just added.
        as_value
            .as_object_mut()
            .unwrap()
            .remove("category")
            .expect("category field must be present after serialize");
        let json_string = serde_json::to_string(&as_value).unwrap();
        let decoded: ActionMetadata = serde_json::from_str(&json_string)
            .expect("legacy metadata without category must deserialize");
        assert_eq!(
            decoded.category,
            ActionCategory::Data,
            "missing category field should default to Data"
        );
    }

    #[test]
    fn default_metadata_values() {
        let meta = ActionMetadata::new(action_key!("test"), "Test", "A test action");
        assert_eq!(meta.base.version, Version::new(1, 0, 0));
        // Default ports
        assert_eq!(meta.inputs.len(), 1);
        assert!(meta.inputs[0].is_flow());
        assert_eq!(meta.inputs[0].key(), "in");
        assert_eq!(meta.outputs.len(), 1);
        assert!(meta.outputs[0].is_flow());
        assert_eq!(meta.outputs[0].key(), "out");
        // Default parameters
        assert!(meta.base.schema.fields().is_empty());
    }

    // ── Port builder tests ──────────────────────────────────────────

    #[test]
    fn with_inputs_builder() {
        let meta = ActionMetadata::new(action_key!("ai.agent"), "AI Agent", "Run agent")
            .with_inputs(vec![
                InputPort::flow("in"),
                InputPort::support("model", "AI Model", "Language model"),
            ]);
        assert_eq!(meta.inputs.len(), 2);
        assert!(meta.inputs[0].is_flow());
        assert!(meta.inputs[1].is_support());
        assert_eq!(meta.inputs[1].key(), "model");
    }

    #[test]
    fn with_outputs_builder() {
        use crate::port::FlowKind;

        let meta = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "Make calls")
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);
        assert_eq!(meta.outputs.len(), 2);
        if let OutputPort::Flow { kind, .. } = &meta.outputs[0] {
            assert_eq!(*kind, FlowKind::Main);
        }
        if let OutputPort::Flow { kind, .. } = &meta.outputs[1] {
            assert_eq!(*kind, FlowKind::Error);
        }
    }

    #[test]
    fn with_dynamic_output() {
        let meta = ActionMetadata::new(action_key!("flow.switch"), "Switch", "Route by conditions")
            .with_inputs(vec![InputPort::flow("in")])
            .with_outputs(vec![OutputPort::dynamic("rule", "rules")]);
        assert_eq!(meta.outputs.len(), 1);
        assert!(meta.outputs[0].is_dynamic());
        assert_eq!(meta.outputs[0].key(), "rule");
    }

    #[test]
    fn with_support_input_full_config() {
        use crate::port::{ConnectionFilter, SupportPort};

        let meta = ActionMetadata::new(action_key!("ai.agent"), "AI Agent", "Run agent")
            .with_inputs(vec![
                InputPort::flow("in"),
                InputPort::Support(SupportPort {
                    key: "tools".into(),
                    name: "Tools".into(),
                    description: "Agent tools".into(),
                    required: false,
                    multi: true,
                    filter: ConnectionFilter::new()
                        .with_allowed_tags(vec!["langchain_tool".into()]),
                }),
            ]);
        assert_eq!(meta.inputs.len(), 2);
        if let InputPort::Support(s) = &meta.inputs[1] {
            assert!(s.multi);
            assert!(!s.required);
            assert!(!s.filter.is_empty());
        } else {
            panic!("expected Support port");
        }
    }

    #[test]
    fn builder_chaining_with_ports() {
        let meta = ActionMetadata::new(action_key!("test"), "Test", "desc")
            .with_version(2, 0)
            .with_inputs(vec![InputPort::flow("in")])
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);

        assert_eq!(meta.base.version, Version::new(2, 0, 0));
        assert_eq!(meta.inputs.len(), 1);
        assert_eq!(meta.outputs.len(), 2);
    }

    #[test]
    fn ports_change_requires_major_bump() {
        let prev = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "desc")
            .with_version(1, 0)
            .with_outputs(vec![OutputPort::flow("out")]);
        let next = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "desc")
            .with_version(1, 1)
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);

        let err = next.validate_compatibility(&prev).unwrap_err();
        assert_eq!(err, MetadataCompatibilityError::PortsChangeWithoutMajorBump);
    }

    #[test]
    fn schema_field_change_requires_major_bump() {
        use nebula_schema::{FieldCollector, Schema};

        let prev =
            ActionMetadata::new(action_key!("http.request"), "HTTP", "desc").with_version(1, 0);
        let next = ActionMetadata::new(action_key!("http.request"), "HTTP", "desc")
            .with_version(1, 1)
            .with_schema(Schema::builder().string("added", |s| s).build().unwrap());

        let err = next.validate_compatibility(&prev).unwrap_err();
        assert_eq!(
            err,
            MetadataCompatibilityError::Base(BaseCompatError::SchemaChangeWithoutMajorBump)
        );
    }

    #[test]
    fn schema_change_with_major_bump_is_valid() {
        let prev = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "desc")
            .with_version(1, 0)
            .with_outputs(vec![OutputPort::flow("out")]);
        let next = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "desc")
            .with_version(2, 0)
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);

        assert!(next.validate_compatibility(&prev).is_ok());
    }

    #[test]
    fn key_change_is_rejected() {
        let prev = ActionMetadata::new(action_key!("a.one"), "A", "desc").with_version(1, 0);
        let next = ActionMetadata::new(action_key!("a.two"), "A", "desc").with_version(2, 0);

        let err = next.validate_compatibility(&prev).unwrap_err();
        assert!(matches!(
            err,
            MetadataCompatibilityError::Base(BaseCompatError::KeyChanged { .. })
        ));
    }

    #[test]
    fn isolation_level_defaults_to_none() {
        let meta = ActionMetadata::new(action_key!("test"), "Test", "A test action");
        assert_eq!(meta.isolation_level, IsolationLevel::None);
        assert_eq!(IsolationLevel::default(), IsolationLevel::None);
    }

    #[test]
    fn with_isolation_level_builder() {
        let meta = ActionMetadata::new(action_key!("test"), "Test", "desc")
            .with_isolation_level(IsolationLevel::Isolated);
        assert_eq!(meta.isolation_level, IsolationLevel::Isolated);
    }

    #[test]
    fn version_regression_is_rejected() {
        let prev = ActionMetadata::new(action_key!("a.one"), "A", "desc").with_version(2, 1);
        let next = ActionMetadata::new(action_key!("a.one"), "A", "desc").with_version(2, 0);

        let err = next.validate_compatibility(&prev).unwrap_err();
        assert!(matches!(
            err,
            MetadataCompatibilityError::Base(BaseCompatError::VersionRegressed { .. })
        ));
    }
}
