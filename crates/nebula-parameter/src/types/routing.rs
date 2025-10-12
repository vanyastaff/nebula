use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};

use crate::core::traits::Expressible;
use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_core::ParameterKey;
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Routing parameter - container with connection point functionality
/// Acts as a wrapper around any child parameter with routing/connection capabilities
#[derive(Serialize)]
pub struct RoutingParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<RoutingValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<RoutingValue>,

    /// Child parameter that this routing parameter wraps
    #[serde(skip)]
    pub children: Option<Box<dyn Parameter>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<RoutingParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for routing parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingParameterOptions {
    /// Label to display on the connection point
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_label: Option<String>,

    /// Description for the connection point
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_description: Option<String>,

    /// Whether a connection is required for this parameter
    #[serde(default)]
    pub connection_required: bool,

    /// Maximum number of connections allowed
    #[serde(skip_serializing_if = "Option::is_none")]
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

/// Value for routing parameter containing connection information
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingValue {
    /// ID of the connected node/parameter (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_node_id: Option<String>,

    /// Name of the connection (for display purposes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_name: Option<String>,

    /// Additional metadata about the connection
    #[serde(default)]
    pub connection_metadata: nebula_value::Object,

    /// Timestamp when connection was established
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<chrono::DateTime<chrono::Utc>>,
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
    pub fn new() -> Self {
        Self {
            connected_node_id: None,
            connection_name: None,
            connection_metadata: nebula_value::Object::new(),
            connected_at: None,
        }
    }

    /// Create a routing value with a connection
    pub fn with_connection(node_id: impl Into<String>) -> Self {
        Self {
            connected_node_id: Some(node_id.into()),
            connection_name: None,
            connection_metadata: nebula_value::Object::new(),
            connected_at: Some(chrono::Utc::now()),
        }
    }

    /// Create a routing value with a named connection
    pub fn with_named_connection(node_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            connected_node_id: Some(node_id.into()),
            connection_name: Some(name.into()),
            connection_metadata: nebula_value::Object::new(),
            connected_at: Some(chrono::Utc::now()),
        }
    }

    /// Check if this routing value has a connection
    pub fn is_connected(&self) -> bool {
        self.connected_node_id.is_some()
    }

    /// Get the connected node ID
    pub fn connection_id(&self) -> Option<&String> {
        self.connected_node_id.as_ref()
    }

    /// Set connection metadata
    pub fn set_metadata(&mut self, key: impl Into<String>, value: nebula_value::Value) {
        use crate::ValueRefExt;
        self.connection_metadata.insert(key.into(), value.to_json());
    }

    /// Get connection metadata
    pub fn get_metadata(&self, key: &str) -> Option<nebula_value::Value> {
        self.connection_metadata
            .get(key)
            .cloned()
    }
}

impl Default for RoutingValue {
    fn default() -> Self {
        Self::new()
    }
}

// Manual Debug implementation since we skip trait objects
impl fmt::Debug for RoutingParameter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoutingParameter")
            .field("metadata", &self.metadata)
            .field("value", &self.value)
            .field("default", &self.default)
            .field("children", &"Option<Box<dyn ParameterType>>")
            .field("options", &self.options)
            .field("display", &self.display)
            .field("validation", &self.validation)
            .finish()
    }
}

impl Parameter for RoutingParameter {
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

impl HasValue for RoutingParameter {
    type Value = RoutingValue;

    fn get(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear(&mut self) {
        self.value = None;
    }
}

#[async_trait::async_trait]
impl Expressible for RoutingParameter {
    fn to_expression(&self) -> Option<MaybeExpression<Value>> {
        // Routing parameter cannot be serialized as MaybeExpression
        // Return None or convert to a descriptive value
        self.value.as_ref().map(|_| {
            MaybeExpression::Value(nebula_value::Value::Text(nebula_value::Text::from(
                "routing_parameter",
            )))
        })
    }

