use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::traits::Expressible;
use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Parameter for multi-line text input
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct TextareaParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Current value of the parameter
    pub value: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<TextareaParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct TextareaParameterOptions {
    /// Minimum number of characters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,

    /// Maximum number of characters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}

impl Parameter for TextareaParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Textarea
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for TextareaParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextareaParameter({})", self.metadata.name)
    }
}

impl HasValue for TextareaParameter {
    type Value = nebula_value::Text;

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

#[async_trait::async_trait]
impl Expressible for TextareaParameter {
    fn to_expression(&self) -> Option<MaybeExpression<Value>> {
        self.value
            .as_ref()
            .map(|s| MaybeExpression::Value(nebula_value::Value::Text(s.clone())))
    }

    fn from_expression(
        &mut self,
        value: impl Into<MaybeExpression<Value>> + Send,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            MaybeExpression::Value(nebula_value::Value::Text(s)) => {
                // Use Text directly
                // Validate length constraints from options
                if let Some(options) = &self.options {
                    if let Some(min_len) = options.min_length {
                        if s.len() < min_len {
                            return Err(ParameterError::InvalidValue {
                                key: self.metadata.key.clone(),
                                reason: format!(
                                    "Text too short: {} chars, minimum {}",
                                    s.len(),
                                    min_len
                                ),
                            });
                        }
                    }
                    if let Some(max_len) = options.max_length {
                        if s.len() > max_len {
                            return Err(ParameterError::InvalidValue {
                                key: self.metadata.key.clone(),
                                reason: format!(
                                    "Text too long: {} chars, maximum {}",
                                    s.len(),
                                    max_len
                                ),
                            });
                        }
                    }
                }
                self.value = Some(s);
                Ok(())
            }
            MaybeExpression::Expression(expr) => {
                // Allow expressions for dynamic text
                self.value = Some(nebula_value::Text::from(expr));
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for textarea parameter".to_string(),
            }),
        }
    }
}

impl Validatable for TextareaParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, value: &Self::Value) -> bool {
        value.trim().is_empty()
    }
}

impl Displayable for TextareaParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl TextareaParameter {
    /// Get character count for current value
    pub fn character_count(&self) -> usize {
        self.value.as_ref().map(|v| v.len()).unwrap_or(0)
    }

    /// Get remaining characters if max_length is set
    pub fn remaining_characters(&self) -> Option<i32> {
        if let Some(options) = &self.options {
            if let Some(max_len) = options.max_length {
                let current = self.character_count();
                return Some(max_len as i32 - current as i32);
            }
        }
        None
    }
}
