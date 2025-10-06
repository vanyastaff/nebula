use nebula_core::ParameterKey as Key;
use nebula_validator::{Validator, ValidationContext as ValidatorContext};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Validation configuration for parameters using nebula-validator
#[derive(Default)]
pub struct ParameterValidation {
    /// The validator from nebula-validator (not serialized)
    validator: Option<Box<dyn Validator>>,

    /// Whether the parameter is required (default: true)
    required: bool,

    /// Custom validation message override
    message: Option<String>,
}

impl ParameterValidation {
    /// Create a new empty validation
    pub fn new() -> Self {
        Self::default()
    }

    /// Create validation with a validator
    pub fn with_validator(validator: Box<dyn Validator>) -> Self {
        Self {
            validator: Some(validator),
            required: true,
            message: None,
        }
    }

    /// Set whether the parameter is required
    pub fn set_required(&mut self, required: bool) {
        self.required = required;
    }

    /// Set custom validation message
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = Some(message.into());
    }

    /// Get the custom validation message
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Check if validation is required
    pub fn is_required(&self) -> bool {
        self.required
    }

    /// Check if this validation has a validator
    pub fn has_validator(&self) -> bool {
        self.validator.is_some()
    }

    /// Validate a value (async)
    pub async fn validate(
        &self,
        value: &nebula_value::Value,
        context: Option<&ValidatorContext>,
    ) -> Result<(), ValidationError> {
        // Check if value is required
        if self.required && self.is_empty_value(value) {
            return Err(ValidationError::Required {
                message: self.message.clone(),
            });
        }

        // If value is empty and not required, skip validation
        if !self.required && self.is_empty_value(value) {
            return Ok(());
        }

        // Run the validator if present
        if let Some(validator) = &self.validator {
            validator
                .validate(value, context)
                .await
                .map_err(|invalid| ValidationError::ValidatorFailed {
                    validator: validator.name().to_string(),
                    message: self.message.clone().or_else(|| Some(invalid.to_string())),
                })?;
        }

        Ok(())
    }

    /// Check if a value is considered empty
    #[inline]
    fn is_empty_value(&self, value: &nebula_value::Value) -> bool {
        match value {
            nebula_value::Value::Null => true,
            nebula_value::Value::Text(s) => s.is_empty(),
            nebula_value::Value::Array(a) => a.is_empty(),
            nebula_value::Value::Object(o) => o.is_empty(),
            _ => false,
        }
    }
}

// Manual Clone implementation since Box<dyn Validator> can't be cloned
impl Clone for ParameterValidation {
    fn clone(&self) -> Self {
        Self {
            validator: None, // Can't clone validator
            required: self.required,
            message: self.message.clone(),
        }
    }
}

// Manual Debug implementation
impl std::fmt::Debug for ParameterValidation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParameterValidation")
            .field("validator", &self.validator.as_ref().map(|v| v.name()))
            .field("required", &self.required)
            .field("message", &self.message)
            .finish()
    }
}

// Custom Serialize - only serialize metadata, not the validator
impl Serialize for ParameterValidation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ParameterValidation", 3)?;
        state.serialize_field(
            "validator_name",
            &self.validator.as_ref().map(|v| v.name()),
        )?;
        state.serialize_field("required", &self.required)?;
        state.serialize_field("message", &self.message)?;
        state.end()
    }
}

// Custom Deserialize - restore metadata only, validator will be set separately
impl<'de> Deserialize<'de> for ParameterValidation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            #[serde(default)]
            required: bool,
            message: Option<String>,
        }

        let helper = Helper::deserialize(deserializer)?;
        Ok(Self {
            validator: None,
            required: helper.required,
            message: helper.message,
        })
    }
}

/// Validation error types
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Value is required{}", .message.as_ref().map(|m| format!(": {}", m)).unwrap_or_default())]
    Required { message: Option<String> },

    #[error("Validation failed for '{validator}'{}", .message.as_ref().map(|m| format!(": {}", m)).unwrap_or_default())]
    ValidatorFailed {
        validator: String,
        message: Option<String>,
    },

    #[error("Custom validation error: {message}")]
    Custom { message: String },
}

/// Builder for parameter validation using nebula-validator
pub struct ParameterValidationBuilder {
    validator: Option<Box<dyn Validator>>,
    required: bool,
    message: Option<String>,
}

impl ParameterValidationBuilder {
    /// Create a new validation builder
    pub fn new() -> Self {
        Self {
            validator: None,
            required: true,
            message: None,
        }
    }

    /// Set the validator
    pub fn validator(mut self, validator: Box<dyn Validator>) -> Self {
        self.validator = Some(validator);
        self
    }

    /// Set whether the field is required
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set a custom validation message
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Build the validation configuration
    pub fn build(self) -> ParameterValidation {
        ParameterValidation {
            validator: self.validator,
            required: self.required,
            message: self.message,
        }
    }
}

impl Default for ParameterValidationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenient validation builders using nebula-validator
pub mod validators {
    use super::*;
    use nebula_validator::*;

    /// Email validation
    pub fn email() -> ParameterValidation {
        ParameterValidation::with_validator(Box::new(
            string()
                .and(string_contains("@".to_string()))
                .and(string_contains(".".to_string()))
                .and(min_length(5)),
        ))
    }

