use bon::Builder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::core::{
    Displayable, HasValue, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterType, ParameterValidation, ParameterValue, Validatable,
};

/// Parameter for structured object data - acts as a container with named child parameters
#[derive(Serialize)]
pub struct ObjectParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<ObjectValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<ObjectValue>,

    /// Named child parameters in this object
    #[serde(skip)]
    pub children: HashMap<String, Box<dyn ParameterType>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ObjectParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for object parameters
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct ObjectParameterOptions {
    /// Whether to allow additional properties beyond defined children
    #[serde(default)]
    pub allow_additional_properties: bool,
}

/// Value container for object parameter
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectValue {
    /// Field values as an Object
    pub values: nebula_value::Object,
}

impl Default for ObjectParameterOptions {
    fn default() -> Self {
        Self {
            allow_additional_properties: false,
        }
    }
}

impl ObjectValue {
    /// Create a new empty ObjectValue
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
    pub fn get_field(&self, key: &str) -> Option<nebula_value::Value> {
        use crate::JsonValueExt;
        self.values.get(key).and_then(|v| v.to_nebula_value())
    }

    /// Remove a field
    pub fn remove_field(&mut self, key: &str) -> Option<nebula_value::Value> {
        use crate::JsonValueExt;
        if let Some((new_obj, v)) = self.values.remove(key) {
            self.values = new_obj;
            v.to_nebula_value()
        } else {
            None
        }
    }

    /// Check if the object has any values
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Get all field keys
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.values.keys()
    }

    /// Get all field values
    pub fn values(&self) -> impl Iterator<Item = &serde_json::Value> {
        self.values.values()
    }

    /// Check if field exists
    pub fn contains_field(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// Get field count
    pub fn field_count(&self) -> usize {
        self.values.len()
    }

    /// Get all entries as (key, value) pairs
    pub fn entries(&self) -> impl Iterator<Item = (&String, &serde_json::Value)> {
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
            .field("metadata", &self.metadata)
            .field("value", &self.value)
            .field("default", &self.default)
            .field(
                "children",
                &format!(
                    "HashMap<String, Box<dyn ParameterType>> (len: {})",
                    self.children.len()
                ),
            )
            .field("options", &self.options)
            .field("display", &self.display)
            .field("validation", &self.validation)
            .finish()
    }
}

impl ParameterType for ObjectParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Object
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for ObjectParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ObjectParameter({})", self.metadata.name)
    }
}

impl HasValue for ObjectParameter {
    type Value = ObjectValue;

    fn get_value(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_value_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set_value_unchecked(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default_value(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear_value(&mut self) {
        self.value = None;
    }

    fn get_parameter_value(&self) -> Option<ParameterValue> {
        self.value
            .as_ref()
            .map(|obj_val| ParameterValue::Value(nebula_value::Value::Object(obj_val.values.clone())))
    }

    fn set_parameter_value(
        &mut self,
        value: impl Into<ParameterValue>,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            ParameterValue::Value(nebula_value::Value::Object(obj)) => {
                let object_value = ObjectValue { values: obj };

                if self.is_valid_object_value(&object_value)? {
                    self.value = Some(object_value);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: "Object value validation failed".to_string(),
                    })
                }
            }
            ParameterValue::Expression(expr) => {
                // For expressions, create an object with a single expression field
                let mut object_value = ObjectValue::new();
                object_value.set_field("_expression", nebula_value::Value::text(expr));
                self.value = Some(object_value);
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected object value for object parameter".to_string(),
            }),
        }
    }
}

impl Validatable for ObjectParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn value_to_nebula_value(&self, value: &Self::Value) -> nebula_value::Value {
        nebula_value::Value::Object(value.values.clone())
    }

    fn is_empty_value(&self, value: &Self::Value) -> bool {
        value.is_empty()
    }
}

