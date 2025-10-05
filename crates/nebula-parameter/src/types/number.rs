use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, HasValue, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterType, ParameterValidation, ParameterValue, Validatable,
};

/// Parameter for numeric input
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct NumberParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<f64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<NumberParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
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

impl ParameterType for NumberParameter {
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
        self.value
            .map(|n| ParameterValue::Value(nebula_value::Value::float(n)))
    }

    fn set_parameter_value(
        &mut self,
        value: impl Into<ParameterValue>,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            ParameterValue::Value(nebula_value::Value::Integer(i)) => {
                let num = i.value() as f64;
                self.validate_number(num)?;
                self.value = Some(num);
                Ok(())
            }
            ParameterValue::Value(nebula_value::Value::Float(f)) => {
                let num = f.value();
                self.validate_number(num)?;
                self.value = Some(num);
                Ok(())
            }
            ParameterValue::Expression(expr) => {
                // Allow expressions for dynamic numbers
                // Try to parse as number, otherwise store for later evaluation
                if let Ok(num) = expr.parse::<f64>() {
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

impl Validatable for NumberParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn value_to_json(&self, value: &Self::Value) -> serde_json::Value {
        serde_json::json!(value)
    }

    fn is_empty_value(&self, _value: &Self::Value) -> bool {
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

impl NumberParameter {
    /// Validate that a number is within the configured constraints
    fn validate_number(&self, num: f64) -> Result<(), ParameterError> {
        if let Some(options) = &self.options {
            // Check minimum
            if let Some(min) = options.min {
                if num < min {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Value {} is below minimum {}", num, min),
                    });
                }
            }

            // Check maximum
            if let Some(max) = options.max {
                if num > max {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Value {} is above maximum {}", num, max),
                    });
                }
            }

            // Check step
            if let Some(step) = options.step {
                if let Some(min) = options.min {
                    let offset = (num - min) % step;
                    if offset.abs() > f64::EPSILON {
                        return Err(ParameterError::InvalidValue {
                            key: self.metadata.key.clone(),
                            reason: format!("Value {} does not align with step {}", num, step),
                        });
                    }
                }
            }

            // Apply precision if specified
            if let Some(precision) = options.precision {
                let multiplier = 10_f64.powi(precision as i32);
                let rounded = (num * multiplier).round() / multiplier;
                if (num - rounded).abs() > f64::EPSILON {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!(
                            "Value {} exceeds precision of {} decimal places",
                            num, precision
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    /// Get the minimum allowed value
    pub fn get_min(&self) -> Option<f64> {
        self.options.as_ref().and_then(|opts| opts.min)
    }

    /// Get the maximum allowed value
    pub fn get_max(&self) -> Option<f64> {
        self.options.as_ref().and_then(|opts| opts.max)
    }

    /// Get the step increment
    pub fn get_step(&self) -> Option<f64> {
        self.options.as_ref().and_then(|opts| opts.step)
    }

    /// Get the precision (decimal places)
    pub fn get_precision(&self) -> Option<u8> {
        self.options.as_ref().and_then(|opts| opts.precision)
    }

    /// Check if the current value is within bounds
    pub fn is_within_bounds(&self) -> bool {
        if let Some(value) = self.value {
            self.validate_number(value).is_ok()
        } else {
            true
        }
    }
}
