//! Parameter validation using nebula-validator
//!
//! This module provides an ergonomic API for parameter validation that wraps
//! the powerful `nebula-validator` crate with parameter-specific conveniences.
//!
//! # Architecture
//!
//! - `ValueCondition` - Core condition type for evaluating field values
//! - `ParameterValidation` - Configuration holding validators
//! - Fluent builder API for common validation patterns
//! - Integration with nebula-validator's `TypedValidator` trait
//!
//! # Examples
//!
//! ```rust
//! use nebula_parameter::prelude::*;
//!
//! // Simple validators
//! let validation = ParameterValidation::string()
//!     .min_length(3)
//!     .max_length(50)
//!     .build();
//!
//! // Email validation
//! let email_validation = ParameterValidation::email();
//!
//! // Number range
//! let age_validation = ParameterValidation::number()
//!     .min(18.0)
//!     .max(120.0)
//!     .build();
//! ```

use nebula_core::ParameterKey;
use nebula_validator::core::{AsyncValidator, TypedValidator, ValidationContext, ValidationError};
use nebula_validator::validators::prelude::*;
use nebula_value::Value;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

// =============================================================================
// ValueCondition - Core condition type
// =============================================================================

/// Condition to evaluate against a field value.
///
/// `ValueCondition` is the core building block for conditional logic in
/// validation rules and display visibility. It evaluates a single value
/// and returns `true` if the condition is met.
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::core::ValueCondition;
/// use nebula_value::Value;
///
/// // Value equality
/// let cond = ValueCondition::Equals(Value::text("api_key"));
/// assert!(cond.evaluate(&Value::text("api_key")));
/// assert!(!cond.evaluate(&Value::text("oauth")));
///
/// // Presence check
/// let cond = ValueCondition::IsSet;
/// assert!(cond.evaluate(&Value::integer(42)));
/// assert!(!cond.evaluate(&Value::Null));
///
/// // Numeric comparison
/// let cond = ValueCondition::GreaterThan(18.0);
/// assert!(cond.evaluate(&Value::integer(21)));
/// assert!(!cond.evaluate(&Value::integer(15)));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueCondition {
    // === Value comparisons ===
    /// Value equals the specified value.
    Equals(Value),

    /// Value does not equal the specified value.
    NotEquals(Value),

    /// Value is one of the specified values.
    OneOf(Vec<Value>),

    // === Presence checks ===
    /// Value is not null (is set).
    IsSet,

    /// Value is null.
    IsNull,

    /// Value is empty (null, empty string, empty array/object).
    IsEmpty,

    /// Value is not empty.
    IsNotEmpty,

    // === Validation state ===
    /// Field has passed validation.
    IsValid,

    /// Field has failed validation.
    IsInvalid,

    // === Numeric comparisons ===
    /// Numeric value is greater than threshold.
    GreaterThan(f64),

    /// Numeric value is greater than or equal to threshold.
    GreaterOrEqual(f64),

    /// Numeric value is less than threshold.
    LessThan(f64),

    /// Numeric value is less than or equal to threshold.
    LessOrEqual(f64),

    /// Numeric value is within range (inclusive).
    InRange {
        /// Minimum value (inclusive).
        min: f64,
        /// Maximum value (inclusive).
        max: f64,
    },

    // === String operations ===
    /// String contains substring.
    Contains(String),

    /// String starts with prefix.
    StartsWith(String),

    /// String ends with suffix.
    EndsWith(String),

    /// String matches regex pattern.
    Matches(String),

    // === Boolean ===
    /// Boolean value is true.
    IsTrue,

    /// Boolean value is false.
    IsFalse,

    // === Combinators ===
    /// All conditions must be true (AND).
    And(Vec<ValueCondition>),

    /// At least one condition must be true (OR).
    Or(Vec<ValueCondition>),

    /// Condition must be false (NOT).
    Not(Box<ValueCondition>),

    // === Cross-field ===
    /// Evaluate condition on another field's value.
    ///
    /// Example: `Field("other_field", IsSet)` - check if other field is set
    Field(ParameterKey, Box<ValueCondition>),

    /// Current value must equal the referenced field's value.
    ///
    /// Example: `EqualsField("password")` - confirm_password equals password
    EqualsField(ParameterKey),

    /// Current value must not equal the referenced field's value.
    NotEqualsField(ParameterKey),
}

