use nebula_parameter::collection::ParameterCollection;
use serde::{Deserialize, Serialize};

use crate::capability::{Capability, IsolationLevel};

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
    /// Category for UI grouping (e.g. `"network"`, `"transform"`, `"database"`).
    pub category: String,
    /// Interface version — changes only when input/output schema changes.
    pub version: InterfaceVersion,
    /// Capabilities this action requires from the runtime.
    pub capabilities: Vec<Capability>,
    /// Required isolation level.
    pub isolation_level: IsolationLevel,
    /// Whether inputs/outputs are strongly typed or dynamic JSON.
    pub execution_mode: ExecutionMode,
    /// JSON Schema for input validation (optional).
    pub input_schema: Option<serde_json::Value>,
    /// JSON Schema for output validation (optional).
    pub output_schema: Option<serde_json::Value>,
    /// User-facing configuration parameters for this action.
    /// Describes the form fields shown in the workflow editor when configuring this node.
    /// Validation of values against this collection is the engine's responsibility.
    pub parameters: Option<ParameterCollection>,
    /// Credential types this action requires, referenced by key.
    /// The engine resolves these to actual credentials at runtime.
    pub required_credentials: Vec<String>,
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
            category: String::new(),
            version: InterfaceVersion::new(1, 0),
            capabilities: Vec::new(),
            isolation_level: IsolationLevel::default(),
            execution_mode: ExecutionMode::Dynamic,
            input_schema: None,
            output_schema: None,
            parameters: None,
            required_credentials: Vec::new(),
        }
    }

    /// Set the UI category for this action.
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    /// Set the interface version (major, minor).
    pub fn with_version(mut self, major: u32, minor: u32) -> Self {
        self.version = InterfaceVersion::new(major, minor);
        self
    }

    /// Add a required capability.
    pub fn with_capability(mut self, cap: Capability) -> Self {
        self.capabilities.push(cap);
        self
    }

    /// Set the required isolation level.
    pub fn with_isolation(mut self, level: IsolationLevel) -> Self {
        self.isolation_level = level;
        self
    }

    /// Set the execution mode (typed or dynamic).
    pub fn with_execution_mode(mut self, mode: ExecutionMode) -> Self {
        self.execution_mode = mode;
        self
    }

    /// Set the JSON Schema for input validation.
    pub fn with_input_schema(mut self, schema: serde_json::Value) -> Self {
        self.input_schema = Some(schema);
        self
    }

    /// Set the JSON Schema for output validation.
    pub fn with_output_schema(mut self, schema: serde_json::Value) -> Self {
        self.output_schema = Some(schema);
        self
    }

    /// Set user-facing configuration parameters for this action.
    pub fn with_parameters(mut self, parameters: ParameterCollection) -> Self {
        self.parameters = Some(parameters);
        self
    }

    /// Add a credential type this action requires.
    pub fn with_required_credential(mut self, credential_key: impl Into<String>) -> Self {
        self.required_credentials.push(credential_key.into());
        self
    }
}

/// Discriminant for the action type hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionType {
    /// Stateless single-execution action.
    Process,
    /// Iterative action with persistent state.
    Stateful,
    /// Event source that starts workflows.
    Trigger,
    /// Continuous stream producer.
    Streaming,
    /// Distributed transaction participant (saga pattern).
    Transactional,
    /// Human-in-the-loop interaction.
    Interactive,
}

/// Whether action I/O is strongly typed or dynamic JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionMode {
    /// Input/Output implement `Serialize + Deserialize` with compile-time checks.
    Typed,
    /// `serde_json::Value` with runtime JSON Schema validation.
    Dynamic,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_builder() {
        let meta = ActionMetadata::new("http.request", "HTTP Request", "Make HTTP calls")
            .with_category("network")
            .with_version(2, 1)
            .with_execution_mode(ExecutionMode::Typed);

        assert_eq!(meta.key, "http.request");
        assert_eq!(meta.name, "HTTP Request");
        assert_eq!(meta.category, "network");
        assert_eq!(meta.version, InterfaceVersion::new(2, 1));
        assert_eq!(meta.execution_mode, ExecutionMode::Typed);
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
        assert_eq!(meta.isolation_level, IsolationLevel::default());
        assert_eq!(meta.execution_mode, ExecutionMode::Dynamic);
        assert!(meta.capabilities.is_empty());
        assert!(meta.input_schema.is_none());
        assert!(meta.output_schema.is_none());
        assert!(meta.parameters.is_none());
        assert!(meta.required_credentials.is_empty());
    }

    #[test]
    fn with_parameters_builder() {
        use nebula_parameter::prelude::*;

        let params = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("url", "URL")))
            .with(ParameterDef::Select(SelectParameter::new(
                "method", "Method",
            )));

        let meta = ActionMetadata::new("http.request", "HTTP Request", "Make HTTP calls")
            .with_parameters(params);

        let params = meta.parameters.expect("parameters should be Some");
        assert_eq!(params.len(), 2);
        assert_eq!(params.get_by_key("url").unwrap().key(), "url");
        assert_eq!(params.get_by_key("method").unwrap().key(), "method");
    }

    #[test]
    fn with_required_credential_builder() {
        let meta = ActionMetadata::new("slack.send", "Slack Send", "Send a Slack message")
            .with_required_credential("slack_oauth")
            .with_required_credential("webhook_secret");

        assert_eq!(meta.required_credentials.len(), 2);
        assert_eq!(meta.required_credentials[0], "slack_oauth");
        assert_eq!(meta.required_credentials[1], "webhook_secret");
    }

    #[test]
    fn builder_chaining_all_new_fields() {
        use nebula_parameter::prelude::*;

        let params = ParameterCollection::new()
            .with(ParameterDef::Text(TextParameter::new("channel", "Channel")));

        let meta = ActionMetadata::new("slack.send", "Slack Send", "Send message")
            .with_category("messaging")
            .with_parameters(params)
            .with_required_credential("slack_oauth")
            .with_execution_mode(ExecutionMode::Dynamic);

        assert_eq!(meta.category, "messaging");
        assert!(meta.parameters.is_some());
        assert_eq!(meta.required_credentials, vec!["slack_oauth"]);
        assert_eq!(meta.execution_mode, ExecutionMode::Dynamic);
    }

    #[test]
    fn parameters_none_by_default() {
        let meta = ActionMetadata::new("noop", "No-Op", "Does nothing");
        assert!(meta.parameters.is_none());
        assert!(meta.required_credentials.is_empty());
        // Existing fields still have their defaults
        assert!(meta.input_schema.is_none());
        assert!(meta.output_schema.is_none());
    }
}
