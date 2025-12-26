use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};

use nebula_value::{Boolean, Value, ValueKind};

/// Parameter for boolean checkbox
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct CheckboxParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<Boolean>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<CheckboxParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct CheckboxParameterOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Custom label text for the checkbox
    pub label: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Help text displayed below the checkbox
    pub help_text: Option<String>,
}

impl Describable for CheckboxParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Checkbox
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for CheckboxParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CheckboxParameter({})", self.metadata.name)
    }
}

impl Validatable for CheckboxParameter {
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::Boolean)
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
        if self.is_required() && self.is_empty(value) {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
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

impl Displayable for CheckboxParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}
