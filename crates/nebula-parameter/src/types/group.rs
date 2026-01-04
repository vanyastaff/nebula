//! Group parameter type for grouping related data into a structured object

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for grouping related data into a structured object
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = GroupParameter::builder()
///     .key("address")
///     .name("Address")
///     .description("Shipping address")
///     .fields(vec![
///         GroupField::builder()
///             .key("street")
///             .name("Street")
///             .field_type(GroupFieldType::Text)
///             .required(true)
///             .build(),
///         GroupField::builder()
///             .key("city")
///             .name("City")
///             .field_type(GroupFieldType::Text)
///             .required(true)
///             .build(),
///     ])
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<GroupValue>,

    /// Field definitions for this group
    pub fields: Vec<GroupField>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<GroupParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Field definition for a group parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupField {
    /// Field key/name
    pub key: String,

    /// Field display name
    pub name: String,

    /// Field description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Field type (for validation and UI hints)
    pub field_type: GroupFieldType,

    /// Whether this field is required
    #[serde(default)]
    pub required: bool,

    /// Default value for this field
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<nebula_value::Value>,
}

/// Supported field types for group parameters
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum GroupFieldType {
    Text,
    Number,
    Boolean,
    Select { options: Vec<String> },
    Date,
    Email,
    Url,
}

/// Configuration options for a group parameter
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupParameterOptions {}

/// Value container for group parameter
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupValue {
    /// Field values stored as an Object
    pub values: nebula_value::Object,
}

impl From<GroupValue> for nebula_value::Value {
    fn from(group: GroupValue) -> Self {
        nebula_value::Value::Object(group.values)
    }
}

impl GroupValue {
    /// Create a new empty GroupValue
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

    /// Check if the group has any values
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Get all field keys
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.values.keys()
    }
}

impl Default for GroupValue {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// GroupParameter Builder
// =============================================================================

/// Builder for `GroupParameter`
#[derive(Debug, Default)]
pub struct GroupParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<GroupValue>,
    fields: Vec<GroupField>,
    options: Option<GroupParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl GroupParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> GroupParameterBuilder {
        GroupParameterBuilder::new()
    }
}

impl GroupParameterBuilder {
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
            fields: Vec::new(),
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
    pub fn default(mut self, default: GroupValue) -> Self {
        self.default = Some(default);
        self
    }

    /// Set the fields
    #[must_use]
    pub fn fields(mut self, fields: impl IntoIterator<Item = GroupField>) -> Self {
        self.fields = fields.into_iter().collect();
        self
    }

    /// Add a single field
    #[must_use]
    pub fn field(mut self, field: GroupField) -> Self {
        self.fields.push(field);
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: GroupParameterOptions) -> Self {
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

    /// Build the `GroupParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<GroupParameter, ParameterError> {
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

        Ok(GroupParameter {
            metadata,
            default: self.default,
            fields: self.fields,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// GroupField Builder
// =============================================================================

/// Builder for `GroupField`
#[derive(Debug, Default)]
pub struct GroupFieldBuilder {
    key: Option<String>,
    name: Option<String>,
    description: Option<String>,
    field_type: Option<GroupFieldType>,
    required: bool,
    default_value: Option<nebula_value::Value>,
}

impl GroupField {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> GroupFieldBuilder {
        GroupFieldBuilder::default()
    }
}

impl GroupFieldBuilder {
    /// Set the field key (required)
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
        self.description = Some(description.into());
        self
    }

    /// Set the field type (required)
    #[must_use]
    pub fn field_type(mut self, field_type: GroupFieldType) -> Self {
        self.field_type = Some(field_type);
        self
    }

    /// Set whether the field is required
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set the default value
    #[must_use]
    pub fn default_value(mut self, default_value: nebula_value::Value) -> Self {
        self.default_value = Some(default_value);
        self
    }

    /// Build the field
    ///
    /// # Panics
    ///
    /// Panics if required fields (key, name, field_type) are not set.
    #[must_use]
    pub fn build(self) -> GroupField {
        GroupField {
            key: self.key.expect("key is required"),
            name: self.name.expect("name is required"),
            description: self.description,
            field_type: self.field_type.expect("field_type is required"),
            required: self.required,
            default_value: self.default_value,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for GroupParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Group
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for GroupParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GroupParameter({})", self.metadata.name)
    }
}

impl Validatable for GroupParameter {
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

        // Validate each field
        for field in &self.fields {
            if field.required && !obj.contains_key(&field.key) {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("Required field '{}' is missing", field.key),
                });
            }

            // Validate field type if value exists
            if let Some(field_value) = obj.get(&field.key)
                && !self.is_valid_field_value(field, field_value)
            {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("Invalid value for field '{}'", field.key),
                });
            }
        }

        Ok(())
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_object().is_some_and(|obj| obj.is_empty())
    }
}

impl Displayable for GroupParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl GroupParameter {
    /// Validate a single field value against its type
    fn is_valid_field_value(&self, field: &GroupField, value: &Value) -> bool {
        match &field.field_type {
            GroupFieldType::Text => matches!(value, Value::Text(_)),
            GroupFieldType::Number => matches!(value, Value::Float(_) | Value::Integer(_)),
            GroupFieldType::Boolean => matches!(value, Value::Boolean(_)),
            GroupFieldType::Select { options } => {
                if let Value::Text(s) = value {
                    options.contains(&s.to_string())
                } else {
                    false
                }
            }
            GroupFieldType::Date | GroupFieldType::Email | GroupFieldType::Url => {
                matches!(value, Value::Text(_))
            }
        }
    }

    /// Get field definition by key
    #[must_use]
    pub fn get_field(&self, key: &str) -> Option<&GroupField> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// Get all field keys
    pub fn field_keys(&self) -> impl Iterator<Item = &String> {
        self.fields.iter().map(|f| &f.key)
    }

    /// Check if a field is required
    #[must_use]
    pub fn is_field_required(&self, key: &str) -> bool {
        self.get_field(key).is_some_and(|f| f.required)
    }

    /// Get default values for all fields
    #[must_use]
    pub fn get_default_group_value(&self) -> GroupValue {
        let mut group_value = GroupValue::new();

        for field in &self.fields {
            if let Some(default) = &field.default_value {
                group_value.set_field(&field.key, default.clone());
            }
        }

        group_value
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_parameter_builder() {
        let param = GroupParameter::builder()
            .key("address")
            .name("Address")
            .description("Shipping address")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "address");
        assert_eq!(param.metadata.name, "Address");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_group_parameter_with_fields() {
        let param = GroupParameter::builder()
            .key("person")
            .name("Person")
            .fields(vec![
                GroupField::builder()
                    .key("name")
                    .name("Name")
                    .field_type(GroupFieldType::Text)
                    .required(true)
                    .build(),
                GroupField::builder()
                    .key("age")
                    .name("Age")
                    .field_type(GroupFieldType::Number)
                    .build(),
            ])
            .build()
            .unwrap();

        assert_eq!(param.fields.len(), 2);
        assert!(param.is_field_required("name"));
        assert!(!param.is_field_required("age"));
    }

    #[test]
    fn test_group_parameter_missing_key() {
        let result = GroupParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_group_value() {
        let mut group = GroupValue::new();
        assert!(group.is_empty());

        group.set_field("name", nebula_value::Value::text("John"));
        assert!(!group.is_empty());
        assert!(group.get_field("name").is_some());
    }
}
