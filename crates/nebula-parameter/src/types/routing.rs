use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};

use crate::ParameterError;
use crate::core::{
    Describable, Displayable, Parameter, ParameterDisplay, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Configuration options for routing parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingParameterOptions {
    /// Label to display on the connection point
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_label: Option<String>,

    /// Description for the connection point
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_description: Option<String>,

    /// Whether a connection is required for this parameter
    #[serde(default)]
    pub connection_required: bool,

    /// Maximum number of connections allowed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<usize>,
}

impl Default for RoutingParameterOptions {
    fn default() -> Self {
        Self {
            connection_label: None,
            connection_description: None,
            connection_required: false,
            max_connections: Some(1),
        }
    }
}

/// Builder for RoutingParameterOptions
#[derive(Debug, Default)]
pub struct RoutingParameterOptionsBuilder {
    connection_label: Option<String>,
    connection_description: Option<String>,
    connection_required: bool,
    max_connections: Option<usize>,
}

impl RoutingParameterOptionsBuilder {
    /// Create a new options builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_connections: Some(1),
            ..Self::default()
        }
    }

    /// Set the connection label
    #[must_use]
    pub fn connection_label(mut self, label: impl Into<String>) -> Self {
        self.connection_label = Some(label.into());
        self
    }

    /// Set the connection description
    #[must_use]
    pub fn connection_description(mut self, description: impl Into<String>) -> Self {
        self.connection_description = Some(description.into());
        self
    }

    /// Set whether connection is required
    #[must_use]
    pub fn connection_required(mut self, required: bool) -> Self {
        self.connection_required = required;
        self
    }

    /// Set the maximum number of connections
    #[must_use]
    pub fn max_connections(mut self, max: usize) -> Self {
        self.max_connections = Some(max);
        self
    }

    /// Allow unlimited connections
    #[must_use]
    pub fn unlimited_connections(mut self) -> Self {
        self.max_connections = None;
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> RoutingParameterOptions {
        RoutingParameterOptions {
            connection_label: self.connection_label,
            connection_description: self.connection_description,
            connection_required: self.connection_required,
            max_connections: self.max_connections,
        }
    }
}

impl RoutingParameterOptions {
    /// Create a new options builder
    #[must_use]
    pub fn builder() -> RoutingParameterOptionsBuilder {
        RoutingParameterOptionsBuilder::new()
    }
}

/// Value for routing parameter containing connection information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingValue {
    /// ID of the connected node/parameter (if any)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_node_id: Option<String>,

    /// Name of the connection (for display purposes)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_name: Option<String>,

    /// Additional metadata about the connection
    #[serde(default)]
    pub connection_metadata: nebula_value::Object,

    /// Timestamp when connection was established
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Default for RoutingValue {
    fn default() -> Self {
        Self::new()
    }
}

impl From<RoutingValue> for nebula_value::Value {
    fn from(routing: RoutingValue) -> Self {
        use crate::ValueRefExt;
        let mut obj = serde_json::Map::new();
        if let Some(node_id) = routing.connected_node_id {
            obj.insert(
                "connected_node_id".to_string(),
                nebula_value::Value::text(node_id).to_json(),
            );
        }
        if let Some(name) = routing.connection_name {
            obj.insert(
                "connection_name".to_string(),
                nebula_value::Value::text(name).to_json(),
            );
        }
        obj.insert(
            "connection_metadata".to_string(),
            nebula_value::Value::Object(routing.connection_metadata).to_json(),
        );
        if let Some(connected_at) = routing.connected_at {
            obj.insert(
                "connected_at".to_string(),
                nebula_value::Value::text(connected_at.to_rfc3339()).to_json(),
            );
        }

        use crate::JsonValueExt;
        serde_json::Value::Object(obj)
            .to_nebula_value()
            .unwrap_or(nebula_value::Value::Null)
    }
}

impl RoutingValue {
    /// Create a new routing value with no connections
    #[must_use]
    pub fn new() -> Self {
        Self {
            connected_node_id: None,
            connection_name: None,
            connection_metadata: nebula_value::Object::new(),
            connected_at: None,
        }
    }

    /// Create a routing value with a connection
    #[must_use]
    pub fn with_connection(node_id: impl Into<String>) -> Self {
        Self {
            connected_node_id: Some(node_id.into()),
            connection_name: None,
            connection_metadata: nebula_value::Object::new(),
            connected_at: Some(chrono::Utc::now()),
        }
    }

