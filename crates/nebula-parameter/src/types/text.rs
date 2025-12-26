use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterBase, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for single-line text input
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// // Using builder with Into conversions
/// let param = TextParameter::builder()
///     .base(ParameterBase::new(
///         ParameterMetadata::builder()
///             .key("username")
///             .name("Username")
///             .description("Enter your username")
///             .build()?
///     ))
///     .default("guest")  // &str -> Text via Into
///     .options(TextParameterOptions::builder()
///         .min_length(3)
///         .max_length(20)
///         .pattern(r"^[a-zA-Z0-9_]+$")
///         .build())
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
#[builder(on(String, into))]
pub struct TextParameter {
    /// Base parameter fields (metadata, display, validation)
    #[serde(flatten)]
    pub base: ParameterBase,

    /// Default value if parameter is not set
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(into)]
    pub default: Option<nebula_value::Text>,

    /// Configuration options for this parameter type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<TextParameterOptions>,
}

/// Configuration options for text parameters
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::TextParameterOptions;
///
/// let options = TextParameterOptions::builder()
///     .min_length(3)
///     .max_length(100)
///     .pattern(r"^[a-zA-Z]+$")  // &str -> String via Into
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
#[builder(on(String, into))]
pub struct TextParameterOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Regex pattern for validation
    pub pattern: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Maximum number of characters
    pub max_length: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Minimum number of characters
    pub min_length: Option<usize>,
}

impl Describable for TextParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Text
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.base.metadata
    }
}

impl std::fmt::Display for TextParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TextParameter({})", self.base.metadata.name)
    }
}

impl Validatable for TextParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check (from expected_kind) + required check
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

        // Options validation (min_length, max_length, pattern)
        if let Some(text) = value.as_text()
            && let Some(opts) = &self.options
        {
            if let Some(min) = opts.min_length
                && text.len() < min
            {
                return Err(ParameterError::InvalidValue {
                    key: self.base.metadata.key.clone(),
                    reason: format!("Text length {} below minimum {}", text.len(), min),
                });
            }
            if let Some(max) = opts.max_length
                && text.len() > max
            {
                return Err(ParameterError::InvalidValue {
                    key: self.base.metadata.key.clone(),
                    reason: format!("Text length {} above maximum {}", text.len(), max),
                });
            }
            // Pattern validation if regex crate is available
        }

        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.base.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().map(|s| s.is_empty()).unwrap_or(false)
    }
}

impl Displayable for TextParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.base.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.base.display = display;
    }
}
