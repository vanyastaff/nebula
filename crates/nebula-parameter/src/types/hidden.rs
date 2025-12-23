use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::traits::Expressible;
use crate::core::{HasValue, Parameter, ParameterError, ParameterKind, ParameterMetadata};
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Parameter that is hidden from the user interface but can store values
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct HiddenParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Current value of the parameter
    pub value: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<String>,
}

impl Parameter for HiddenParameter {
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
impl Expressible for HiddenParameter {
    fn to_expression(&self) -> Option<MaybeExpression<Value>> {
        self.value.as_ref().map(|s| {
            MaybeExpression::Value(nebula_value::Value::Text(nebula_value::Text::from(
                s.clone(),
            )))
        })
    }

    fn from_expression(
        &mut self,
        value: impl Into<MaybeExpression<Value>> + Send,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            MaybeExpression::Value(nebula_value::Value::Text(s)) => {
                self.value = Some(s.to_string());
                Ok(())
            }
            MaybeExpression::Expression(expr) => {
                // Hidden parameters commonly use expressions - store the expression source
                self.value = Some(expr.source);
                Ok(())
            }
            _ => {
                // Hidden parameters are flexible and can store any value as string
                self.value = Some(format!("{value:?}"));
                Ok(())
            }
        }
    }
}

// Hidden parameters don't implement Validatable or Displayable by design
// They're meant to be internal-only values
