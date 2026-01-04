//! Object parameter type for structured object containers

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::core::{
    Describable, Displayable, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for structured object data - acts as a container with named child parameters
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = ObjectParameter::builder()
///     .key("config")
///     .name("Configuration")
///     .description("Configuration object")
///     .options(
///         ObjectParameterOptions::builder()
///             .allow_additional_properties(false)
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Serialize)]
pub struct ObjectParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ObjectValue>,

    /// Named child parameters in this object
    #[serde(skip)]
    pub children: HashMap<String, Box<dyn Parameter>>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ObjectParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for object parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
        self.values = self.values.insert(key.into(), value);
    }

    /// Get a field value
    #[must_use]
    pub fn get_field(&self, key: &str) -> Option<&nebula_value::Value> {
        self.values.get(key)
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

// =============================================================================
// ObjectParameter Builder
// =============================================================================

/// Builder for `ObjectParameter`
#[derive(Default)]
pub struct ObjectParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<ObjectValue>,
    children: HashMap<String, Box<dyn Parameter>>,
    options: Option<ObjectParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl ObjectParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> ObjectParameterBuilder {
        ObjectParameterBuilder::new()
    }
}

impl ObjectParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: None,
            name: None,
            description: String::new(),
            required: false,
            placeholder: None,
            hint: None,
            default: None,
            children: HashMap::new(),
            options: None,
            display: None,
            validation: None,
        }
    }

    // -------------------------------------------------------------------------
    // Metadata methods
    // -------------------------------------------------------------------------

    /// Set the parameter key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the display name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set whether the parameter is required
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set hint text
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    // -------------------------------------------------------------------------
    // Parameter-specific methods
    // -------------------------------------------------------------------------

    /// Set the default value
    #[must_use]
    pub fn default(mut self, default: ObjectValue) -> Self {
        self.default = Some(default);
        self
    }

    /// Set child parameters
    #[must_use]
    pub fn children(mut self, children: HashMap<String, Box<dyn Parameter>>) -> Self {
        self.children = children;
        self
    }

    /// Add a child parameter
    #[must_use]
    pub fn child(mut self, key: impl Into<String>, child: Box<dyn Parameter>) -> Self {
        self.children.insert(key.into(), child);
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: ObjectParameterOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Set display conditions
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Set validation rules
    #[must_use]
    pub fn validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    // -------------------------------------------------------------------------
    // Build
    // -------------------------------------------------------------------------

    /// Build the `ObjectParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<ObjectParameter, ParameterError> {
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
            .description(self.description)
            .required(self.required)
            .build()?;

        let mut metadata = metadata;
        metadata.placeholder = self.placeholder;
        metadata.hint = self.hint;

        Ok(ObjectParameter {
            metadata,
            default: self.default,
            children: self.children,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// ObjectParameterOptions Builder
// =============================================================================

/// Builder for `ObjectParameterOptions`
#[derive(Debug, Default)]
pub struct ObjectParameterOptionsBuilder {
    allow_additional_properties: bool,
}

impl ObjectParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> ObjectParameterOptionsBuilder {
        ObjectParameterOptionsBuilder::default()
    }
}

impl ObjectParameterOptionsBuilder {
    /// Set whether to allow additional properties
    #[must_use]
    pub fn allow_additional_properties(mut self, allow: bool) -> Self {
        self.allow_additional_properties = allow;
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> ObjectParameterOptions {
        ObjectParameterOptions {
            allow_additional_properties: self.allow_additional_properties,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

// Manual Debug implementation since we skip trait objects
impl std::fmt::Debug for ObjectParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectParameter")
            .field("metadata", &self.metadata)
            .field("default", &self.default)
            .field(
                "children",
                &format!(
                    "HashMap<String, Box<dyn Parameter>> (len: {})",
                    self.children.len()
                ),
            )
            .field("options", &self.options)
            .field("display", &self.display)
            .field("validation", &self.validation)
            .finish()
    }
}

impl Describable for ObjectParameter {
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

impl Validatable for ObjectParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::Object)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        if let Some(expected) = self.expected_kind() {
            let actual = value.kind();
            if actual != ValueKind::Null && actual != expected {
                return Err(ParameterError::InvalidType {
                    key: self.metadata.key.clone(),
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
                key: self.metadata.key.clone(),
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
                    key: self.metadata.key.clone(),
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
                        key: self.metadata.key.clone(),
                        reason: format!("Additional property '{key}' is not allowed"),
                    });
                }
            }
        }

        Ok(())
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_object().is_some_and(|obj| obj.is_empty())
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
    /// Get child parameter by key
    #[must_use]
    pub fn get_child(&self, key: &str) -> Option<&dyn Parameter> {
        self.children.get(key).map(|b| b.as_ref())
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
        value.insert(key.to_string(), field_value)
    }

    /// Get a field value from an object value
    #[must_use]
    pub fn get_field_value<'a>(
        value: &'a nebula_value::Object,
        key: &str,
    ) -> Option<&'a nebula_value::Value> {
        value.get(key)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_parameter_builder() {
        let param = ObjectParameter::builder()
            .key("config")
            .name("Configuration")
            .description("Configuration object")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "config");
        assert_eq!(param.metadata.name, "Configuration");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_object_parameter_with_options() {
        let param = ObjectParameter::builder()
            .key("settings")
            .name("Settings")
            .options(
                ObjectParameterOptions::builder()
                    .allow_additional_properties(false)
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.options.unwrap();
        assert!(!opts.allow_additional_properties);
    }

    #[test]
    fn test_object_parameter_missing_key() {
        let result = ObjectParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_object_value() {
        let mut obj = ObjectValue::new();
        assert!(obj.is_empty());

        obj.set_field("name", nebula_value::Value::text("test"));
        assert!(!obj.is_empty());
        assert!(obj.contains_field("name"));
        assert_eq!(obj.field_count(), 1);
    }
}