    /// Create a routing value with a named connection
    #[must_use]
    pub fn with_named_connection(node_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            connected_node_id: Some(node_id.into()),
            connection_name: Some(name.into()),
            connection_metadata: nebula_value::Object::new(),
            connected_at: Some(chrono::Utc::now()),
        }
    }

    /// Check if this routing value has a connection
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected_node_id.is_some()
    }

    /// Get the connected node ID
    #[must_use]
    pub fn connection_id(&self) -> Option<&String> {
        self.connected_node_id.as_ref()
    }

    /// Set connection metadata
    pub fn set_metadata(&mut self, key: impl Into<String>, value: nebula_value::Value) {
        self.connection_metadata = self.connection_metadata.insert(key.into(), value);
    }

    /// Get connection metadata
    #[must_use]
    pub fn get_metadata(&self, key: &str) -> Option<&nebula_value::Value> {
        self.connection_metadata.get(key)
    }
}

/// Routing parameter - container with connection point functionality.
///
/// Acts as a wrapper around any child parameter with routing/connection capabilities.
#[derive(Serialize)]
pub struct RoutingParameter {
    /// Parameter metadata (flattened for cleaner JSON)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default routing value
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<RoutingValue>,

    /// Child parameter that this routing parameter wraps
    #[serde(skip)]
    pub children: Option<Box<dyn Parameter>>,

    /// Configuration options
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<RoutingParameterOptions>,

    /// Display configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

impl fmt::Debug for RoutingParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoutingParameter")
            .field("metadata", &self.metadata)
            .field("default", &self.default)
            .field("children", &"Option<Box<dyn Parameter>>")
            .field("options", &self.options)
            .field("display", &self.display)
            .field("validation", &self.validation)
            .finish()
    }
}

/// Builder for RoutingParameter
#[derive(Default)]
pub struct RoutingParameterBuilder {
    key: Option<String>,
    name: Option<String>,
    description: Option<String>,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    default: Option<RoutingValue>,
    children: Option<Box<dyn Parameter>>,
    options: Option<RoutingParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl RoutingParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        <Self as Default>::default()
    }

    /// Set the parameter key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the parameter name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the parameter description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set whether the parameter is required
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set the placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set the hint text
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Set the default routing value
    #[must_use]
    pub fn default(mut self, default: RoutingValue) -> Self {
        self.default = Some(default);
        self
    }

    /// Set the child parameter
    #[must_use]
    pub fn child(mut self, child: Box<dyn Parameter>) -> Self {
        self.children = Some(child);
        self
    }

    /// Set the connection label
    #[must_use]
    pub fn connection_label(mut self, label: impl Into<String>) -> Self {
        let options = self
            .options
            .get_or_insert_with(RoutingParameterOptions::default);
        options.connection_label = Some(label.into());
        self
    }

    /// Set whether connection is required
    #[must_use]
    pub fn connection_required(mut self, required: bool) -> Self {
        let options = self
            .options
            .get_or_insert_with(RoutingParameterOptions::default);
        options.connection_required = required;
        self
    }

    /// Set maximum connections
    #[must_use]
    pub fn max_connections(mut self, max: usize) -> Self {
        let options = self
            .options
            .get_or_insert_with(RoutingParameterOptions::default);
        options.max_connections = Some(max);
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: RoutingParameterOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Set the display configuration
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Set the validation rules
    #[must_use]
    pub fn validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    /// Build the RoutingParameter
    ///
    /// # Errors
    ///
    /// Returns an error if required fields are missing or invalid
    pub fn build(self) -> Result<RoutingParameter, ParameterError> {
        let metadata = ParameterMetadata::builder()
            .key(
                self.key
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "key".into(),
                    })?,
            )
            .name(
                self.name
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "name".into(),
                    })?,
            )
            .description(self.description.unwrap_or_default())
            .required(self.required)
            .maybe_placeholder(self.placeholder)
            .maybe_hint(self.hint)
            .build()?;

        Ok(RoutingParameter {
            metadata,
            default: self.default,
            children: self.children,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

impl RoutingParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> RoutingParameterBuilder {
        RoutingParameterBuilder::new()
    }

    /// Get the child parameter
    #[must_use]
    pub fn child(&self) -> Option<&dyn Parameter> {
        self.children.as_deref()
    }

    /// Set the child parameter
    pub fn set_child(&mut self, child: Option<Box<dyn Parameter>>) {
        self.children = child;
    }

    /// Set connection label
    pub fn set_connection_label(&mut self, label: Option<String>) {
        let options = self
            .options
            .get_or_insert_with(RoutingParameterOptions::default);
        options.connection_label = label;
    }

    /// Get connection label
    #[must_use]
    pub fn connection_label(&self) -> Option<&String> {
        self.options.as_ref()?.connection_label.as_ref()
    }

    /// Set whether connection is required
    pub fn set_connection_required(&mut self, required: bool) {
        let options = self
            .options
            .get_or_insert_with(RoutingParameterOptions::default);
        options.connection_required = required;
    }

    /// Check if connection is required
    #[must_use]
    pub fn is_connection_required(&self) -> bool {
        self.options.as_ref().is_some_and(|o| o.connection_required)
    }

    /// Validate a routing value
    #[must_use = "validation result must be checked"]
    pub fn validate_routing(&self, value: &Value) -> Result<(), ParameterError> {
        let is_connected = if let Some(obj) = value.as_object() {
            obj.get("connected_node_id").is_some()
        } else {
            false
        };

        if self.is_connection_required() && !is_connected {
            return Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Connection is required but not configured".to_string(),
            });
        }

        Ok(())
    }
}

