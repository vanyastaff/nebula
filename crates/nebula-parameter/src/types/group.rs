use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, Parameter, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::Value;

/// Parameter for grouping related data into a structured object
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = GroupParameter::builder()
///     .metadata(ParameterMetadata::new()
///         .key("address")
///         .name("Address")
///         .description("Shipping address")
///         .call()?)
///     .fields([
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
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
#[builder(on(String, into))]
pub struct GroupParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<GroupValue>,

    /// Field definitions for this group
    #[builder(with = FromIterator::from_iter)]
    pub fields: Vec<GroupField>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<GroupParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

/// Field definition for a group parameter
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::{GroupField, GroupFieldType};
///
/// let field = GroupField::builder()
///     .key("email")  // &str -> String via Into
///     .name("Email Address")
///     .description("Your contact email")
///     .field_type(GroupFieldType::Email)
///     .required(true)
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
#[builder(on(String, into))]
pub struct GroupField {
    /// Field key/name
    pub key: String,

    /// Field display name
    pub name: String,

    /// Field description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Field type (for validation and UI hints)
    pub field_type: GroupFieldType,

    /// Whether this field is required
    #[serde(default)]
    #[builder(default)]
    pub required: bool,

    /// Default value for this field
    #[serde(skip_serializing_if = "Option::is_none")]
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
#[derive(Debug, Clone, Serialize, Deserialize, Default, bon::Builder)]
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

impl Parameter for GroupParameter {
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
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        let obj = match value {
            Value::Object(o) => o,
            _ => {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("Expected object value for group, got {}", value.kind()),
                });
            }
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
        match value {
            Value::Object(o) => o.is_empty(),
            _ => true,
        }
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
                // Basic string validation - more specific validation could be added
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
