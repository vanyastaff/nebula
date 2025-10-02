use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, HasValue, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterType, ParameterValidation, ParameterValue, Validatable,
};

/// Parameter for password or sensitive inputs
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct SecretParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<SecretParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct SecretParameterOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,

    /// Whether to show a "reveal" button to temporarily show the password
    #[serde(default = "default_show_reveal")]
    pub show_reveal: bool,

    /// Whether the value should be masked even in API responses (for extra security)
    #[serde(default)]
    pub always_masked: bool,
}

fn default_show_reveal() -> bool {
    true
}

impl ParameterType for SecretParameter {
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

impl HasValue for SecretParameter {
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
                self.value = Some(s.to_string());
                Ok(())
            }
            ParameterValue::Expression(expr) => {
                // Expressions are allowed for secrets (e.g., from environment variables)
                self.value = Some(expr);
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for secret".to_string(),
            }),
        }
    }
}

impl Validatable for SecretParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn value_to_json(&self, _value: &Self::Value) -> serde_json::Value {
        // Never expose the actual secret value in JSON
        serde_json::Value::String("***REDACTED***".to_string())
    }

    fn is_empty_value(&self, value: &Self::Value) -> bool {
        value.is_empty()
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
    pub fn value_length(&self) -> Option<usize> {
        self.value.as_ref().map(|v| v.len())
    }

    /// Check if the secret value is set (without exposing it)
    pub fn has_value(&self) -> bool {
        self.value.is_some() && !self.value.as_ref().unwrap().is_empty()
    }

    /// Create a masked representation of the value for display
    pub fn masked_value(&self) -> Option<String> {
        self.value
            .as_ref()
            .map(|v| "*".repeat(v.len().min(8).max(3)))
    }
}
