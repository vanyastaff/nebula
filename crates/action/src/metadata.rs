use crate::port::{self, InputPort, OutputPort};
use nebula_parameter::collection::ParameterCollection;

// Re-export from core so downstream code can continue using `nebula_action::InterfaceVersion`.
pub use nebula_core::InterfaceVersion;

/// Static metadata describing an action type.
///
/// Used by the engine for action discovery, capability checks, schema
/// validation, and interface versioning.
#[derive(Debug, Clone)]
pub struct ActionMetadata {
    /// Unique key identifying this action type (e.g. `"http.request"`).
    pub key: String,
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
}

impl ActionMetadata {
    /// Create metadata with the minimum required fields.
    pub fn new(
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            description: description.into(),
            version: InterfaceVersion::new(1, 0),
            inputs: port::default_input_ports(),
            outputs: port::default_output_ports(),
            parameters: ParameterCollection::new(),
        }
    }

    /// Set the interface version (major, minor).
    pub fn with_version(mut self, major: u32, minor: u32) -> Self {
        self.version = InterfaceVersion::new(major, minor);
        self
    }

    /// Set the input port definitions for this action.
    pub fn with_inputs(mut self, inputs: Vec<InputPort>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Set the output port definitions for this action.
    pub fn with_outputs(mut self, outputs: Vec<OutputPort>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Set the parameter definitions for this action.
    pub fn with_parameters(mut self, parameters: ParameterCollection) -> Self {
        self.parameters = parameters;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_builder() {
        let meta = ActionMetadata::new("http.request", "HTTP Request", "Make HTTP calls")
            .with_version(2, 1);

        assert_eq!(meta.key, "http.request");
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
    fn default_metadata_values() {
        let meta = ActionMetadata::new("test", "Test", "A test action");
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
        let meta = ActionMetadata::new("ai.agent", "AI Agent", "Run agent").with_inputs(vec![
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

        let meta = ActionMetadata::new("http.request", "HTTP Request", "Make calls")
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
        let meta = ActionMetadata::new("flow.switch", "Switch", "Route by conditions")
            .with_inputs(vec![InputPort::flow("in")])
            .with_outputs(vec![OutputPort::dynamic("rule", "rules")]);
        assert_eq!(meta.outputs.len(), 1);
        assert!(meta.outputs[0].is_dynamic());
        assert_eq!(meta.outputs[0].key(), "rule");
    }

    #[test]
    fn with_support_input_full_config() {
        use crate::port::{ConnectionFilter, SupportPort};

        let meta = ActionMetadata::new("ai.agent", "AI Agent", "Run agent").with_inputs(vec![
            InputPort::flow("in"),
            InputPort::Support(SupportPort {
                key: "tools".into(),
                name: "Tools".into(),
                description: "Agent tools".into(),
                required: false,
                multi: true,
                filter: ConnectionFilter::new().with_allowed_tags(vec!["langchain_tool".into()]),
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
        let meta = ActionMetadata::new("test", "Test", "desc")
            .with_version(2, 0)
            .with_inputs(vec![InputPort::flow("in")])
            .with_outputs(vec![OutputPort::flow("out"), OutputPort::error("error")]);

        assert_eq!(meta.version, InterfaceVersion::new(2, 0));
        assert_eq!(meta.inputs.len(), 1);
        assert_eq!(meta.outputs.len(), 2);
    }
}
