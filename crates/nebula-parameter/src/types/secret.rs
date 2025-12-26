use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for password or sensitive inputs
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct SecretParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<SecretParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct SecretParameterOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Minimum number of characters
    pub min_length: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Maximum number of characters
    pub max_length: Option<usize>,

    /// Whether the value should be masked even in API responses (for extra security)
    #[builder(default)]
    #[serde(default)]
    pub always_masked: bool,
}

impl Describable for SecretParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Secret
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for SecretParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretParameter({})", self.metadata.name)
    }
}

impl Validatable for SecretParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        if let Some(expected) = self.expected_kind() {
            let actual = value.kind();
            if actual != ValueKind::Null && actual != expected {
                return Err(ParameterError::InvalidType {
                    key: self.metadata.key.clone(),
                    expected_type: expected.name().to_string(),
                    actual_details: actual.name().to_string(),
                });
            }
        }

        // Required check
        if self.is_empty(value) && self.is_required() {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        // Length validation
        if let Some(text) = value.as_text() {
            if let Some(options) = &self.options {
                let len = text.len();

                if let Some(min_length) = options.min_length
                    && len < min_length
                {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!(
                            "Secret must be at least {min_length} characters, got {len}"
                        ),
                    });
                }

                if let Some(max_length) = options.max_length
                    && len > max_length
                {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!(
                            "Secret must be at most {max_length} characters, got {len}"
                        ),
                    });
                }
            }
        }

        Ok(())
    }

    fn is_empty(&self, value: &Value) -> bool {
        match value {
            Value::Text(t) => t.is_empty(),
            Value::Null => true,
            _ => true,
        }
    }
}

impl Displayable for SecretParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl SecretParameter {
    /// Get the value length without exposing the actual value
    #[must_use]
    pub fn value_length(value: &Value) -> Option<usize> {
        match value {
            Value::Text(t) => Some(t.len()),
            _ => None,
        }
    }

    /// Check if the secret value is set (without exposing it)
    #[must_use]
    pub fn has_value(value: &Value) -> bool {
        match value {
            Value::Text(t) => !t.is_empty(),
            _ => false,
        }
    }

    /// Create a masked representation of the value for display
    #[must_use]
    pub fn masked_value(value: &Value) -> Option<String> {
        match value {
            Value::Text(t) => Some("*".repeat(t.len().clamp(3, 8))),
            _ => None,
        }
    }
}
