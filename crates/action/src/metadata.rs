use serde::{Deserialize, Serialize};

use crate::capability::{Capability, IsolationLevel};

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
}

/// Interface version — tracks schema compatibility independently of package version.
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
    }
}
