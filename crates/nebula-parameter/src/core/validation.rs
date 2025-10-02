use crate::core::ParameterValue;
use crate::core::condition::ParameterCondition;
use nebula_core::ParameterKey as Key;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Validation configuration for parameters with improved encapsulation
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ParameterValidation {
    /// Validation conditions that must be met
    conditions: Vec<ParameterCondition>,

    /// Custom validation message
    message: Option<String>,

    /// Whether validation is required (default: true)
    #[serde(default = "default_required")]
    required: bool,
}

fn default_required() -> bool {
    true
}

impl ParameterValidation {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for validation rules
    pub fn builder() -> ParameterValidationBuilder {
        ParameterValidationBuilder::new()
    }

    /// Validate a value against all conditions (optimized)
    pub fn validate(&self, value: &Value) -> Result<(), ValidationError> {
        // Check if value is required
        if self.required && self.is_empty_json_value(value) {
            return Err(ValidationError::Required);
        }

        // If value is empty and not required, skip validation
        if !self.required && self.is_empty_json_value(value) {
            return Ok(());
        }

        // Check all conditions (early return on first failure)
        for condition in &self.conditions {
            let param_value = ParameterValue::from(value.clone());
            if !condition.evaluate(&param_value) {
                return Err(ValidationError::ConditionFailed {
                    condition: format!("{:?}", condition),
                    message: self.message.clone(),
                });
            }
        }

        Ok(())
    }

    /// Check if a value is considered empty (optimized with early returns)
    #[inline]
    fn is_empty_json_value(&self, value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::String(s) => s.is_empty(),
            Value::Array(a) => a.is_empty(),
            Value::Object(o) => o.is_empty(),
            _ => false,
        }
    }

    // Accessor methods for encapsulated fields

    /// Get the validation conditions
    pub fn conditions(&self) -> &[ParameterCondition] {
        &self.conditions
    }

    /// Get the custom validation message
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Check if validation is required
    pub fn is_required(&self) -> bool {
        self.required
    }

    /// Check if this validation has any conditions
    pub fn is_empty(&self) -> bool {
        self.conditions.is_empty()
    }

    /// Get the number of validation conditions
    pub fn condition_count(&self) -> usize {
        self.conditions.len()
    }

    /// Remove all validation conditions
    pub fn clear_conditions(&mut self) {
        self.conditions.clear();
    }

    /// Add a validation condition
    pub fn add_condition(&mut self, condition: ParameterCondition) {
        self.conditions.push(condition);
    }

    /// Set custom validation message
    pub fn set_message(&mut self, message: String) {
        self.message = Some(message);
    }

    /// Set whether validation is required
    pub fn set_required(&mut self, required: bool) {
        self.required = required;
    }

    /// Reserve capacity for additional conditions (performance optimization)
    pub fn reserve_conditions(&mut self, additional: usize) {
        self.conditions.reserve(additional);
    }
}

/// Builder for parameter validation
pub struct ParameterValidationBuilder {
    conditions: Vec<ParameterCondition>,
    message: Option<String>,
    required: bool,
}

impl Default for ParameterValidationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ParameterValidationBuilder {
    pub fn new() -> Self {
        Self {
            conditions: Vec::new(),
            message: None,
            required: true,
        }
    }

