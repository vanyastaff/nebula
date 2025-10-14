use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::traits::Expressible;
use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, SelectOption, Validatable,
};
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Parameter for selecting a single option from radio buttons
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct RadioParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Current value of the parameter
    pub value: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    /// Available options for selection
    pub options: Vec<SelectOption>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub radio_options: Option<RadioParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct RadioParameterOptions {
    /// Show "other" option with text input
    #[serde(default)]
    pub allow_other: bool,

    /// Label for the "other" option
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_label: Option<String>,
}

impl Parameter for RadioParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Radio
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for RadioParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RadioParameter({})", self.metadata.name)
    }
}

impl HasValue for RadioParameter {
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
impl Expressible for RadioParameter {
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
                // Use Text directly
                // Validate that the value is one of the available options or "other"
                if self.is_valid_option(s.as_str()) {
                    self.value = Some(s);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Value '{}' is not a valid radio option", s.as_str()),
                    })
                }
            }
            MaybeExpression::Expression(expr) => {
                // Allow expressions for dynamic selection
                self.value = Some(nebula_value::Text::from(expr.source.as_str()));
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for radio parameter".to_string(),
            }),
        }
    }
}

impl Validatable for RadioParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, value: &Self::Value) -> bool {
        value.is_empty()
    }
}

impl Displayable for RadioParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl RadioParameter {
    /// Check if a value matches one of the available options
    fn is_valid_option(&self, value: &str) -> bool {
        if value.is_empty() {
            return false;
        }

        // Check for expressions (start with {{ and end with }})
        if value.starts_with("{{") && value.ends_with("}}") {
            return true;
        }

        // Check if value matches any option's value or key
        let is_standard_option = self
            .options
            .iter()
            .any(|option| option.value == value || option.key == value);

        if is_standard_option {
            return true;
        }

        // Check if "other" is allowed and this might be an "other" value
        if let Some(radio_options) = &self.radio_options
            && radio_options.allow_other
        {
            // For "other" option, we accept any non-empty string
            return !value.is_empty();
        }

        false
    }

    /// Get option by value
    #[must_use]
    pub fn get_option_by_value(&self, value: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.value == value)
    }

    /// Get option by key
    #[must_use]
    pub fn get_option_by_key(&self, key: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.key == key)
    }

    /// Get the display name for the current value
    #[must_use]
    pub fn get_display_name(&self) -> Option<String> {
        if let Some(value) = &self.value {
            if let Some(option) = self.get_option_by_value(value) {
                return Some(option.name.clone());
            }
            // If not found in options and "other" is allowed, return as-is
            if let Some(radio_options) = &self.radio_options
                && radio_options.allow_other
            {
                return Some(value.to_string());
            }
        }
        None
    }

    /// Check if "other" option is allowed
    #[must_use]
    pub fn allows_other(&self) -> bool {
        self.radio_options
            .as_ref()
            .is_some_and(|opts| opts.allow_other)
    }

    /// Get the "other" option label
    #[must_use]
    pub fn get_other_label(&self) -> String {
        self.radio_options
            .as_ref()
            .and_then(|opts| opts.other_label.as_ref())
            .cloned()
            .unwrap_or_else(|| "Other".to_string())
    }
}
