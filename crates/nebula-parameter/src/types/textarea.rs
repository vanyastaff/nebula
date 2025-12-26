use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterBase, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for multi-line text input
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct TextareaParameter {
    /// Base parameter fields (metadata, display, validation)
    #[serde(flatten)]
    pub base: ParameterBase,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<TextareaParameterOptions>,
}

#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct TextareaParameterOptions {
    /// Minimum number of characters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,

    /// Maximum number of characters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
}

impl Describable for TextareaParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Textarea
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

impl std::fmt::Display for TextareaParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextareaParameter({})", self.base.metadata.name)
    }
}

impl Validatable for TextareaParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        if let Some(expected) = self.expected_kind() {
            let actual = value.kind();
            if actual != ValueKind::Null && actual != expected {
                return Err(ParameterError::InvalidType {
                    key: self.base.metadata.key.clone(),
                    expected_type: expected.name().to_string(),
                    actual_details: actual.name().to_string(),
                });
            }
        }

        // Required check
        if self.is_required() && self.is_empty(value) {
            return Err(ParameterError::MissingValue {
                key: self.base.metadata.key.clone(),
            });
        }

        // Validate length constraints from options
        if let Some(text) = value.as_text()
            && let Some(opts) = &self.options
        {
            if let Some(min_len) = opts.min_length
                && text.len() < min_len
            {
                return Err(ParameterError::InvalidValue {
                    key: self.base.metadata.key.clone(),
                    reason: format!("Text too short: {} chars, minimum {}", text.len(), min_len),
                });
            }
            if let Some(max_len) = opts.max_length
                && text.len() > max_len
            {
                return Err(ParameterError::InvalidValue {
                    key: self.base.metadata.key.clone(),
                    reason: format!("Text too long: {} chars, maximum {}", text.len(), max_len),
                });
            }
        }

        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.base.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null()
            || value
                .as_text()
                .map(|s| s.trim().is_empty())
                .unwrap_or(false)
    }
}

impl Displayable for TextareaParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
    }
}

impl TextareaParameter {
    /// Get character count for a value
    #[must_use]
    pub fn character_count(&self, value: &nebula_value::Text) -> usize {
        value.len()
    }

    /// Get remaining characters if `max_length` is set
    #[must_use]
    pub fn remaining_characters(&self, value: &nebula_value::Text) -> Option<i32> {
        if let Some(options) = &self.options
            && let Some(max_len) = options.max_length
        {
            let current = self.character_count(value);
            // Use try_from to safely convert usize to i32, saturating at i32::MAX if too large
            let max = i32::try_from(max_len).unwrap_or(i32::MAX);
            let curr = i32::try_from(current).unwrap_or(i32::MAX);
            return Some(max.saturating_sub(curr));
        }
        None
    }
}
