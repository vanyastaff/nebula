use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    ParameterError, ParameterMetadata, ParameterValue, ParameterType,
    HasValue, ParameterKind,
};

/// Parameter that is hidden from the user interface but can store values
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct HiddenParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl ParameterType for HiddenParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Hidden
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for HiddenParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HiddenParameter({})", self.metadata.name)
    }
}

impl HasValue for HiddenParameter {
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
        self.value.as_ref().map(|s| ParameterValue::Value(nebula_value::Value::String(s.clone().into())))
    }

    fn set_parameter_value(&mut self, value: ParameterValue) -> Result<(), ParameterError> {
        match value {
            ParameterValue::Value(nebula_value::Value::String(s)) => {
                self.value = Some(s.to_string());
                Ok(())
            },
            ParameterValue::Expression(expr) => {
                // Hidden parameters commonly use expressions for computed values
                self.value = Some(expr);
                Ok(())
            },
            _ => {
                // Hidden parameters are flexible and can store any value as string
                self.value = Some(format!("{:?}", value));
                Ok(())
            }
        }
    }
}

// Hidden parameters don't implement Validatable or Displayable by design
// They're meant to be internal-only values