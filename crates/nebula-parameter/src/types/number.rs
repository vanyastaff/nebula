use serde::{Deserialize, Serialize};

use crate::core::traits::ParameterValue;
use crate::core::{
    Displayable, Parameter, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::Value;

/// Parameter for numeric input
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = NumberParameter::builder()
///     .metadata(ParameterMetadata::new()
///         .key("age")
///         .name("Age")
///         .description("Your age in years")
///         .call()?)
///     .default(18.0)
///     .options(NumberParameterOptions::builder()
///         .min(0.0)
///         .max(150.0)
///         .step(1.0)
///         .precision(0)
///         .build())
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct NumberParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<NumberParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for number parameters
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::NumberParameterOptions;
///
/// // All fields are optional
/// let options = NumberParameterOptions::builder()
///     .min(0.0)
///     .max(100.0)
///     .step(0.5)
///     .precision(2)
///     .build();
///
/// // Or just set what you need
/// let options = NumberParameterOptions::builder()
///     .min(0.0)
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
pub struct NumberParameterOptions {
    /// Minimum allowed value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,

    /// Maximum allowed value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,

    /// Step increment for the value
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,

    /// Number of decimal places to allow
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<u8>,
}

impl Parameter for NumberParameter {
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

impl ParameterValue for NumberParameter {
    fn validate_value(
        &self,
        value: &Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), ParameterError>> + Send + '_>>
    {
        let value = value.clone();
        Box::pin(async move { self.validate(&value).await })
    }

    fn accepts_value(&self, value: &Value) -> bool {
        value.is_null() || value.as_float().is_some() || value.as_integer().is_some()
    }

    fn expected_type(&self) -> &'static str {
        "number"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
