//! Number parameter type for numeric input

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for numeric input
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = NumberParameter::builder()
///     .key("age")
///     .name("Age")
///     .description("Your age in years")
///     .required(true)
///     .default(18.0)
///     .options(
///         NumberParameterOptions::builder()
///             .min(0.0)
///             .max(150.0)
///             .step(1.0)
///             .precision(0)
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<f64>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<NumberParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for number parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NumberParameterOptions {
    /// Minimum allowed value
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,

    /// Maximum allowed value
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,

    /// Step increment for the value
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,

    /// Number of decimal places to allow
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
}

// =============================================================================
// NumberParameter Builder
// =============================================================================

/// Builder for `NumberParameter`
#[derive(Debug, Default)]
pub struct NumberParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<f64>,
    options: Option<NumberParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl NumberParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> NumberParameterBuilder {
        NumberParameterBuilder::new()
    }
}

impl NumberParameterBuilder {
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
    pub fn default(mut self, default: f64) -> Self {
        self.default = Some(default);
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: NumberParameterOptions) -> Self {
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

    /// Build the `NumberParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<NumberParameter, ParameterError> {
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

        // Apply optional metadata fields
        let mut metadata = metadata;
        metadata.placeholder = self.placeholder;
        metadata.hint = self.hint;

        Ok(NumberParameter {
            metadata,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// NumberParameterOptions Builder
// =============================================================================

/// Builder for `NumberParameterOptions`
#[derive(Debug, Default)]
pub struct NumberParameterOptionsBuilder {
    min: Option<f64>,
    max: Option<f64>,
    step: Option<f64>,
    precision: Option<u8>,
}

impl NumberParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> NumberParameterOptionsBuilder {
        NumberParameterOptionsBuilder::default()
    }
}

impl NumberParameterOptionsBuilder {
    /// Set minimum allowed value
    #[must_use]
    pub fn min(mut self, min: f64) -> Self {
        self.min = Some(min);
        self
    }

    /// Set maximum allowed value
    #[must_use]
    pub fn max(mut self, max: f64) -> Self {
        self.max = Some(max);
        self
    }

    /// Set step increment
    #[must_use]
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }

    /// Set precision (decimal places)
    #[must_use]
    pub fn precision(mut self, precision: u8) -> Self {
        self.precision = Some(precision);
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> NumberParameterOptions {
        NumberParameterOptions {
            min: self.min,
            max: self.max,
            step: self.step,
            precision: self.precision,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for NumberParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Number
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for NumberParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NumberParameter({})", self.metadata.name)
    }
}

impl Validatable for NumberParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        // Numbers can be integers or floats, but Float is the more general type
        Some(ValueKind::Float)
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Check required
        if self.is_required() && self.is_empty(value) {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        // Type check - allow null or number
        if !value.is_null() && value.as_float().is_none() && value.as_integer().is_none() {
            return Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected numeric value".to_string(),
            });
        }

        // Options validation (min, max, step, precision)
        if let Some(num) = value
            .as_float()
            .map(|f| f.value())
            .or_else(|| value.as_integer().map(|i| i.value() as f64))
        {
            self.validate_number(num)?;
        }

        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null()
    }
}

impl Displayable for NumberParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl NumberParameter {
    /// Validate that a number is within the configured constraints
    fn validate_number(&self, num: f64) -> Result<(), ParameterError> {
        if let Some(options) = &self.options {
            // Check minimum
            if let Some(min) = options.min
                && num < min
            {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("Value {num} is below minimum {min}"),
                });
            }

            // Check maximum
            if let Some(max) = options.max
                && num > max
            {
                return Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: format!("Value {num} is above maximum {max}"),
                });
            }

            // Check step
            if let Some(step) = options.step
                && let Some(min) = options.min
            {
                let offset = (num - min) % step;
                if offset.abs() > f64::EPSILON {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Value {num} does not align with step {step}"),
                    });
                }
            }

            // Apply precision if specified
            if let Some(precision) = options.precision {
                let multiplier = 10_f64.powi(i32::from(precision));
                let rounded = (num * multiplier).round() / multiplier;
                if (num - rounded).abs() > f64::EPSILON {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!(
                            "Value {num} exceeds precision of {precision} decimal places"
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    /// Get the minimum allowed value
    #[must_use]
    pub fn get_min(&self) -> Option<f64> {
        self.options.as_ref().and_then(|opts| opts.min)
    }

    /// Get the maximum allowed value
    #[must_use]
    pub fn get_max(&self) -> Option<f64> {
        self.options.as_ref().and_then(|opts| opts.max)
    }

    /// Get the step increment
    #[must_use]
    pub fn get_step(&self) -> Option<f64> {
        self.options.as_ref().and_then(|opts| opts.step)
    }

    /// Get the precision (decimal places)
    #[must_use]
    pub fn get_precision(&self) -> Option<u8> {
        self.options.as_ref().and_then(|opts| opts.precision)
    }

    /// Check if a value is within bounds
    #[must_use]
    pub fn is_within_bounds(&self, value: f64) -> bool {
        self.validate_number(value).is_ok()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_number_parameter_builder() {
        let param = NumberParameter::builder()
            .key("age")
            .name("Age")
            .description("Your age in years")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "age");
        assert_eq!(param.metadata.name, "Age");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_number_parameter_with_options() {
        let param = NumberParameter::builder()
            .key("quantity")
            .name("Quantity")
            .options(
                NumberParameterOptions::builder()
                    .min(0.0)
                    .max(100.0)
                    .step(1.0)
                    .precision(0)
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.options.unwrap();
        assert_eq!(opts.min, Some(0.0));
        assert_eq!(opts.max, Some(100.0));
        assert_eq!(opts.step, Some(1.0));
        assert_eq!(opts.precision, Some(0));
    }

    #[test]
    fn test_number_parameter_with_default() {
        let param = NumberParameter::builder()
            .key("count")
            .name("Count")
            .default(42.0)
            .build()
            .unwrap();

        assert_eq!(param.default, Some(42.0));
    }

    #[test]
    fn test_number_parameter_missing_key() {
        let result = NumberParameter::builder().name("Age").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_number_parameter_serialization() {
        let param = NumberParameter::builder()
            .key("test")
            .name("Test")
            .description("A test parameter")
            .required(true)
            .build()
            .unwrap();

        let json = serde_json::to_string(&param).unwrap();
        let deserialized: NumberParameter = serde_json::from_str(&json).unwrap();

        assert_eq!(param.metadata.key, deserialized.metadata.key);
        assert_eq!(param.metadata.name, deserialized.metadata.name);
    }

    #[test]
    fn test_number_validation_bounds() {
        let param = NumberParameter::builder()
            .key("score")
            .name("Score")
            .options(
                NumberParameterOptions::builder()
                    .min(0.0)
                    .max(100.0)
                    .build(),
            )
            .build()
            .unwrap();

        assert!(param.is_within_bounds(50.0));
        assert!(param.is_within_bounds(0.0));
        assert!(param.is_within_bounds(100.0));
        assert!(!param.is_within_bounds(-1.0));
        assert!(!param.is_within_bounds(101.0));
    }
}
