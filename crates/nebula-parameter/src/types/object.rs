use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::core::{
    Describable, Displayable, Parameter, ParameterBase, ParameterDisplay, ParameterError,
    ParameterKind, ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for structured object data - acts as a container with named child parameters
#[derive(Serialize)]
pub struct ObjectParameter {
    /// Base parameter fields (metadata, display, validation)
    #[serde(flatten)]
    pub base: ParameterBase,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<ObjectValue>,

    /// Named child parameters in this object
    #[serde(skip)]
    pub children: HashMap<String, Box<dyn Parameter>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ObjectParameterOptions>,
}

/// Configuration options for object parameters
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize, Default)]
pub struct ObjectParameterOptions {
    /// Whether to allow additional properties beyond defined children
    #[builder(default)]
    #[serde(default)]
    pub allow_additional_properties: bool,
}

/// Value container for object parameter
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectValue {
    /// Field values as an Object
    pub values: nebula_value::Object,
}

impl From<ObjectValue> for nebula_value::Value {
    fn from(obj: ObjectValue) -> Self {
        nebula_value::Value::Object(obj.values)
    }
}

impl ObjectValue {
    /// Create a new empty `ObjectValue`
    #[must_use]
    pub fn new() -> Self {
        Self {
            values: nebula_value::Object::new(),
        }
    }

    /// Set a field value
    pub fn set_field(&mut self, key: impl Into<String>, value: nebula_value::Value) {
        use crate::ValueRefExt;
        self.values.insert(key.into(), value.to_json());
    }

    /// Get a field value
    #[must_use]
    pub fn get_field(&self, key: &str) -> Option<nebula_value::Value> {
        self.values.get(key).cloned()
    }

    /// Remove a field
    pub fn remove_field(&mut self, key: &str) -> Option<nebula_value::Value> {
        if let Some((new_obj, v)) = self.values.remove(key) {
            self.values = new_obj;
            Some(v)
        } else {
            None
        }
    }

    /// Check if the object has any values
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Get all field keys
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.values.keys()
    }

    /// Get all field values
    pub fn values(&self) -> impl Iterator<Item = &nebula_value::Value> {
        self.values.values()
    }

    /// Check if field exists
    #[must_use]
    pub fn contains_field(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// Get field count
    #[must_use]
    pub fn field_count(&self) -> usize {
        self.values.len()
    }

    /// Get all entries as (key, value) pairs
    pub fn entries(&self) -> impl Iterator<Item = (&String, &nebula_value::Value)> {
        self.values.entries()
    }
}

impl Default for ObjectValue {
    fn default() -> Self {
        Self::new()
    }
}

// Manual Debug implementation since we skip trait objects
impl std::fmt::Debug for ObjectParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectParameter")
            .field("base", &self.base)
            .field("default", &self.default)
            .field(
                "children",
                &format!(
                    "HashMap<String, Box<dyn ParameterType>> (len: {})",
                    self.children.len()
                ),
            )
            .field("options", &self.options)
            .finish()
    }
}

impl Describable for ObjectParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Object
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

impl std::fmt::Display for ObjectParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ObjectParameter({})", self.base.metadata.name)
    }
}

impl Validatable for ObjectParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::Object)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.base.validation.as_ref()
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        if let Some(expected) = self.expected_kind() {
            let actual = value.kind();
            if actual != ValueKind::Null && actual != expected {
                return Err(ParameterError::InvalidType {
                    key: self.base.metadata.key.clone(),
                    expected_type: expected.name().to_string(),
                    actual_details: actual.name().to_string(),
                });
            }
        }

        let obj = match value {
            Value::Object(o) => o,
            Value::Null => return Ok(()), // Null is allowed for optional
            _ => return Ok(()),           // Type error already handled above
        };

        // Required check
        if self.is_empty(value) && self.is_required() {
            return Err(ParameterError::MissingValue {
                key: self.base.metadata.key.clone(),
            });
        }

        // Check for expression values
        if let Some(Value::Text(expr)) = obj.get("_expression")
            && expr.as_str().starts_with("{{")
            && expr.as_str().ends_with("}}")
        {
            return Ok(());
        }

        // Validate that all required child parameters have values
        for (key, child) in &self.children {
            if child.metadata().required && !obj.contains_key(key) {
                return Err(ParameterError::InvalidValue {
                    key: self.base.metadata.key.clone(),
                    reason: format!("Required field '{key}' is missing"),
                });
            }
        }

        // Check for additional properties if not allowed
        if let Some(options) = &self.options
            && !options.allow_additional_properties
        {
            let defined_children: std::collections::HashSet<_> = self.children.keys().collect();

            for key in obj.keys() {
                if !defined_children.contains(key) && key != "_expression" {
                    return Err(ParameterError::InvalidValue {
                        key: self.base.metadata.key.clone(),
                        reason: format!("Additional property '{key}' is not allowed"),
                    });
                }
            }
        }

        Ok(())
    }

    fn is_empty(&self, value: &Value) -> bool {
        match value {
            Value::Object(o) => o.is_empty(),
            _ => true,
        }
    }
}