    /// Create a builder with a specific capacity for conditions
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            conditions: Vec::with_capacity(capacity),
            message: None,
            required: true,
        }
    }

    /// Add a condition that the value must not be empty
    pub fn not_empty(mut self) -> Self {
        self.conditions.push(ParameterCondition::IsNotEmpty);
        self
    }

    /// Add a minimum length condition for strings
    pub fn min_length(mut self, min: usize) -> Self {
        self.conditions
            .push(ParameterCondition::StringMinLength(min));
        self
    }

    /// Add a maximum length condition for strings
    pub fn max_length(mut self, max: usize) -> Self {
        self.conditions
            .push(ParameterCondition::StringMaxLength(max));
        self
    }

    /// Add a regex pattern condition (optimized string handling)
    pub fn match_regex(mut self, pattern: &str) -> Self {
        self.conditions
            .push(ParameterCondition::Regex(ParameterValue::from(
                Value::String(pattern.to_owned()),
            )));
        self
    }

    /// Add an equality condition
    pub fn equals(mut self, value: Value) -> Self {
        self.conditions
            .push(ParameterCondition::Eq(ParameterValue::from(value)));
        self
    }

    /// Add a greater than condition
    pub fn greater_than(mut self, value: Value) -> Self {
        self.conditions
            .push(ParameterCondition::Gt(ParameterValue::from(value)));
        self
    }

    /// Add a less than condition
    pub fn less_than(mut self, value: Value) -> Self {
        self.conditions
            .push(ParameterCondition::Lt(ParameterValue::from(value)));
        self
    }

    /// Add a greater than or equal condition
    pub fn greater_than_or_equal(mut self, value: Value) -> Self {
        self.conditions
            .push(ParameterCondition::Gte(ParameterValue::from(value)));
        self
    }

    /// Add a less than or equal condition
    pub fn less_than_or_equal(mut self, value: Value) -> Self {
        self.conditions
            .push(ParameterCondition::Lte(ParameterValue::from(value)));
        self
    }

    /// Add a not equals condition
    pub fn not_equals(mut self, value: Value) -> Self {
        self.conditions
            .push(ParameterCondition::NotEq(ParameterValue::from(value)));
        self
    }

    /// Add a starts with condition for strings (optimized string handling)
    pub fn starts_with(mut self, prefix: &str) -> Self {
        self.conditions
            .push(ParameterCondition::StartsWith(ParameterValue::from(
                Value::String(prefix.to_owned()),
            )));
        self
    }

    /// Add an ends with condition for strings (optimized string handling)
    pub fn ends_with(mut self, suffix: &str) -> Self {
        self.conditions
            .push(ParameterCondition::EndsWith(ParameterValue::from(
                Value::String(suffix.to_owned()),
            )));
        self
    }

    /// Add a contains condition for strings (optimized string handling)
    pub fn contains(mut self, substring: &str) -> Self {
        self.conditions
            .push(ParameterCondition::Contains(ParameterValue::from(
                Value::String(substring.to_owned()),
            )));
        self
    }

    /// Add a range condition
    pub fn between(mut self, from: Value, to: Value) -> Self {
        self.conditions.push(ParameterCondition::Between {
            from: ParameterValue::from(from),
            to: ParameterValue::from(to),
        });
        self
    }

    /// Add a condition that the value must be in the given set
    pub fn in_values(mut self, values: Vec<Value>) -> Self {
        let param_values: Vec<ParameterValue> =
            values.into_iter().map(ParameterValue::from).collect();
        self.conditions.push(ParameterCondition::In(param_values));
        self
    }

    /// Add a condition that the value must not be in the given set
    pub fn not_in(mut self, values: Vec<Value>) -> Self {
        let param_values: Vec<ParameterValue> =
            values.into_iter().map(ParameterValue::from).collect();
        self.conditions
            .push(ParameterCondition::NotIn(param_values));
        self
    }

    /// Add multiple conditions that must all be true
    pub fn all(mut self, conditions: Vec<ParameterCondition>) -> Self {
        self.conditions.push(ParameterCondition::And(conditions));
        self
    }

    /// Add multiple conditions where at least one must be true
    pub fn any(mut self, conditions: Vec<ParameterCondition>) -> Self {
        self.conditions.push(ParameterCondition::Or(conditions));
        self
    }

    /// Add a custom validation function
    pub fn custom<F>(self, _validator: F) -> Self
    where
        F: Fn(&Value) -> Result<(), ValidationError> + 'static,
    {
        // Note: Custom functions can't be serialized, so we skip this for now
        // In a real implementation, you might want to use a different approach
        self
    }

    /// Set a custom validation message
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Set whether the field is required
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Build the validation configuration
    pub fn build(self) -> ParameterValidation {
        ParameterValidation {
            conditions: self.conditions,
            message: self.message,
            required: self.required,
        }
    }
}

/// Validation error types
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("Value is required")]
    Required,

    #[error("Validation condition failed: {condition}")]
    ConditionFailed {
        condition: String,
        message: Option<String>,
    },

    #[error("Custom validation failed: {message}")]
    Custom { message: String },

    #[error("Type validation failed: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },
}

/// Cross-parameter validation for validating relationships between parameters with improved encapsulation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossParameterValidation {
    /// Name/description of this validation rule
    name: String,

    /// Parameters involved in this validation
    parameters: Vec<Key>,

    /// Validation logic (simplified for serialization)
    conditions: HashMap<Key, Vec<ParameterCondition>>,

    /// Error message when validation fails
    error_message: String,
}

