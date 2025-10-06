use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable,  HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, Validatable,
};
use crate::core::traits::Expressible;
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Parameter for password or sensitive inputs
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct SecretParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Text>,

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

    /// Whether the value should be masked even in API responses (for extra security)
    #[serde(default)]
    pub always_masked: bool,
}

impl Parameter for SecretParameter {
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
impl Expressible for SecretParameter {
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
                self.value = Some(s);
                Ok(())
            }
            MaybeExpression::Expression(expr) => {
                // Expressions are allowed for secrets (e.g., from environment variables)
                self.value = Some(nebula_value::Text::from(expr));
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
    fn is_empty(&self, value: &Self::Value) -> bool {
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