impl ValueCondition {
    /// Evaluate the condition against a value.
    ///
    /// Returns `true` if the condition is met, `false` otherwise.
    ///
    /// Note: `IsValid` and `IsInvalid` require validation context and
    /// always return `false` when evaluated directly against a value.
    #[must_use]
    pub fn evaluate(&self, value: &Value) -> bool {
        match self {
            Self::Equals(expected) => value == expected,
            Self::NotEquals(expected) => value != expected,
            Self::OneOf(values) => values.iter().any(|v| value == v),

            Self::IsSet => !value.is_null(),
            Self::IsNull => value.is_null(),
            Self::IsEmpty => Self::is_value_empty(value),
            Self::IsNotEmpty => !Self::is_value_empty(value),

            // Validation state requires context
            Self::IsValid | Self::IsInvalid => false,

            Self::GreaterThan(threshold) => {
                Self::get_numeric(value).is_some_and(|n| n > *threshold)
            }
            Self::GreaterOrEqual(threshold) => {
                Self::get_numeric(value).is_some_and(|n| n >= *threshold)
            }
            Self::LessThan(threshold) => Self::get_numeric(value).is_some_and(|n| n < *threshold),
            Self::LessOrEqual(threshold) => {
                Self::get_numeric(value).is_some_and(|n| n <= *threshold)
            }
            Self::InRange { min, max } => {
                Self::get_numeric(value).is_some_and(|n| n >= *min && n <= *max)
            }

            Self::Contains(substring) => {
                Self::get_string(value).is_some_and(|s| s.contains(substring))
            }
            Self::StartsWith(prefix) => {
                Self::get_string(value).is_some_and(|s| s.starts_with(prefix))
            }
            Self::EndsWith(suffix) => Self::get_string(value).is_some_and(|s| s.ends_with(suffix)),
            Self::Matches(pattern) => Self::get_string(value).is_some_and(|s| {
                regex::Regex::new(pattern)
                    .map(|re| re.is_match(s))
                    .unwrap_or(false)
            }),

            Self::IsTrue => value.as_boolean() == Some(true),
            Self::IsFalse => value.as_boolean() == Some(false),

            // Combinators
            Self::And(conditions) => conditions.iter().all(|c| c.evaluate(value)),
            Self::Or(conditions) => conditions.iter().any(|c| c.evaluate(value)),
            Self::Not(condition) => !condition.evaluate(value),

            // Cross-field requires context, return false here
            Self::Field(_, _) | Self::EqualsField(_) | Self::NotEqualsField(_) => false,
        }
    }

    /// Check if this condition requires context (validation state or other fields).
    #[must_use]
    pub fn requires_context(&self) -> bool {
        match self {
            Self::IsValid | Self::IsInvalid => true,
            Self::Field(_, _) | Self::EqualsField(_) | Self::NotEqualsField(_) => true,
            Self::And(conditions) | Self::Or(conditions) => {
                conditions.iter().any(|c| c.requires_context())
            }
            Self::Not(condition) => condition.requires_context(),
            _ => false,
        }
    }

    /// Collect all referenced fields from this condition and nested conditions.
    pub fn collect_referenced_fields(&self, fields: &mut Vec<ParameterKey>) {
        match self {
            Self::Field(key, condition) => {
                fields.push(key.clone());
                condition.collect_referenced_fields(fields);
            }
            Self::EqualsField(key) | Self::NotEqualsField(key) => {
                fields.push(key.clone());
            }
            Self::And(conditions) | Self::Or(conditions) => {
                for c in conditions {
                    c.collect_referenced_fields(fields);
                }
            }
            Self::Not(condition) => condition.collect_referenced_fields(fields),
            _ => {}
        }
    }

