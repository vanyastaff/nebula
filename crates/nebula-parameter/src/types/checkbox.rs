use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, HasValue, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterType, ParameterValidation, Validatable,
};

use nebula_expression::MaybeExpression;
use nebula_value::{Boolean, Value};

/// Parameter for boolean checkbox
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct CheckboxParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Boolean>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Boolean>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<CheckboxParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct CheckboxParameterOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
}

impl ParameterType for CheckboxParameter {
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

impl HasValue for CheckboxParameter {
    type Value = Boolean;

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

    fn get_parameter_value(&self) -> Option<MaybeExpression<Value>> {
        self.value
            .map(|b| MaybeExpression::Value(Value::boolean(b.value())))
    }

    fn set_parameter_value(
        &mut self,
        value: impl Into<MaybeExpression<Value>>,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            MaybeExpression::Value(Value::Boolean(b)) => {
                self.value = Some(Boolean::new(b));
                Ok(())
            }
            MaybeExpression::Expression(_expr) => {
                // Checkboxes cannot be expressions, return error
                Err(ParameterError::InvalidValue {
                    key: self.metadata.key.clone(),
                    reason: "Checkbox parameter cannot be an expression".to_string(),
                })
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected boolean value".to_string(),
            }),
        }
    }
}

impl Validatable for CheckboxParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty_value(&self, _value: &Self::Value) -> bool {
        false // Booleans are never considered empty
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