impl Displayable for ObjectParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
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
            metadata: ParameterMetadata {
                key: nebula_core::ParameterKey::new(key)?,
                name: name.to_string(),
                description: description.to_string(),
                required: false,
                placeholder: Some("Configure object fields...".to_string()),
                hint: Some("Object container with child parameters".to_string()),
            },
            value: None,
            default: None,
            children: HashMap::new(),
            options: Some(ObjectParameterOptions::default()),
            display: None,
            validation: None,
        })
    }

    /// Validate if an object value is valid for this parameter
    fn is_valid_object_value(&self, object_value: &ObjectValue) -> Result<bool, ParameterError> {
        // Check for expression values
        if let Some(nebula_value::Value::Text(expr)) = object_value.get_field("_expression") {
            if expr.as_str().starts_with("{{") && expr.as_str().ends_with("}}") {
                return Ok(true); // Allow expressions
            }
        }

        // For container architecture, validate that all required child parameters have values
        for (key, child) in &self.children {
            if child.metadata().required {
                if !object_value.contains_field(key) {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Required field '{}' is missing", key),
                    });
                }
            }
        }

        // Check for additional properties if not allowed
        if let Some(options) = &self.options {
            if !options.allow_additional_properties {
                let defined_children: std::collections::HashSet<_> = self.children.keys().collect();

                for key in object_value.keys() {
                    if !defined_children.contains(key) && key != "_expression" {
                        return Err(ParameterError::InvalidValue {
                            key: self.metadata.key.clone(),
                            reason: format!("Additional property '{}' is not allowed", key),
                        });
                    }
                }
            }
        }

        Ok(true)
    }

    /// Get child parameter by key
    pub fn get_child(&self, key: &str) -> Option<&Box<dyn ParameterType>> {
        self.children.get(key)
    }

    /// Get mutable child parameter by key
    pub fn get_child_mut(&mut self, key: &str) -> Option<&mut Box<dyn ParameterType>> {
        self.children.get_mut(key)
    }

    /// Get all child keys
    pub fn child_keys(&self) -> impl Iterator<Item = &String> {
        self.children.keys()
    }

    /// Get all children as (key, parameter) pairs
    pub fn children(&self) -> &HashMap<String, Box<dyn ParameterType>> {
        &self.children
    }

    /// Get mutable reference to all children
    pub fn children_mut(&mut self) -> &mut HashMap<String, Box<dyn ParameterType>> {
        &mut self.children
    }

    /// Check if a child is required
    pub fn is_child_required(&self, key: &str) -> bool {
        self.children
            .get(key)
            .map(|c| c.metadata().required)
            .unwrap_or(false)
    }

    /// Get default values for all children
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
    pub fn add_child(&mut self, key: impl Into<String>, child: Box<dyn ParameterType>) {
        self.children.insert(key.into(), child);
    }

    /// Remove a child parameter from the object
    pub fn remove_child(&mut self, key: &str) -> Option<Box<dyn ParameterType>> {
        self.children.remove(key)
    }

    /// Check if a child exists
    pub fn has_child(&self, key: &str) -> bool {
        self.children.contains_key(key)
    }

    /// Get visible children based on display conditions
    pub fn get_visible_children(&self) -> impl Iterator<Item = (&String, &Box<dyn ParameterType>)> {
        self.children.iter().filter(|(_key, _child)| {
            // TODO: Implement display condition evaluation based on current values
            // For now, return all children
            true
        })
    }

    /// Get children count
    pub fn children_count(&self) -> usize {
        self.children.len()
    }

    /// Get all required children
    pub fn get_required_children(
        &self,
    ) -> impl Iterator<Item = (&String, &Box<dyn ParameterType>)> {
        self.children
            .iter()
            .filter(|(_key, child)| child.metadata().required)
    }

    /// Get all optional children
    pub fn get_optional_children(
        &self,
    ) -> impl Iterator<Item = (&String, &Box<dyn ParameterType>)> {
        self.children
            .iter()
            .filter(|(_key, child)| !child.metadata().required)
    }

    /// Check if all required children have values in the current ObjectValue
    pub fn has_all_required_values(&self) -> bool {
        if let Some(value) = &self.value {
            self.get_required_children()
                .all(|(key, _child)| value.contains_field(key))
        } else {
            self.get_required_children().count() == 0
        }
    }

    /// Set a field value in the object
    pub fn set_field_value(
        &mut self,
        key: &str,
        value: nebula_value::Value,
    ) -> Result<(), ParameterError> {
        if !self.has_child(key)
            && !self
                .options
                .as_ref()
                .map(|o| o.allow_additional_properties)
                .unwrap_or(false)
        {
            return Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: format!("Field '{}' is not defined in this object", key),
            });
        }

        if let Some(obj_value) = &mut self.value {
            obj_value.set_field(key, value);
        } else {
            let mut obj_value = ObjectValue::new();
            obj_value.set_field(key, value);
            self.value = Some(obj_value);
        }

        Ok(())
    }

    /// Get a field value from the object
    pub fn get_field_value(&self, key: &str) -> Option<nebula_value::Value> {
        self.value.as_ref().and_then(|obj| obj.get_field(key))
    }
}

// Note: Conversion function removed - use nebula_value::ValueRefExt trait instead
// The trait provides .to_json() method for ergonomic conversions