impl Describable for RoutingParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Routing
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl Display for RoutingParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RoutingParameter({})", self.metadata.name)
    }
}

impl Validatable for RoutingParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::Object)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        if let Some(obj) = value.as_object() {
            obj.get("connected_node_id").is_none()
        } else {
            true
        }
    }
}

impl Displayable for RoutingParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_routing_parameter_builder() {
        let param = RoutingParameter::builder()
            .key("input_connection")
            .name("Input")
            .description("Input connection point")
            .connection_label("Data In")
            .connection_required(true)
            .max_connections(1)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "input_connection");
        assert_eq!(param.metadata.name, "Input");
        assert!(param.is_connection_required());
        assert_eq!(param.connection_label(), Some(&"Data In".to_string()));
    }

    #[test]
    fn test_routing_parameter_missing_key() {
        let result = RoutingParameter::builder().name("Test").build();

        assert!(result.is_err());
    }

    #[test]
    fn test_routing_value_creation() {
        let value = RoutingValue::new();
        assert!(!value.is_connected());
        assert!(value.connection_id().is_none());

        let connected = RoutingValue::with_connection("node-123");
        assert!(connected.is_connected());
        assert_eq!(connected.connection_id(), Some(&"node-123".to_string()));
    }

    #[test]
    fn test_routing_value_with_name() {
        let value = RoutingValue::with_named_connection("node-456", "Main Input");
        assert!(value.is_connected());
        assert_eq!(value.connected_node_id, Some("node-456".to_string()));
        assert_eq!(value.connection_name, Some("Main Input".to_string()));
    }

    #[test]
    fn test_routing_value_metadata() {
        let mut value = RoutingValue::with_connection("node-789");
        value.set_metadata("priority", nebula_value::Value::integer(1));

        assert!(value.get_metadata("priority").is_some());
    }

    #[test]
    fn test_routing_options_builder() {
        let options = RoutingParameterOptions::builder()
            .connection_label("Output")
            .connection_required(true)
            .max_connections(5)
            .build();

        assert_eq!(options.connection_label, Some("Output".to_string()));
        assert!(options.connection_required);
        assert_eq!(options.max_connections, Some(5));
    }

    #[test]
    fn test_validate_routing_required() {
        let param = RoutingParameter::builder()
            .key("test")
            .name("Test")
            .connection_required(true)
            .build()
            .unwrap();

        let empty_value = nebula_value::Value::Object(nebula_value::Object::new());
        let result = param.validate_routing(&empty_value);
        assert!(result.is_err());

        let connected_obj = nebula_value::Object::new()
            .insert("connected_node_id".to_string(), serde_json::json!("node-1"));
        let connected_value = nebula_value::Value::Object(connected_obj);
        let result = param.validate_routing(&connected_value);
        assert!(result.is_ok());
    }
}
