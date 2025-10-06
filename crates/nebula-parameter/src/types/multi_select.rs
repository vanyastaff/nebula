use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, HasValue, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterType, ParameterValidation, ParameterValue, SelectOption, Validatable,
};

/// Parameter for selecting multiple options from a dropdown
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct MultiSelectParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Vec<String>>,

    pub options: Vec<SelectOption>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_select_options: Option<MultiSelectParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct MultiSelectParameterOptions {
    /// Minimum number of selections required
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_selections: Option<usize>,

    /// Maximum number of selections allowed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_selections: Option<usize>,
}

impl ParameterType for MultiSelectParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::MultiSelect
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for MultiSelectParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MultiSelectParameter({})", self.metadata.name)
    }
}

impl HasValue for MultiSelectParameter {
    type Value = Vec<String>;

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
        self.value.as_ref().map(|vec| {
            let values: Vec<nebula_value::Value> = vec
                .iter()
                .map(|s| nebula_value::Value::text(s.clone()))
                .collect();
            ParameterValue::Value(nebula_value::Value::Array(
                nebula_value::Array::from_nebula_values(values),
            ))
        })
    }

    fn set_parameter_value(
        &mut self,
        value: impl Into<ParameterValue>,
    ) -> Result<(), ParameterError> {
        let value = value.into();
        match value {
            ParameterValue::Value(nebula_value::Value::Array(arr)) => {
                let mut string_values = Vec::new();

                // arr.iter() returns &serde_json::Value, convert to nebula_value::Value
                use crate::JsonValueExt;
                for item in arr.iter() {
                    if let Some(nebula_val) = item.to_nebula_value() {
                        match nebula_val {
                            nebula_value::Value::Text(s) => {
                                string_values.push(s.to_string());
                            }
                            _ => {
                                return Err(ParameterError::InvalidValue {
                                    key: self.metadata.key.clone(),
                                    reason:
                                        "All array items must be strings for multi-select parameter"
                                            .to_string(),
                                });
                            }
                        }
                    }
                }

                // Validate all selected options exist and constraints are met
                if self.are_valid_selections(&string_values)? {
                    self.value = Some(string_values);
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: "One or more selected values are not valid options".to_string(),
                    })
                }
            }
            ParameterValue::Expression(expr) => {
                // For expressions, store as single-item array with the expression
                self.value = Some(vec![expr]);
                Ok(())
            }
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected array value for multi-select parameter".to_string(),
            }),
        }
    }
}

impl Validatable for MultiSelectParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }
    fn is_empty_value(&self, value: &Self::Value) -> bool {
        value.is_empty()
    }
}

impl Displayable for MultiSelectParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl MultiSelectParameter {
    /// Validate all selected values are valid options and meet constraints
    fn are_valid_selections(&self, selections: &[String]) -> Result<bool, ParameterError> {
        // Check if all selections are expressions
        if selections.len() == 1 && selections[0].starts_with("{{") && selections[0].ends_with("}}")
        {
            return Ok(true); // Allow expressions
        }

        // Validate each selection is a valid option
        for selection in selections {
            if !self.is_valid_option(selection) {
                return Ok(false);
            }
        }

        // Check min/max constraints
        if let Some(options) = &self.multi_select_options {
            if let Some(min) = options.min_selections {
                if selections.len() < min {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!(
                            "Must select at least {} options, got {}",
                            min,
                            selections.len()
                        ),
                    });
                }
            }
            if let Some(max) = options.max_selections {
                if selections.len() > max {
                    return Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!(
                            "Must select at most {} options, got {}",
                            max,
                            selections.len()
                        ),
                    });
                }
            }
        }

        Ok(true)
    }

    /// Check if a single value is a valid option
    fn is_valid_option(&self, value: &str) -> bool {
        if value.is_empty() {
            return false;
        }

        // Check if value matches any option's value or key
        self.options
            .iter()
            .any(|option| option.value == value || option.key == value)
    }

    /// Get option by value
    pub fn get_option_by_value(&self, value: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.value == value)
    }

    /// Get option by key
    pub fn get_option_by_key(&self, key: &str) -> Option<&SelectOption> {
        self.options.iter().find(|option| option.key == key)
    }

    /// Get display names for current selections
    pub fn get_display_names(&self) -> Vec<String> {
        if let Some(selections) = &self.value {
            selections
                .iter()
                .filter_map(|value| {
                    self.get_option_by_value(value)
                        .map(|option| option.name.clone())
                        .or_else(|| Some(value.clone())) // Fallback to raw value
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Add a selection if it's valid and not already selected
    pub fn add_selection(&mut self, value: String) -> Result<(), ParameterError> {
        if !self.is_valid_option(&value) {
            return Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: format!("Value '{}' is not a valid option", value),
            });
        }

        let mut current = self.value.clone().unwrap_or_default();

        // Don't add if already selected
        if current.contains(&value) {
            return Ok(());
        }

        current.push(value);

        // Validate constraints
        self.are_valid_selections(&current)?;
        self.value = Some(current);
        Ok(())
    }

    /// Remove a selection
    pub fn remove_selection(&mut self, value: &str) -> Result<(), ParameterError> {
        if let Some(current) = &mut self.value {
            current.retain(|v| v != value);
        }

        // Validate constraints after removal
        if let Some(current) = &self.value {
            self.are_valid_selections(current)?;
        }
        Ok(())
    }

    /// Toggle a selection (add if not present, remove if present)
    pub fn toggle_selection(&mut self, value: String) -> Result<(), ParameterError> {
        if let Some(current) = &self.value {
            if current.contains(&value) {
                self.remove_selection(&value)
            } else {
                self.add_selection(value)
            }
        } else {
            self.add_selection(value)
        }
    }

    /// Check if a value is currently selected
    pub fn is_selected(&self, value: &str) -> bool {
        self.value
            .as_ref()
            .map(|selections| selections.contains(&value.to_string()))
            .unwrap_or(false)
    }

    /// Get the number of current selections
    pub fn selection_count(&self) -> usize {
        self.value.as_ref().map(|v| v.len()).unwrap_or(0)
    }

    /// Check if minimum selections requirement is met
    pub fn meets_minimum(&self) -> bool {
        if let Some(options) = &self.multi_select_options {
            if let Some(min) = options.min_selections {
                return self.selection_count() >= min;
            }
        }
        true // No minimum requirement
    }

    /// Check if maximum selections limit is exceeded
    pub fn exceeds_maximum(&self) -> bool {
        if let Some(options) = &self.multi_select_options {
            if let Some(max) = options.max_selections {
                return self.selection_count() > max;
            }
        }
        false // No maximum limit
    }

    /// Get available slots for more selections
    pub fn remaining_slots(&self) -> Option<usize> {
        if let Some(options) = &self.multi_select_options {
            if let Some(max) = options.max_selections {
                let current = self.selection_count();
                return Some(max.saturating_sub(current));
            }
        }
        None // No limit
    }
}