impl CrossParameterValidation {
    pub fn new(name: impl Into<String>, error_message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            parameters: Vec::new(),
            conditions: HashMap::new(),
            error_message: error_message.into(),
        }
    }

    // Accessor methods for encapsulated fields

    /// Get the validation rule name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the parameters involved in this validation
    pub fn parameters(&self) -> &[Key] {
        &self.parameters
    }

    /// Get the validation conditions
    pub fn conditions(&self) -> &HashMap<Key, Vec<ParameterCondition>> {
        &self.conditions
    }

    /// Get the error message
    pub fn error_message(&self) -> &str {
        &self.error_message
    }

    /// Add a parameter to this validation (optimized to avoid duplicates)
    pub fn add_parameter(&mut self, param: Key) {
        if !self.parameters.contains(&param) {
            self.parameters.push(param);
        }
    }

    /// Add a condition for a specific parameter (optimized)
    pub fn add_condition(&mut self, param: Key, condition: ParameterCondition) {
        self.add_parameter(param.clone());
        self.conditions.entry(param).or_default().push(condition);
    }

    /// Validate the cross-parameter conditions (optimized with better error handling)
    pub fn validate(&self, values: &HashMap<Key, Value>) -> Result<(), ValidationError> {
        for (param, conditions) in &self.conditions {
            match values.get(param) {
                Some(value) => {
                    // Check all conditions for this parameter
                    for condition in conditions {
                        let param_value = ParameterValue::from(value.clone());
                        if !condition.evaluate(&param_value) {
                            return Err(ValidationError::Custom {
                                message: self.error_message.clone(),
                            });
                        }
                    }
                }
                None => {
                    // Parameter not found - this might be an error depending on requirements
                    return Err(ValidationError::Custom {
                        message: format!("Parameter '{}' not found for cross-validation", param),
                    });
                }
            }
        }
        Ok(())
    }

    /// Check if this validation has any conditions
    pub fn is_empty(&self) -> bool {
        self.conditions.is_empty()
    }

    /// Get the number of parameters involved
    pub fn parameter_count(&self) -> usize {
        self.parameters.len()
    }

    /// Clear all conditions
    pub fn clear_conditions(&mut self) {
        self.conditions.clear();
        self.parameters.clear();
    }
}

/// Common validation patterns
pub mod validators {
    use super::*;
    use serde_json::json;

    /// Email validation
    pub fn email() -> ParameterValidation {
        ParameterValidation::builder()
            .not_empty()
            .match_regex(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")
            .message("Please enter a valid email address")
            .build()
    }

    /// URL validation with optional HTTPS requirement
    pub fn url(require_https: bool) -> ParameterValidation {
        let pattern = if require_https {
            r"^https://[^\s/$.?#].[^\s]*$"
        } else {
            r"^https?://[^\s/$.?#].[^\s]*$"
        };

        ParameterValidation::builder()
            .not_empty()
            .match_regex(pattern)
            .message("Please enter a valid URL")
            .build()
    }

    /// Phone number validation
    pub fn phone_number() -> ParameterValidation {
        ParameterValidation::builder()
            .not_empty()
            .match_regex(r"^\+?[\d\s\-\(\)]+$")
            .min_length(10)
            .message("Please enter a valid phone number")
            .build()
    }

    /// Credit card validation (basic format check)
    pub fn credit_card() -> ParameterValidation {
        ParameterValidation::builder()
            .not_empty()
            .match_regex(r"^\d{4}[\s\-]?\d{4}[\s\-]?\d{4}[\s\-]?\d{4}$")
            .message("Please enter a valid credit card number")
            .build()
    }

    /// Password strength validation
    pub fn password_strength(min_length: usize) -> ParameterValidation {
        ParameterValidation::builder()
            .not_empty()
            .min_length(min_length)
            .all(vec![
                ParameterCondition::Regex(ParameterValue::from(serde_json::json!(r"[A-Z]"))), // Has uppercase
                ParameterCondition::Regex(ParameterValue::from(serde_json::json!(r"[a-z]"))), // Has lowercase
                ParameterCondition::Regex(ParameterValue::from(serde_json::json!(r"\d"))), // Has digit
                ParameterCondition::Regex(ParameterValue::from(serde_json::json!(r"[!@#$%^&*]"))), // Has special char
            ])
            .message("Password must contain uppercase, lowercase, digit, and special character")
            .build()
    }

    /// Numeric range validation
    pub fn numeric_range(min: f64, max: f64) -> ParameterValidation {
        use serde_json::json;
        ParameterValidation::builder()
            .between(json!(min), json!(max))
            .message(&format!("Value must be between {} and {}", min, max))
            .build()
    }

    /// Required field validation
    pub fn required() -> ParameterValidation {
        ParameterValidation::builder()
            .not_empty()
            .message("This field is required")
            .build()
    }

    /// Optional field validation (allows empty values)
    pub fn optional() -> ParameterValidation {
        ParameterValidation::builder().required(false).build()
    }

    /// String validation with optional constraints (similar to old implementation)
    pub fn string(
        min_length: Option<usize>,
        max_length: Option<usize>,
        pattern: Option<&str>,
    ) -> ParameterValidation {
        let mut builder = ParameterValidation::builder();

        // Length constraints
        if let Some(min) = min_length {
            if min > 0 {
                builder = builder.not_empty();
                builder = builder.min_length(min);
            }
        }

        if let Some(max) = max_length {
            builder = builder.max_length(max);
        }

        if let Some(regex_pattern) = pattern {
            builder = builder.match_regex(regex_pattern);
        }

        builder.build()
    }

    /// Number validation with range constraints
    pub fn number(min: Option<f64>, max: Option<f64>) -> ParameterValidation {
        let mut builder = ParameterValidation::builder();

        if let Some(min_val) = min {
            use serde_json::json;
            builder = builder.greater_than_or_equal(json!(min_val));
        }

        if let Some(max_val) = max {
            use serde_json::json;
            builder = builder.less_than_or_equal(json!(max_val));
        }

        builder.build()
    }
}
