use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, HasValue, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterType, ParameterValidation, ParameterValue, Validatable,
};

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct TextareaParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<TextareaParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
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

impl ParameterType for TextareaParameter {
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
    type Value = String;

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
            .as_ref()
            .map(|s| ParameterValue::Value(nebula_value::Value::text(s.clone())))
    }

    fn set_parameter_value(
        &mut self,
        value: impl Into<ParameterValue>,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            ParameterValue::Value(nebula_value::Value::Text(s)) => {
                let text = s.to_string();
                // Validate length constraints from options
                if let Some(options) = &self.options {
                    if let Some(min_len) = options.min_length {
                        if text.len() < min_len {
                            return Err(ParameterError::InvalidValue {
                                key: self.metadata.key.clone(),
                                reason: format!(
                                    "Text too short: {} chars, minimum {}",
                                    text.len(),
                                    min_len
                                ),
                            });
                        }
                    }
                    if let Some(max_len) = options.max_length {
                        if text.len() > max_len {
                            return Err(ParameterError::InvalidValue {
                                key: self.metadata.key.clone(),
                                reason: format!(
                                    "Text too long: {} chars, maximum {}",
                                    text.len(),
                                    max_len
                                ),
                            });
                        }
                    }
                }
                self.value = Some(text);
                Ok(())
            }
            ParameterValue::Expression(expr) => {
                // Allow expressions for dynamic text
                self.value = Some(expr);
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

    fn value_to_json(&self, value: &Self::Value) -> serde_json::Value {
        serde_json::Value::String(value.clone())
    }

    fn is_empty_value(&self, value: &Self::Value) -> bool {
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
