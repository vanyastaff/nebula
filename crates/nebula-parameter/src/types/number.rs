use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::traits::Expressible;
use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Parameter for numeric input
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct NumberParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Current value of the parameter
    pub value: Option<f64>,

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

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
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

impl HasValue for NumberParameter {
    type Value = f64;

    fn get(&self) -> Option<&Self::Value> {
        self.value.as_ref()
    }

    fn get_mut(&mut self) -> Option<&mut Self::Value> {
        self.value.as_mut()
    }

    fn set(&mut self, value: Self::Value) -> Result<(), ParameterError> {
        self.value = Some(value);
        Ok(())
    }

    fn default(&self) -> Option<&Self::Value> {
        self.default.as_ref()
    }

    fn clear(&mut self) {
        self.value = None;
    }
}

impl Validatable for NumberParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, _value: &Self::Value) -> bool {
        false // Numbers are never considered empty
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

#[async_trait::async_trait]
impl Expressible for NumberParameter {
    fn to_expression(&self) -> Option<MaybeExpression<Value>> {
        self.value
            .map(|n| MaybeExpression::Value(Value::Float(nebula_value::Float::new(n))))
    }

    fn from_expression(
        &mut self,
        value: impl Into<MaybeExpression<Value>> + Send,
    ) -> Result<(), ParameterError> {
        match value.into() {
            MaybeExpression::Value(Value::Integer(i)) => {
                let num = i.value() as f64;
                self.validate_number(num)?;
                self.value = Some(num);
                Ok(())
            }
            MaybeExpression::Value(Value::Float(f)) => {
                let num = f.value();
                self.validate_number(num)?;
                self.value = Some(num);
                Ok(())
            }
            MaybeExpression::Expression(expr) => {
                if let Ok(num) = expr.source.parse::<f64>() {
                    self.validate_number(num)?;
                    self.value = Some(num);
                }
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected numeric value".to_string(),
            }),
        }
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

    /// Check if the current value is within bounds
    #[must_use]
    pub fn is_within_bounds(&self) -> bool {
        if let Some(value) = self.value {
            self.validate_number(value).is_ok()
        } else {
            true
        }
    }
}
