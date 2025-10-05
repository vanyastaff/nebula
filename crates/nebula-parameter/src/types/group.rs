use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, HasValue, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterType, ParameterValidation, ParameterValue, Validatable,
};

/// Parameter for grouping related data into a structured object
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct GroupParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<GroupValue>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<GroupValue>,

    /// Field definitions for this group
    pub fields: Vec<GroupField>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<GroupParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Field definition for a group parameter
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Builder, Serialize, Deserialize, Default)]
pub struct GroupParameterOptions {}

/// Value container for group parameter
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupValue {
    /// Field values as an Object
    pub values: nebula_value::Object,
}

impl GroupValue {
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

    /// Check if the group has any values
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

impl ParameterType for GroupParameter {
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

impl HasValue for GroupParameter {
    type Value = GroupValue;

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
        self.value.as_ref().map(|group_val| {
            ParameterValue::Value(nebula_value::Value::Object(group_val.values.clone()))
        })
    }

    fn set_parameter_value(
        &mut self,
        value: impl Into<ParameterValue>,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            ParameterValue::Value(nebula_value::Value::Object(obj)) => {
                let mut group_value = GroupValue::new();

                for (key, val) in obj.entries() {
                    // val is already a &serde_json::Value, just clone it
                    group_value.set_field(key.to_string(), val.clone());
                }

                if self.is_valid_group_value(&group_value)? {
                    self.value = Some(group_value);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: "Group value validation failed".to_string(),
                    })
                }
            }
            ParameterValue::Expression(expr) => {
                // For expressions, create a group with a single expression field
                let mut group_value = GroupValue::new();
                group_value.set_field("_expression", nebula_value::Value::text(expr));
                self.value = Some(group_value);
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected object value for group parameter".to_string(),
            }),
        }
    }
}

impl Validatable for GroupParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn value_to_nebula_value(&self, value: &Self::Value) -> nebula_value::Value {
        nebula_value::Value::text(value.clone())
    }

    fn is_empty_value(&self, value: &Self::Value) -> bool {
        value.is_empty()
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
    /// Validate if a group value is valid for this parameter
    fn is_valid_group_value(&self, group_value: &GroupValue) -> Result<bool, ParameterError> {
        // Check for expression values
        if let Some(nebula_value::Value::Text(expr)) = group_value.get_field("_expression") {
            if expr.as_str().starts_with("{{") && expr.as_str().ends_with("}}") {
                return Ok(true); // Allow expressions
            }
        }

        // Validate each field
        for field in &self.fields {
            if field.required {
                if !group_value.values.contains_key(&field.key) {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Required field '{}' is missing", field.key),
                    });
                }
            }

            // Validate field type if value exists
            if let Some(value) = group_value.get_field(&field.key) {
                if !self.is_valid_field_value(field, value) {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Invalid value for field '{}'", field.key),
                    });
                }
            }
        }

        Ok(true)
    }

    /// Validate a single field value against its type
    fn is_valid_field_value(&self, field: &GroupField, value: &serde_json::Value) -> bool {
        match &field.field_type {
            GroupFieldType::Text => value.is_string(),
            GroupFieldType::Number => value.is_number(),
            GroupFieldType::Boolean => value.is_boolean(),
            GroupFieldType::Select { options } => {
                if let Some(s) = value.as_str() {
                    options.contains(&s.to_string())
                } else {
                    false
                }
            }
            GroupFieldType::Date | GroupFieldType::Email | GroupFieldType::Url => {
                // Basic string validation - more specific validation could be added
                value.is_string()
            }
        }
    }

    /// Get field definition by key
    pub fn get_field(&self, key: &str) -> Option<&GroupField> {
        self.fields.iter().find(|f| f.key == key)
    }

    /// Get all field keys
    pub fn field_keys(&self) -> impl Iterator<Item = &String> {
        self.fields.iter().map(|f| &f.key)
    }

    /// Check if a field is required
    pub fn is_field_required(&self, key: &str) -> bool {
        self.get_field(key).map(|f| f.required).unwrap_or(false)
    }

    /// Get default values for all fields
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