impl Displayable for ObjectParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
    }
}

impl ObjectParameter {
    /// Create a new object parameter as a container
    pub fn new(
        key: &str,
        name: &str,
        description: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            base: ParameterBase::new(ParameterMetadata {
                key: nebula_core::ParameterKey::new(key)?,
                name: name.to_string(),
                description: description.to_string(),
                required: false,
                placeholder: Some("Configure object fields...".to_string()),
                hint: Some("Object container with child parameters".to_string()),
            }),
            default: None,
            children: HashMap::new(),
            options: Some(ObjectParameterOptions::default()),
        })
    }

    /// Get child parameter by key
    #[must_use]
    pub fn get_child(&self, key: &str) -> Option<&Box<dyn Parameter>> {
        self.children.get(key)
    }

    /// Get mutable child parameter by key
    pub fn get_child_mut(&mut self, key: &str) -> Option<&mut Box<dyn Parameter>> {
        self.children.get_mut(key)
    }

    /// Get all child keys
    pub fn child_keys(&self) -> impl Iterator<Item = &String> {
        self.children.keys()
    }

    /// Get all children as (key, parameter) pairs
    #[must_use]
    pub fn children(&self) -> &HashMap<String, Box<dyn Parameter>> {
        &self.children
    }

    /// Get mutable reference to all children
    pub fn children_mut(&mut self) -> &mut HashMap<String, Box<dyn Parameter>> {
        &mut self.children
    }

    /// Check if a child is required
    #[must_use]
    pub fn is_child_required(&self, key: &str) -> bool {
        self.children
            .get(key)
            .is_some_and(|c| c.metadata().required)
    }

    /// Get default values for all children
    #[must_use]
    pub fn get_default_object_value(&self) -> ObjectValue {
        let mut object_value = ObjectValue::new();

        for (key, child) in &self.children {
            // For container architecture, create default values based on parameter type
            let default_val = match child.kind() {
                ParameterKind::Text => nebula_value::Value::text(""),
                ParameterKind::Number => nebula_value::Value::integer(0),
                ParameterKind::Checkbox => nebula_value::Value::boolean(false),
                ParameterKind::Date => nebula_value::Value::text(""),
                ParameterKind::DateTime => nebula_value::Value::text(""),
                ParameterKind::Time => nebula_value::Value::text(""),
                ParameterKind::Color => nebula_value::Value::text("#000000"),
                ParameterKind::Secret => nebula_value::Value::text(""),
                ParameterKind::Hidden => nebula_value::Value::text(""),
                _ => nebula_value::Value::text(""),
            };
            object_value.set_field(key, default_val);
        }

        object_value
    }

    /// Add a child parameter to the object
    pub fn add_child(&mut self, key: impl Into<String>, child: Box<dyn Parameter>) {
        self.children.insert(key.into(), child);
    }

    /// Remove a child parameter from the object
    pub fn remove_child(&mut self, key: &str) -> Option<Box<dyn Parameter>> {
        self.children.remove(key)
    }

    /// Check if a child exists
    #[must_use]
    pub fn has_child(&self, key: &str) -> bool {
        self.children.contains_key(key)
    }

    /// Get visible children based on display conditions
    pub fn get_visible_children(&self) -> impl Iterator<Item = (&String, &Box<dyn Parameter>)> {
        self.children.iter().filter(|(_key, _child)| {
            // TODO: Implement display condition evaluation based on current values
            // For now, return all children
            true
        })
    }

    /// Get children count
    #[must_use]
    pub fn children_count(&self) -> usize {
        self.children.len()
    }

    /// Get all required children
    pub fn get_required_children(&self) -> impl Iterator<Item = (&String, &Box<dyn Parameter>)> {
        self.children
            .iter()
            .filter(|(_key, child)| child.metadata().required)
    }

    /// Get all optional children
    pub fn get_optional_children(&self) -> impl Iterator<Item = (&String, &Box<dyn Parameter>)> {
        self.children
            .iter()
            .filter(|(_key, child)| !child.metadata().required)
    }

    /// Check if all required children have values in an ObjectValue
    #[must_use]
    pub fn has_all_required_values(value: &Value) -> bool {
        if let Value::Object(obj) = value {
            // This is a simplified check - ideally would check against schema
            !obj.is_empty()
        } else {
            false
        }
    }

    /// Set a field value in an object value
    pub fn set_field_value(
        value: &nebula_value::Object,
        key: &str,
        field_value: nebula_value::Value,
    ) -> nebula_value::Object {
        use crate::ValueRefExt;
        value.insert(key.to_string(), field_value.to_json())
    }

    /// Get a field value from an object value
    #[must_use]
    pub fn get_field_value(value: &nebula_value::Object, key: &str) -> Option<nebula_value::Value> {
        value.get(key).cloned()
    }
}

// Note: Conversion function removed - use nebula_value::ValueRefExt trait instead
// The trait provides .to_json() method for ergonomic conversions
