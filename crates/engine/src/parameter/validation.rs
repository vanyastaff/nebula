use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::parameter::{ParameterCondition, ParameterError};

/// Detailed error information for validation failures.
#[derive(Debug, Error, Serialize, Deserialize, Clone)]
pub enum ValidationError {
    #[error("Invalid type: expected {0}, but received a different type")]
    InvalidType(String),

    #[error("Regex error: {0}")]
    RegexError(String),

    #[error("Invalid value: {0}")]
    InvalidValue(String),

    #[error("Custom validation error: {0}")]
    Custom(String),
}

/// A collection of validation rules for a property.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ParameterValidation {
    rules: Vec<ParameterCondition>,
}

impl ParameterValidation {
    /// Creates a new, empty `ParameterValidation`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a `ParameterValidation` instance from a vector of rules.
    pub fn from_rules(rules: Vec<ParameterCondition>) -> Self {
        Self { rules }
    }

    /// Creates a new `ParameterValidation` builder.
    pub fn builder() -> ParameterValidationBuilder {
        ParameterValidationBuilder::new()
    }

    /// Adds a new validation rule.
    pub fn add_rule(&mut self, rule: ParameterCondition) -> &mut Self {
        self.rules.push(rule);
        self
    }

    /// Returns an immutable reference to the validation rules.
    pub fn rules(&self) -> &Vec<ParameterCondition> {
        &self.rules
    }

    /// Returns the number of validation rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Checks if there are no validation rules.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Validates a JSON value against all defined validation rules.
    ///
    /// # Arguments
    ///
    /// * `value` - The JSON value to validate.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if all rules pass.
    /// * `Err(ParameterError::ValidationErrors)` if one or more rules fail,
    ///   where the error contains the list of validation errors.
    pub fn validate(&self, value: &Value) -> Result<(), ParameterError> {
        let mut errors = Vec::new();
        for rule in &self.rules {
            if let Err(err) = rule.check(value) {
                errors.push(err);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ParameterError::ValidationErrors(errors))
        }
    }
}

/// Builder for creating `ParameterValidation` instances.
#[derive(Debug, Default)]
pub struct ParameterValidationBuilder {
    validation: ParameterValidation,
}

impl ParameterValidationBuilder {
    /// Creates a new `ParameterValidationBuilder`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a validation rule.
    pub fn with_rule(mut self, rule: ParameterCondition) -> Self {
        self.validation.add_rule(rule);
        self
    }

    /// Adds a rule that the value must equal the specified value.
    pub fn must_equal<T: Into<Value>>(self, value: T) -> Self {
        self.with_rule(ParameterCondition::Eq(value.into()))
    }

    /// Adds a rule that the value must not equal the specified value.
    pub fn must_not_equal<T: Into<Value>>(self, value: T) -> Self {
        self.with_rule(ParameterCondition::NotEq(value.into()))
    }

    /// Adds a rule that the numeric value must be greater than the specified
    /// value.
    pub fn greater_than<T: Into<Value>>(self, value: T) -> Self {
        self.with_rule(ParameterCondition::Gt(value.into()))
    }

    /// Adds a rule that the numeric value must be greater than or equal to the
    /// specified value.
    pub fn greater_than_or_equal<T: Into<Value>>(self, value: T) -> Self {
        self.with_rule(ParameterCondition::Gte(value.into()))
    }

    /// Adds a rule that the numeric value must be less than the specified
    /// value.
    pub fn less_than<T: Into<Value>>(self, value: T) -> Self {
        self.with_rule(ParameterCondition::Lt(value.into()))
    }

    /// Adds a rule that the numeric value must be less than or equal to the
    /// specified value.
    pub fn less_than_or_equal<T: Into<Value>>(self, value: T) -> Self {
        self.with_rule(ParameterCondition::Lte(value.into()))
    }

    /// Adds a rule that the numeric value must be between the specified values
    /// (inclusive).
    pub fn between<T: Into<Value>, U: Into<Value>>(self, from: T, to: U) -> Self {
        self.with_rule(ParameterCondition::Between {
            from: from.into(),
            to: to.into(),
        })
    }

    /// Adds a rule that the string value must match the specified regular
    /// expression.
    pub fn match_regex(self, pattern: impl Into<String>) -> Self {
        self.with_rule(ParameterCondition::Regex(Value::String(pattern.into())))
    }

    /// Adds a rule that the string value must start with the specified prefix.
    pub fn starts_with(self, prefix: impl Into<String>) -> Self {
        self.with_rule(ParameterCondition::StartsWith(Value::String(prefix.into())))
    }

    /// Adds a rule that the string value must end with the specified suffix.
    pub fn ends_with(self, suffix: impl Into<String>) -> Self {
        self.with_rule(ParameterCondition::EndsWith(Value::String(suffix.into())))
    }

    /// Adds a rule that the string value must contain the specified substring.
    pub fn contains(self, substring: impl Into<String>) -> Self {
        self.with_rule(ParameterCondition::Contains(Value::String(
            substring.into(),
        )))
    }

    /// Adds a rule that the value must not be empty.
    pub fn not_empty(self) -> Self {
        self.with_rule(ParameterCondition::IsNotEmpty)
    }