    /// Check if a value is empty.
    #[must_use]
    pub fn is_value_empty(value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::Text(t) => t.as_str().is_empty(),
            Value::Array(arr) => arr.is_empty(),
            Value::Object(obj) => obj.is_empty(),
            _ => false,
        }
    }

    /// Extract a numeric value as f64.
    #[must_use]
    pub fn get_numeric(value: &Value) -> Option<f64> {
        value.as_float_lossy().map(|f| f.value())
    }

    /// Extract a string value.
    #[must_use]
    pub fn get_string(value: &Value) -> Option<&str> {
        value.as_str()
    }

    // === Convenience constructors ===

    /// Create AND condition.
    pub fn and(conditions: impl IntoIterator<Item = ValueCondition>) -> Self {
        Self::And(conditions.into_iter().collect())
    }

    /// Create OR condition.
    pub fn or(conditions: impl IntoIterator<Item = ValueCondition>) -> Self {
        Self::Or(conditions.into_iter().collect())
    }

    /// Create NOT condition.
    pub fn not(condition: ValueCondition) -> Self {
        Self::Not(Box::new(condition))
    }

    /// Create cross-field condition: check if other field's value matches condition.
    pub fn field(key: impl Into<ParameterKey>, condition: ValueCondition) -> Self {
        Self::Field(key.into(), Box::new(condition))
    }

    /// Create cross-field equals: current value must equal other field's value.
    pub fn equals_field(key: impl Into<ParameterKey>) -> Self {
        Self::EqualsField(key.into())
    }

    /// Create cross-field not-equals: current value must not equal other field's value.
    pub fn not_equals_field(key: impl Into<ParameterKey>) -> Self {
        Self::NotEqualsField(key.into())
    }
}

/// Alias for validation context usage.
pub type FieldCondition = ValueCondition;

// =============================================================================
// ParameterValidation
// =============================================================================

/// Validation configuration for parameters
///
/// This wraps validators from `nebula-validator` and provides parameter-specific
/// conveniences like required field checking and custom error messages.
///
/// Note: The validator itself is not serialized, only the configuration (required, message, key).
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ParameterValidation {
    /// The underlying validator (type-erased for storage)
    /// Not serialized - validators must be reconstructed when deserializing
    #[serde(skip)]
    validator: Option<
        Arc<dyn AsyncValidator<Input = Value, Output = (), Error = ValidationError> + Send + Sync>,
    >,

    /// Whether the parameter is required (checked before validator)
    required: bool,

    /// Custom validation message override
    message: Option<String>,

    /// Parameter key (for error context)
    key: Option<ParameterKey>,
}

impl std::fmt::Debug for ParameterValidation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParameterValidation")
            .field("has_validator", &self.validator.is_some())
            .field("required", &self.required)
            .field("message", &self.message)
            .field("key", &self.key)
            .finish()
    }
}

impl ParameterValidation {
    /// Create a new empty validation
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create validation with a typed validator
    pub fn with_validator<V>(validator: V) -> Self
    where
        V: AsyncValidator<Input = Value, Output = (), Error = ValidationError>
            + Send
            + Sync
            + 'static,
    {
        Self {
            validator: Some(Arc::new(validator)),
            required: false,
            message: None,
            key: None,
        }
    }

    /// Set whether the parameter is required
    #[must_use = "builder methods must be chained or built"]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Set whether the parameter is optional
    #[must_use = "builder methods must be chained or built"]
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    /// Set custom validation message
    #[must_use = "builder methods must be chained or built"]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Set parameter key for error context
    #[must_use = "builder methods must be chained or built"]
    pub fn with_key(mut self, key: ParameterKey) -> Self {
        self.key = Some(key);
        self
    }

    /// Get the custom validation message
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Check if validation is required
    #[must_use]
    pub fn is_required(&self) -> bool {
        self.required
    }

    /// Validate a value
    pub async fn validate(
        &self,
        value: &Value,
        _context: Option<&ValidationContext>,
    ) -> Result<(), ValidationError> {
        // Check required first
        if self.required && value.is_null() {
            let mut err = ValidationError::new(
                "required",
                self.message.as_deref().unwrap_or("This field is required"),
            );

            if let Some(key) = &self.key {
                err = err.with_field(key.as_str());
            }

            return Err(err);
        }

        // If no value and not required, skip validation
        if value.is_null() {
            return Ok(());
        }

        // Run validator if present
        if let Some(validator) = &self.validator {
            let result = validator.validate_async(value).await;

            // Apply custom message and field if validation failed
            if let Err(mut err) = result {
                if let Some(msg) = &self.message {
                    // Create new error with custom message using the error code field
                    err = ValidationError::new(&err.code, msg);
                }
                if let Some(key) = &self.key {
                    err = err.with_field(key.as_str());
                }
                return Err(err);
            }
        }

        Ok(())
    }
}

// =============================================================================
// Fluent Builder API
// =============================================================================

/// Builder for string validation
pub struct StringValidationBuilder {
    min_len: Option<usize>,
    max_len: Option<usize>,
    pattern: Option<String>,
    contains_str: Option<String>,
    starts_with_str: Option<String>,
    ends_with_str: Option<String>,
    is_email: bool,
    is_url: bool,
    required: bool,
    message: Option<String>,
}

