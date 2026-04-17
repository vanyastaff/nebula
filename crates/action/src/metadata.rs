use nebula_core::ActionKey;
use nebula_parameter::collection::ParameterCollection;
use serde::{Deserialize, Serialize};

/// Interface version -- tracks schema compatibility independently of package version.
///
/// - `major` increments on breaking schema changes.
/// - `minor` increments on backward-compatible additions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InterfaceVersion {
    /// Major version -- incremented on breaking schema changes.
    pub major: u32,
    /// Minor version -- incremented on backward-compatible additions.
    pub minor: u32,
}

impl InterfaceVersion {
    /// Create a new interface version.
    pub fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// Check if `other` is compatible with `self`.
    ///
    /// Compatible means same major version and `other.minor >= self.minor`.
    pub fn is_compatible_with(&self, other: &InterfaceVersion) -> bool {
        self.major == other.major && other.minor >= self.minor
    }
}

impl std::fmt::Display for InterfaceVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

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

/// Compatibility validation errors for metadata evolution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataCompatibilityError {
    /// Action key changed across versions.
    #[error("action key changed from `{previous}` to `{current}`")]
    KeyChanged {
        /// Previous key.
        previous: ActionKey,
        /// Current key.
        current: ActionKey,
    },
    /// Interface version regressed.
    #[error(
        "interface version regressed from {previous_major}.{previous_minor} to {current_major}.{current_minor}"
    )]
    VersionRegressed {
        /// Previous major.
        previous_major: u32,
        /// Previous minor.
        previous_minor: u32,
        /// Current major.
        current_major: u32,
        /// Current minor.
        current_minor: u32,
    },
    /// Breaking schema change without a major version bump.
    #[error("breaking metadata change detected without major version bump")]
    BreakingChangeWithoutMajorBump,
}

/// Static metadata describing an action type.
///
/// Used by the engine for action discovery, capability checks, schema
/// validation, and interface versioning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionMetadata {
    /// Unique key identifying this action type (e.g. `"http.request"`).
    pub key: ActionKey,
    /// Human-readable display name (e.g. `"HTTP Request"`).
    pub name: String,
    /// Short description of what this action does.
    pub description: String,
    /// Interface version — changes only when input/output schema changes.
    pub version: InterfaceVersion,
    /// Input ports this action accepts.
    /// Defaults to a single flow input `"in"`.
    pub inputs: Vec<InputPort>,
    /// Output ports this action produces.
    /// Defaults to a single main flow output `"out"`.
    pub outputs: Vec<OutputPort>,
    /// Parameter definitions for this action (from nebula-parameter).
    pub parameters: ParameterCollection,
    /// Isolation level for this action's execution.
    pub isolation_level: IsolationLevel,
    /// Broad category of this action for UI grouping, validator rules,
    /// and audit log filtering. Runtime dispatch does not depend on it.
    ///
    /// Defaults to [`ActionCategory::Data`] for backward compatibility
    /// with metadata serialized before this field existed.
    #[serde(default)]
    pub category: ActionCategory,
}

impl ActionMetadata {
    /// Create metadata with the minimum required fields.
    #[must_use]
    pub fn new(key: ActionKey, name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            key,
            name: name.into(),
            description: description.into(),
            version: InterfaceVersion::new(1, 0),
            inputs: port::default_input_ports(),
            outputs: port::default_output_ports(),
            parameters: ParameterCollection::new(),
            isolation_level: IsolationLevel::None,
            category: ActionCategory::Data,
        }
    }

    /// Set the interface version (major, minor).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_version(mut self, major: u32, minor: u32) -> Self {
        self.version = InterfaceVersion::new(major, minor);
        self
    }

    /// Set the input port definitions for this action.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_inputs(mut self, inputs: Vec<InputPort>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Set the output port definitions for this action.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_outputs(mut self, outputs: Vec<OutputPort>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Set the parameter definitions for this action.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_parameters(mut self, parameters: ParameterCollection) -> Self {
        self.parameters = parameters;
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

    /// Validate that this metadata update is version-compatible with `previous`.
    ///
    /// Rules:
    /// - `key` is immutable across versions.
    /// - Interface version cannot go backwards.
    /// - If input/output/parameter schema changed, major must increase.
    pub fn validate_compatibility(
        &self,
        previous: &Self,
    ) -> Result<(), MetadataCompatibilityError> {
        if self.key != previous.key {
            return Err(MetadataCompatibilityError::KeyChanged {
                previous: previous.key.clone(),
                current: self.key.clone(),
            });
        }

        let regressed = self.version.major < previous.version.major
            || (self.version.major == previous.version.major
                && self.version.minor < previous.version.minor);
        if regressed {
            return Err(MetadataCompatibilityError::VersionRegressed {
                previous_major: previous.version.major,
                previous_minor: previous.version.minor,
                current_major: self.version.major,
                current_minor: self.version.minor,
            });
        }

        let schema_changed = self.inputs != previous.inputs
            || self.outputs != previous.outputs
            || self.parameters != previous.parameters;
        if schema_changed && self.version.major == previous.version.major {
            return Err(MetadataCompatibilityError::BreakingChangeWithoutMajorBump);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::action_key;

    use super::*;

    #[test]
    fn metadata_builder() {
        let meta = ActionMetadata::new(
            action_key!("http.request"),
            "HTTP Request",
            "Make HTTP calls",
        )
        .with_version(2, 1);

        assert_eq!(meta.key, action_key!("http.request"));
        assert_eq!(meta.name, "HTTP Request");
        assert_eq!(meta.version, InterfaceVersion::new(2, 1));
    }

    #[test]
    fn interface_version_compatibility() {
        let v1_0 = InterfaceVersion::new(1, 0);
        let v1_2 = InterfaceVersion::new(1, 2);
        let v2_0 = InterfaceVersion::new(2, 0);

        // v1.2 is compatible with v1.0 requirement
        assert!(v1_0.is_compatible_with(&v1_2));
        // v1.0 is NOT compatible with v1.2 requirement (minor too low)
        assert!(!v1_2.is_compatible_with(&v1_0));
        // Different major = incompatible
        assert!(!v1_0.is_compatible_with(&v2_0));
    }

    #[test]
    fn interface_version_display() {
        assert_eq!(InterfaceVersion::new(1, 3).to_string(), "1.3");
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
        assert_eq!(meta.version, InterfaceVersion::new(1, 0));
        // Default ports
        assert_eq!(meta.inputs.len(), 1);
        assert!(meta.inputs[0].is_flow());
        assert_eq!(meta.inputs[0].key(), "in");
        assert_eq!(meta.outputs.len(), 1);
        assert!(meta.outputs[0].is_flow());
        assert_eq!(meta.outputs[0].key(), "out");
        // Default parameters
        assert!(meta.parameters.is_empty());
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

        assert_eq!(meta.version, InterfaceVersion::new(2, 0));
        assert_eq!(meta.inputs.len(), 1);
        assert_eq!(meta.outputs.len(), 2);
    }

    #[test]
    fn schema_change_requires_major_bump() {
        let prev = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "desc")
            .with_version(1, 0)
            .with_outputs(vec![OutputPort::flow("out")]);
        let next = ActionMetadata::new(action_key!("http.request"), "HTTP Request", "desc")
            .with_version(1, 1)
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);

        let err = next.validate_compatibility(&prev).unwrap_err();
        assert_eq!(
            err,
            MetadataCompatibilityError::BreakingChangeWithoutMajorBump
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
        assert!(matches!(err, MetadataCompatibilityError::KeyChanged { .. }));
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
            MetadataCompatibilityError::VersionRegressed { .. }
        ));
    }
}