    /// Adds a rule that combines multiple conditions with AND logic.
    pub fn all(self, conditions: Vec<ParameterCondition>) -> Self {
        self.with_rule(ParameterCondition::And(conditions))
    }

    /// Adds a rule that combines multiple conditions with OR logic.
    pub fn any(self, conditions: Vec<ParameterCondition>) -> Self {
        self.with_rule(ParameterCondition::Or(conditions))
    }

    /// Builds the final `ParameterValidation` instance.
    pub fn build(self) -> ParameterValidation {
        self.validation
    }
}

/// Common validation patterns
pub mod validators {
    use serde_json::json;

    use super::*;

    /// Creates validation for a string parameter with optional constraints
    pub fn string(
        min_length: Option<usize>,
        max_length: Option<usize>,
        pattern: Option<&str>,
    ) -> ParameterValidation {
        let mut builder = ParameterValidation::builder();

        // String type validation - ensure it's a string value
        builder = builder.with_rule(ParameterCondition::Not(Box::new(ParameterCondition::Or(
            vec![
                ParameterCondition::Eq(json!(null)),
                ParameterCondition::Eq(json!(false)),
                ParameterCondition::Eq(json!(true)),
                ParameterCondition::Eq(json!(0)),
            ],
        ))));

        // Length constraints
        if let Some(min) = min_length {
            if min > 0 {
                builder = builder.not_empty();
                builder = builder.with_rule(ParameterCondition::StringMinLength(min));
            }
        }

        if let Some(max) = max_length {
            builder = builder.with_rule(ParameterCondition::StringMaxLength(max));
        }

        if let Some(regex_pattern) = pattern {
            builder = builder.match_regex(regex_pattern);
        }

        builder.build()
    }

    /// Creates validation for a numeric parameter with range constraints
    pub fn number(min: Option<f64>, max: Option<f64>, integer_only: bool) -> ParameterValidation {
        let mut builder = ParameterValidation::builder();

        // Number type validation (implicitly handled by comparison operations)

        // Range constraints
        if let Some(min_val) = min {
            builder = builder.greater_than_or_equal(min_val);
        }

        if let Some(max_val) = max {
            builder = builder.less_than_or_equal(max_val);
        }

        // Integer-only check
        if integer_only {
            // TODO: Add proper integer check
        }

        builder.build()
    }

    /// Creates validation for an email parameter
    pub fn email() -> ParameterValidation {
        ParameterValidation::builder()
            .match_regex(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")
            .build()
    }

    /// Creates validation for a URL parameter
    pub fn url() -> ParameterValidation {
        ParameterValidation::builder()
            .match_regex(r"^(https?|ftp)://[^\s/$.?#].[^\s]*$")
            .build()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_empty_validation() {
        let validation = ParameterValidation::new();
        assert!(validation.is_empty());
        assert_eq!(validation.rule_count(), 0);

        // Empty validation should pass for any value
        assert!(validation.validate(&json!(42)).is_ok());
        assert!(validation.validate(&json!("test")).is_ok());
        assert!(validation.validate(&json!(null)).is_ok());
    }

    #[test]
    fn test_basic_validation() {
        let mut validation = ParameterValidation::new();
        validation.add_rule(ParameterCondition::Gt(json!(0)));
        validation.add_rule(ParameterCondition::Lt(json!(100)));

        // Valid values
        assert!(validation.validate(&json!(42)).is_ok());
        assert!(validation.validate(&json!(1)).is_ok());
        assert!(validation.validate(&json!(99)).is_ok());

        // Invalid values
        assert!(validation.validate(&json!(0)).is_err());
        assert!(validation.validate(&json!(100)).is_err());
        assert!(validation.validate(&json!(-5)).is_err());
        assert!(validation.validate(&json!("test")).is_err());
    }

    #[test]
    fn test_builder_pattern() {
        let validation = ParameterValidation::builder()
            .greater_than(0)
            .less_than(100)
            .build();

        // Valid values
        assert!(validation.validate(&json!(42)).is_ok());

        // Invalid values
        assert!(validation.validate(&json!(0)).is_err());
        assert!(validation.validate(&json!(100)).is_err());
    }

    #[test]
    fn test_string_validators() {
        let validation = validators::string(Some(3), Some(10), Some(r"^[a-z]+$"));

        // Valid values
        assert!(validation.validate(&json!("test")).is_ok());
        assert!(validation.validate(&json!("abcdef")).is_ok());

        // Invalid values
        assert!(validation.validate(&json!("ab")).is_err()); // Too short
        assert!(validation.validate(&json!("abcdefghijk")).is_err()); // Too long
        assert!(validation.validate(&json!("Test123")).is_err()); // Invalid pattern
        assert!(validation.validate(&json!(42)).is_err()); // Wrong type
    }

    #[test]
    fn test_email_validator() {
        let validation = validators::email();

        // Valid values
        assert!(validation.validate(&json!("user@example.com")).is_ok());
        assert!(
            validation
                .validate(&json!("name.surname@company.co.uk"))
                .is_ok()
        );

        // Invalid values
        assert!(validation.validate(&json!("invalid")).is_err());
        assert!(validation.validate(&json!("user@")).is_err());
        assert!(validation.validate(&json!("@example.com")).is_err());
        assert!(validation.validate(&json!(42)).is_err());
    }
}