impl StringValidationBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            min_len: None,
            max_len: None,
            pattern: None,
            contains_str: None,
            starts_with_str: None,
            ends_with_str: None,
            is_email: false,
            is_url: false,
            required: false,
            message: None,
        }
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn min_length(mut self, min: usize) -> Self {
        self.min_len = Some(min);
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn max_length(mut self, max: usize) -> Self {
        self.max_len = Some(max);
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn pattern(mut self, pattern: impl Into<String>) -> Self {
        self.pattern = Some(pattern.into());
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn contains(mut self, s: impl Into<String>) -> Self {
        self.contains_str = Some(s.into());
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn starts_with(mut self, s: impl Into<String>) -> Self {
        self.starts_with_str = Some(s.into());
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn ends_with(mut self, s: impl Into<String>) -> Self {
        self.ends_with_str = Some(s.into());
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn email(mut self) -> Self {
        self.is_email = true;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn url(mut self) -> Self {
        self.is_url = true;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn message(mut self, msg: impl Into<String>) -> Self {
        self.message = Some(msg.into());
        self
    }

    #[must_use]
    pub fn build(self) -> ParameterValidation {
        // Build composite validator
        let mut validators: Vec<
            Box<
                dyn TypedValidator<Input = str, Output = (), Error = ValidationError> + Send + Sync,
            >,
        > = Vec::new();

        if let Some(min) = self.min_len {
            validators.push(Box::new(min_length(min)));
        }

        if let Some(max) = self.max_len {
            validators.push(Box::new(max_length(max)));
        }

        if self.is_email {
            validators.push(Box::new(email()));
        }

        if self.is_url {
            validators.push(Box::new(url()));
        }

        if let Some(pattern) = self.pattern {
            // matches_regex returns Result, need to unwrap or handle
            if let Ok(validator) = matches_regex(&pattern) {
                validators.push(Box::new(validator));
            }
        }

        if let Some(s) = self.contains_str {
            validators.push(Box::new(contains(s)));
        }

        if let Some(s) = self.starts_with_str {
            validators.push(Box::new(starts_with(s)));
        }

        if let Some(s) = self.ends_with_str {
            validators.push(Box::new(ends_with(s)));
        }

        // Combine all validators with AND logic
        let validator = if validators.is_empty() {
            None
        } else {
            // Create a composite validator that checks all conditions
            Some(Arc::new(StringCompositeValidator { validators })
                as Arc<
                    dyn AsyncValidator<Input = Value, Output = (), Error = ValidationError>
                        + Send
                        + Sync,
                >)
        };

        ParameterValidation {
            validator,
            required: self.required,
            message: self.message,
            key: None,
        }
    }
}

impl Default for StringValidationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for number validation
pub struct NumberValidationBuilder {
    min_val: Option<f64>,
    max_val: Option<f64>,
    must_be_positive: bool,
    must_be_negative: bool,
    must_be_even: bool,
    must_be_odd: bool,
    required: bool,
    message: Option<String>,
}

impl NumberValidationBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            min_val: None,
            max_val: None,
            must_be_positive: false,
            must_be_negative: false,
            must_be_even: false,
            must_be_odd: false,
            required: false,
            message: None,
        }
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn min(mut self, min: f64) -> Self {
        self.min_val = Some(min);
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn max(mut self, max: f64) -> Self {
        self.max_val = Some(max);
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn positive(mut self) -> Self {
        self.must_be_positive = true;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn negative(mut self) -> Self {
        self.must_be_negative = true;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn even(mut self) -> Self {
        self.must_be_even = true;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn odd(mut self) -> Self {
        self.must_be_odd = true;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn message(mut self, msg: impl Into<String>) -> Self {
        self.message = Some(msg.into());
        self
    }

    #[must_use]
    pub fn build(self) -> ParameterValidation {
        let mut validators: Vec<
            Box<
                dyn TypedValidator<Input = f64, Output = (), Error = ValidationError> + Send + Sync,
            >,
        > = Vec::new();

        if let Some(min_value) = self.min_val {
            validators.push(Box::new(nebula_validator::validators::numeric::min(
                min_value,
            )));
        }

        if let Some(max_value) = self.max_val {
            validators.push(Box::new(nebula_validator::validators::numeric::max(
                max_value,
            )));
        }

        if self.must_be_positive {
            validators.push(Box::new(positive()));
        }

        if self.must_be_negative {
            validators.push(Box::new(negative()));
        }

        if self.must_be_even {
            validators.push(Box::new(even()));
        }

        if self.must_be_odd {
            validators.push(Box::new(odd()));
        }

        let validator = if validators.is_empty() {
            None
        } else {
            Some(Arc::new(NumberCompositeValidator { validators })
                as Arc<
                    dyn AsyncValidator<Input = Value, Output = (), Error = ValidationError>
                        + Send
                        + Sync,
                >)
        };

        ParameterValidation {
            validator,
            required: self.required,
            message: self.message,
            key: None,
        }
    }
}

impl Default for NumberValidationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Composite Validators (bridge TypedValidator to AsyncValidator on Value)
// =============================================================================

/// Composite validator for strings
struct StringCompositeValidator {
    validators: Vec<
        Box<dyn TypedValidator<Input = str, Output = (), Error = ValidationError> + Send + Sync>,
    >,
}

#[async_trait::async_trait]
impl AsyncValidator for StringCompositeValidator {
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    async fn validate_async(&self, value: &Value) -> Result<(), ValidationError> {
        // Extract string from Value
        let s = value
            .as_text()
            .ok_or_else(|| ValidationError::new("type_error", "Expected text value"))?;

        // Run all validators
        for validator in &self.validators {
            validator.validate(s.as_str())?;
        }

        Ok(())
    }
}

/// Composite validator for numbers
struct NumberCompositeValidator {
    validators: Vec<
        Box<dyn TypedValidator<Input = f64, Output = (), Error = ValidationError> + Send + Sync>,
    >,
}

#[async_trait::async_trait]
impl AsyncValidator for NumberCompositeValidator {
    type Input = Value;
    type Output = ();
    type Error = ValidationError;

    async fn validate_async(&self, value: &Value) -> Result<(), ValidationError> {
        // Extract number from Value
        let num = match value {
            Value::Integer(i) => {
                // Convert Integer to i64 then to f64
                let i64_val: i64 = i.value();
                i64_val as f64
            }
            Value::Float(f) => {
                // Convert Float to f64
                let f64_val: f64 = f.value();
                f64_val
            }
            _ => return Err(ValidationError::new("type_error", "Expected numeric value")),
        };

        // Run all validators
        for validator in &self.validators {
            validator.validate(&num)?;
        }

        Ok(())
    }
}

// =============================================================================
// Convenience constructors
// =============================================================================

impl ParameterValidation {
    /// Start building string validation
    #[must_use]
    pub fn string() -> StringValidationBuilder {
        StringValidationBuilder::new()
    }

    /// Start building number validation
    #[must_use]
    pub fn number() -> NumberValidationBuilder {
        NumberValidationBuilder::new()
    }

    /// Quick email validation
    #[must_use]
    pub fn email() -> Self {
        Self::string().email().build()
    }

    /// Quick URL validation
    #[must_use]
    pub fn url() -> Self {
        Self::string().url().build()
    }

    /// Quick required validation
    #[must_use]
    pub fn required_field() -> Self {
        Self {
            validator: None,
            required: true,
            message: Some("This field is required".to_string()),
            key: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_string_validation() {
        let validation = ParameterValidation::string()
            .min_length(3)
            .max_length(10)
            .build();

        // Valid
        assert!(
            validation
                .validate(&Value::text("hello"), None)
                .await
                .is_ok()
        );

        // Too short
        assert!(validation.validate(&Value::text("hi"), None).await.is_err());

        // Too long
        assert!(
            validation
                .validate(&Value::text("hello world!"), None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_email_validation() {
        let validation = ParameterValidation::email();

        // Valid email
        assert!(
            validation
                .validate(&Value::text("user@example.com"), None)
                .await
                .is_ok()
        );

        // Invalid email
        assert!(
            validation
                .validate(&Value::text("not-an-email"), None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_number_validation() {
        let validation = ParameterValidation::number().min(0.0).max(100.0).build();

        // Valid
        assert!(validation.validate(&Value::float(50.0), None).await.is_ok());

        // Too small
        assert!(
            validation
                .validate(&Value::float(-10.0), None)
                .await
                .is_err()
        );

        // Too large
        assert!(
            validation
                .validate(&Value::float(150.0), None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_required_validation() {
        let validation = ParameterValidation::required_field();

        // Null value should fail
        assert!(validation.validate(&Value::Null, None).await.is_err());

        // Non-null value should pass
        assert!(
            validation
                .validate(&Value::text("anything"), None)
                .await
                .is_ok()
        );
    }
}
