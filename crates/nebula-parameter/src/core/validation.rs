//! Parameter validation using nebula-validator
//!
//! This module provides an ergonomic API for parameter validation that wraps
//! the powerful `nebula-validator` crate with parameter-specific conveniences.
//!
//! # Architecture
//!
//! - `ValueCondition` - Core condition type for evaluating field values
//! - `ParameterValidation` - Configuration holding validators
//! - Universal `from()` method to wrap any validator from `nebula-validator`
//!
//! # Examples
//!
//! ```ignore
//! use nebula_parameter::prelude::*;
//! use nebula_validator::validators::string::{min_length, max_length, email};
//! use nebula_validator::validators::numeric::{min, max, positive};
//! use nebula_validator::combinators::and;
//!
//! // String validation
//! let validation = ParameterValidation::from(and(min_length(3), max_length(50)));
//!
//! // Email validation
//! let email_validation = ParameterValidation::email();
//!
//! // Number range
//! let age_validation = ParameterValidation::from(and(min(18.0), max(120.0)));
//! ```

use nebula_core::ParameterKey;
use nebula_validator::core::{
    AsValidatable, AsyncValidator, ValidationContext, ValidationError, Validator,
};
use nebula_validator::validators::string::{email, url};
use nebula_value::Value;
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::marker::PhantomData;
use std::sync::Arc;

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

    // === Form-level rules ===
    /// At least one of the specified fields must be set.
    AtLeastOneOf(Vec<ParameterKey>),

    /// Exactly one of the specified fields must be set.
    ExactlyOneOf(Vec<ParameterKey>),

    /// All specified fields must be set, or none of them.
    AllOrNone(Vec<ParameterKey>),

    /// If this field is set, then required fields must also be set.
    RequiresWith(Vec<ParameterKey>),

    /// If this field is set, then forbidden fields must not be set.
    ConflictsWith(Vec<ParameterKey>),
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

            // Cross-field and form-level require context, return false here
            Self::Field(_, _)
            | Self::EqualsField(_)
            | Self::NotEqualsField(_)
            | Self::AtLeastOneOf(_)
            | Self::ExactlyOneOf(_)
            | Self::AllOrNone(_)
            | Self::RequiresWith(_)
            | Self::ConflictsWith(_) => false,
        }
    }

    /// Check if this condition requires context (validation state or other fields).
    #[must_use]
    pub fn requires_context(&self) -> bool {
        match self {
            Self::IsValid | Self::IsInvalid => true,
            Self::Field(_, _)
            | Self::EqualsField(_)
            | Self::NotEqualsField(_)
            | Self::AtLeastOneOf(_)
            | Self::ExactlyOneOf(_)
            | Self::AllOrNone(_)
            | Self::RequiresWith(_)
            | Self::ConflictsWith(_) => true,
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
            Self::AtLeastOneOf(keys)
            | Self::ExactlyOneOf(keys)
            | Self::AllOrNone(keys)
            | Self::RequiresWith(keys)
            | Self::ConflictsWith(keys) => {
                fields.extend(keys.iter().cloned());
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

    // === Form-level constructors ===

    /// At least one of the specified fields must be set.
    pub fn at_least_one_of(keys: impl IntoIterator<Item = impl Into<ParameterKey>>) -> Self {
        Self::AtLeastOneOf(keys.into_iter().map(Into::into).collect())
    }

    /// Exactly one of the specified fields must be set.
    pub fn exactly_one_of(keys: impl IntoIterator<Item = impl Into<ParameterKey>>) -> Self {
        Self::ExactlyOneOf(keys.into_iter().map(Into::into).collect())
    }

    /// All specified fields must be set, or none of them.
    pub fn all_or_none(keys: impl IntoIterator<Item = impl Into<ParameterKey>>) -> Self {
        Self::AllOrNone(keys.into_iter().map(Into::into).collect())
    }

    /// If this field is set, then required fields must also be set.
    pub fn requires_with(keys: impl IntoIterator<Item = impl Into<ParameterKey>>) -> Self {
        Self::RequiresWith(keys.into_iter().map(Into::into).collect())
    }

    /// If this field is set, then forbidden fields must not be set.
    pub fn conflicts_with(keys: impl IntoIterator<Item = impl Into<ParameterKey>>) -> Self {
        Self::ConflictsWith(keys.into_iter().map(Into::into).collect())
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
    validator: Option<Arc<dyn AsyncValidator<Input = Value> + Send + Sync>>,

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
        V: AsyncValidator<Input = Value> + Send + Sync + 'static,
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
// Universal Value Validator Adapter
// =============================================================================

/// Universal adapter that converts any `Validator<Input=T>` to work with `Value`.
///
/// Uses `AsValidatable` trait to automatically extract the correct type from Value.
/// This enables any validator from `nebula-validator` to work with parameter values
/// without manual bridging code.
///
/// # Example
///
/// ```ignore
/// use nebula_validator::validators::string::min_length;
/// use nebula_validator::combinators::and;
///
/// // Any string validator works automatically
/// let validator = ValueValidatorAdapter::new(and(min_length(3), max_length(20)));
/// validator.validate_async(&Value::text("hello")).await; // Ok
/// ```
pub struct ValueValidatorAdapter<V, T: ?Sized> {
    validator: V,
    _phantom: PhantomData<fn() -> T>,
}

impl<V, T: ?Sized> ValueValidatorAdapter<V, T> {
    /// Create a new adapter wrapping a validator.
    pub fn new(validator: V) -> Self {
        Self {
            validator,
            _phantom: PhantomData,
        }
    }
}

#[async_trait::async_trait]
impl<V, T> AsyncValidator for ValueValidatorAdapter<V, T>
where
    V: Validator<Input = T> + Send + Sync,
    T: ?Sized + 'static,
    Value: AsValidatable<T>,
    for<'a> <Value as AsValidatable<T>>::Output<'a>: Borrow<T>,
{
    type Input = Value;

    async fn validate_async(&self, value: &Value) -> Result<(), ValidationError> {
        let extracted = AsValidatable::<T>::as_validatable(value)?;
        self.validator.validate(extracted.borrow())
    }
}

// =============================================================================
// Convenience constructors
// =============================================================================

impl ParameterValidation {
    /// Create validation from any validator.
    ///
    /// This is the universal way to use validators from `nebula-validator`.
    /// The type is automatically extracted from `Value` using `AsValidatable`.
    /// If the value type doesn't match, validation returns a type mismatch error.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use nebula_validator::validators::string::{min_length, email};
    /// use nebula_validator::validators::numeric::{min, positive};
    /// use nebula_validator::combinators::and;
    ///
    /// // String validators
    /// let validation = ParameterValidation::from(and(min_length(3), email()));
    ///
    /// // Number validators
    /// let validation = ParameterValidation::from(and(positive(), min(0.0)));
    ///
    /// // If value type doesn't match validator's expected type,
    /// // validation fails with "type_mismatch" error
    /// ```
    pub fn from<V, T>(validator: V) -> Self
    where
        V: Validator<Input = T> + Send + Sync + 'static,
        T: ?Sized + 'static,
        Value: AsValidatable<T>,
        for<'a> <Value as AsValidatable<T>>::Output<'a>: Borrow<T>,
    {
        Self {
            validator: Some(Arc::new(ValueValidatorAdapter::<V, T>::new(validator))),
            required: false,
            message: None,
            key: None,
        }
    }

    /// Quick email validation
    #[must_use]
    pub fn email() -> Self {
        Self::from(email())
    }

    /// Quick URL validation
    #[must_use]
    pub fn url() -> Self {
        Self::from(url())
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
    use nebula_validator::combinators::and;
    use nebula_validator::validators::string::{max_length, min_length};

    #[tokio::test]
    async fn test_string_validation() {
        let validation = ParameterValidation::from(and(min_length(3), max_length(10)));

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
        use nebula_validator::validators::numeric::{max, min};
        let validation = ParameterValidation::from(and(min(0.0), max(100.0)));

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

    #[tokio::test]
    async fn test_type_mismatch() {
        let validation = ParameterValidation::from(min_length(3));

        // String works
        assert!(
            validation
                .validate(&Value::text("hello"), None)
                .await
                .is_ok()
        );

        // Number fails with type mismatch
        let err = validation.validate(&Value::integer(42), None).await;
        assert!(err.is_err());
        assert_eq!(err.unwrap_err().code, "type_mismatch");
    }
}