    /// URL validation
    pub fn url(require_https: bool) -> ParameterValidation {
        let validator = if require_https {
            Box::new(string().and(string_starts_with("https://".to_string())))
        } else {
            Box::new(
                string().and(
                    string_starts_with("http://".to_string())
                        .or(string_starts_with("https://".to_string())),
                ),
            )
        };
        ParameterValidation::with_validator(validator)
    }

    /// String validation with length constraints
    pub fn string_length(min: Option<usize>, max: Option<usize>) -> ParameterValidation {
        let mut validator: Box<dyn Validator> = Box::new(string());

        if let Some(min_len) = min {
            validator = Box::new(validator.and(min_length(min_len)));
        }

        if let Some(max_len) = max {
            validator = Box::new(validator.and(max_length(max_len)));
        }

        ParameterValidation::with_validator(validator)
    }

    /// Numeric range validation
    pub fn number_range(min: Option<f64>, max: Option<f64>) -> ParameterValidation {
        let mut validator: Box<dyn Validator> = Box::new(number());

        if let Some(min_val) = min {
            validator = Box::new(validator.and(nebula_validator::min(min_val)));
        }

        if let Some(max_val) = max {
            validator = Box::new(validator.and(nebula_validator::max(max_val)));
        }

        ParameterValidation::with_validator(validator)
    }

    /// Required field validation
    pub fn required() -> ParameterValidation {
        ParameterValidation::with_validator(Box::new(nebula_validator::required()))
    }

    /// Optional field validation
    pub fn optional() -> ParameterValidation {
        let mut validation = ParameterValidation::new();
        validation.set_required(false);
        validation
    }

    /// Integer validation
    pub fn integer() -> ParameterValidation {
        ParameterValidation::with_validator(Box::new(nebula_validator::integer()))
    }

    /// Positive number validation
    pub fn positive() -> ParameterValidation {
        ParameterValidation::with_validator(Box::new(nebula_validator::positive()))
    }

    /// Array size validation
    pub fn array_size(min: Option<usize>, max: Option<usize>) -> ParameterValidation {
        let mut validator: Box<dyn Validator> = Box::new(array());

        if let Some(min_size) = min {
            validator = Box::new(validator.and(array_min_size(min_size)));
        }

        if let Some(max_size) = max {
            validator = Box::new(validator.and(array_max_size(max_size)));
        }

        ParameterValidation::with_validator(validator)
    }

    /// One of values validation
    pub fn one_of(values: Vec<nebula_value::Value>) -> ParameterValidation {
        ParameterValidation::with_validator(Box::new(nebula_validator::one_of(values)))
    }

    /// Not in values validation
    pub fn not_in(values: Vec<&str>) -> ParameterValidation {
        ParameterValidation::with_validator(Box::new(not_in_str_values(values)))
    }

    /// Alphanumeric string validation
    pub fn alphanumeric(allow_spaces: bool) -> ParameterValidation {
        ParameterValidation::with_validator(Box::new(
            string().and(nebula_validator::alphanumeric(allow_spaces)),
        ))
    }
}

/// Cross-parameter validation for validating relationships between parameters
pub struct CrossParameterValidation {
    /// Name/description of this validation rule
    name: String,

    /// Parameters involved in this validation
    parameters: Vec<Key>,

    /// Error message when validation fails
    error_message: String,

    /// Validator to run (stored separately, not serialized)
    #[allow(dead_code)]
    validator: Option<Box<dyn Validator>>,
}

impl CrossParameterValidation {
    /// Create a new cross-parameter validation
    pub fn new(name: impl Into<String>, error_message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parameters: Vec::new(),
            error_message: error_message.into(),
            validator: None,
        }
    }

    /// Get the validation rule name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the parameters involved in this validation
    pub fn parameters(&self) -> &[Key] {
        &self.parameters
    }

    /// Get the error message
    pub fn error_message(&self) -> &str {
        &self.error_message
    }

    /// Add a parameter to this validation
    pub fn add_parameter(&mut self, param: Key) {
        if !self.parameters.contains(&param) {
            self.parameters.push(param);
        }
    }

    /// Validate using the validator and context
    pub async fn validate(
        &self,
        values: &HashMap<Key, nebula_value::Value>,
    ) -> Result<(), ValidationError> {
        // Build a validation context from the values
        // Convert HashMap<Key, Value> to Object by mapping keys to strings
        use crate::ValueRefExt;
        let obj_entries: Vec<(String, _)> = values
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_json()))
            .collect();

        let root_value = nebula_value::Value::Object(obj_entries.into_iter().collect());
        let context = ValidatorContext::simple(root_value);

        // Run the validator if present
        if let Some(validator) = &self.validator {
            // We validate against the root object
            let dummy_value = nebula_value::Value::boolean(true);
            validator
                .validate(&dummy_value, Some(&context))
                .await
                .map_err(|_| ValidationError::Custom {
                    message: self.error_message.clone(),
                })?;
        }

        Ok(())
    }
}

// Manual Serialize for CrossParameterValidation
impl Serialize for CrossParameterValidation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("CrossParameterValidation", 3)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("parameters", &self.parameters)?;
        state.serialize_field("error_message", &self.error_message)?;
        state.end()
    }
}

// Manual Deserialize for CrossParameterValidation
impl<'de> Deserialize<'de> for CrossParameterValidation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Helper {
            name: String,
            parameters: Vec<Key>,
            error_message: String,
        }

        let helper = Helper::deserialize(deserializer)?;
        Ok(Self {
            name: helper.name,
            parameters: helper.parameters,
            error_message: helper.error_message,
            validator: None,
        })
    }
}
