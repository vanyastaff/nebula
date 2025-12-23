use serde::{Deserialize, Serialize};

use crate::core::traits::Expressible;
use crate::core::{
    Displayable, HasValue, Parameter, ParameterDisplay, ParameterError, ParameterKind,
    ParameterMetadata, ParameterValidation, SelectOption, Validatable,
};
use nebula_expression::MaybeExpression;
use nebula_value::Value;

/// Parameter for single-choice selection from a dropdown
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::prelude::*;
///
/// let param = SelectParameter::builder()
///     .metadata(ParameterMetadata::new()
///         .key("auth_type")
///         .name("Authentication Type")
///         .description("Choose authentication method")
///         .call()?)
///     .options(vec![
///         SelectOption::new("api_key", "API Key", "api_key"),
///         SelectOption::new("oauth", "OAuth 2.0", "oauth"),
///         SelectOption::new("basic", "Basic Auth", "basic"),
///     ])
///     .default("api_key")  // &str -> Text via Into
///     .select_options(SelectParameterOptions::builder()
///         .placeholder("Select authentication...")
///         .build())
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
#[builder(on(String, into))]
pub struct SelectParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(into)]
    /// Current value of the parameter
    pub value: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(into)]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    /// Available options for selection
    #[builder(with = FromIterator::from_iter)]
    pub options: Vec<SelectOption>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub select_options: Option<SelectParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for select parameters
///
/// # Examples
///
/// ```rust
/// use nebula_parameter::SelectParameterOptions;
///
/// let options = SelectParameterOptions::builder()
///     .multiple(true)
///     .placeholder("Choose an option...")  // &str -> String via Into
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, bon::Builder)]
#[builder(on(String, into))]
pub struct SelectParameterOptions {
    /// Allow multiple selections
    #[serde(default)]
    #[builder(default)]
    pub multiple: bool,

    /// Placeholder text when no selection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

impl Parameter for SelectParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Select
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for SelectParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SelectParameter({})", self.metadata.name)
    }
}

impl HasValue for SelectParameter {
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
impl Expressible for SelectParameter {
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
                // Validate that the value is one of the available options
                if self.is_valid_option(s.as_str()) {
                    self.value = Some(s);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Value '{}' is not a valid option", s.as_str()),
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
                reason: "Expected string value for select parameter".to_string(),
            }),
        }
    }
}

impl Validatable for SelectParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty(&self, value: &Self::Value) -> bool {
        value.is_empty()
    }
}

impl Displayable for SelectParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl SelectParameter {
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
        self.options
            .iter()
            .any(|option| option.value == value || option.key == value)
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
        if let Some(value) = &self.value
            && let Some(option) = self.get_option_by_value(value)
        {
            return Some(option.name.clone());
        }
        None
    }
}