    fn from_expression(
        &mut self,
        _value: impl Into<MaybeExpression<Value>> + Send,
    ) -> Result<(), ParameterError> {
        // Routing parameters cannot be set via generic ParameterValue
        // They must be set directly using set_value_unchecked
        Err(ParameterError::InvalidValue {
            key: self.metadata.key.clone(),
            reason: "Routing parameters must be set directly, not via ParameterValue".to_string(),
        })
    }
}

impl Validatable for RoutingParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Self::Value) -> bool {
        !value.is_connected()
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

impl RoutingParameter {
    /// Create a new routing parameter as a container
    pub fn new(
        key: &str,
        name: &str,
        description: &str,
        child: Option<Box<dyn Parameter>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            metadata: ParameterMetadata {
                key: ParameterKey::new(key)?,
                name: name.to_string(),
                description: description.to_string(),
                required: false,
                placeholder: Some("Configure routing connection...".to_string()),
                hint: Some("Routing container with connection point".to_string()),
            },
            value: None,
            default: None,
            children: child,
            options: Some(RoutingParameterOptions::default()),
            display: None,
            validation: None,
        })
    }

    /// Get the child parameter
    pub fn child(&self) -> Option<&Box<dyn Parameter>> {
        self.children.as_ref()
    }

    /// Set the child parameter
    pub fn set_child(&mut self, child: Option<Box<dyn Parameter>>) {
        self.children = child;
    }

    /// Connect this parameter to another node
    pub fn connect_to(&mut self, node_id: impl Into<String>) {
        let routing_value = RoutingValue::with_connection(node_id);
        self.value = Some(routing_value);
    }

    /// Connect this parameter to another node with a name
    pub fn connect_to_named(&mut self, node_id: impl Into<String>, name: impl Into<String>) {
        let routing_value = RoutingValue::with_named_connection(node_id, name);
        self.value = Some(routing_value);
    }

    /// Disconnect this parameter
    pub fn disconnect(&mut self) {
        if let Some(value) = &mut self.value {
            value.connected_node_id = None;
            value.connection_name = None;
            value.connected_at = None;
        }
    }

    /// Check if this parameter has a connection
    pub fn is_connected(&self) -> bool {
        self.value
            .as_ref()
            .map(|v| v.is_connected())
            .unwrap_or(false)
    }

    /// Get the connected node ID
    pub fn connected_node_id(&self) -> Option<&String> {
        self.value.as_ref()?.connection_id()
    }

    /// Get the connection name
    pub fn connection_name(&self) -> Option<&String> {
        self.value.as_ref()?.connection_name.as_ref()
    }

    /// Set connection metadata
    pub fn set_connection_metadata(&mut self, key: impl Into<String>, value: nebula_value::Value) {
        if self.value.is_none() {
            self.value = Some(RoutingValue::new());
        }
        if let Some(routing_value) = &mut self.value {
            routing_value.set_metadata(key, value);
        }
    }

    /// Get connection metadata
    pub fn get_connection_metadata(&self, key: &str) -> Option<nebula_value::Value> {
        self.value.as_ref()?.get_metadata(key)
    }

    /// Set connection label
    pub fn set_connection_label(&mut self, label: Option<String>) {
        if self.options.is_none() {
            self.options = Some(RoutingParameterOptions::default());
        }
        if let Some(options) = &mut self.options {
            options.connection_label = label;
        }
    }

    /// Get connection label
    pub fn connection_label(&self) -> Option<&String> {
        self.options.as_ref()?.connection_label.as_ref()
    }

    /// Set whether connection is required
    pub fn set_connection_required(&mut self, required: bool) {
        if self.options.is_none() {
            self.options = Some(RoutingParameterOptions::default());
        }
        if let Some(options) = &mut self.options {
            options.connection_required = required;
        }
    }

    /// Check if connection is required
    pub fn is_connection_required(&self) -> bool {
        self.options
            .as_ref()
            .map(|o| o.connection_required)
            .unwrap_or(false)
    }

    /// Validate the routing parameter
    #[must_use = "validation result must be checked"]
    pub fn validate_routing(&self) -> Result<(), ParameterError> {
        // Check if connection is required but missing
        if self.is_connection_required() && !self.is_connected() {
            return Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Connection is required but not configured".to_string(),
            });
        }

        Ok(())
    }
}
