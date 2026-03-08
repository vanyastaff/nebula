//! Generic checkbox parameter with trait-based boolean subtypes.

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::subtype::traits::BooleanSubtype;
use crate::validation::ValidationRule;

/// Options for checkbox parameters.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CheckboxOptions {
    /// Label shown near the checkbox.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Additional help text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
}

/// Generic checkbox parameter with type-safe subtype.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkbox<S: BooleanSubtype> {
    /// Common parameter metadata (`key`, `name`, `description`, flags).
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default boolean value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<bool>,

    /// Checkbox-specific options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<CheckboxOptions>,

    /// Semantic boolean subtype.
    #[serde(rename = "subtype")]
    pub subtype: S,

    /// UI display rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules applied to this parameter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl<S: BooleanSubtype> Checkbox<S> {
    /// Creates a new checkbox parameter.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            options: None,
            subtype: S::default(),
            display: None,
            validation: Vec::new(),
        }
    }

    /// Creates a builder.
    #[must_use]
    pub fn builder(key: impl Into<String>) -> CheckboxBuilder<S> {
        CheckboxBuilder::new(key)
    }
}

/// Builder for generic checkbox parameters.
#[derive(Debug, Clone)]
pub struct CheckboxBuilder<S: BooleanSubtype> {
    key: String,
    name: Option<String>,
    description: Option<String>,
    default: Option<bool>,
    options: CheckboxOptions,
    subtype: S,
    required: bool,
}

impl<S: BooleanSubtype> CheckboxBuilder<S> {
    /// Creates a new builder.
    pub fn new(key: impl Into<String>) -> Self {
        let subtype = S::default();
        let mut options = CheckboxOptions::default();

        if let Some(label) = S::label() {
            options.label = Some(label.to_owned());
        }

        if let Some(help_text) = S::help_text() {
            options.help_text = Some(help_text.to_owned());
        }

        Self {
            key: key.into(),
            name: None,
            description: None,
            default: S::default_value(),
            options,
            subtype,
            required: false,
        }
    }

    /// Sets the display label.
    #[must_use]
    pub fn label(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the description.
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Sets the default value.
    #[must_use]
    pub fn default_value(mut self, value: bool) -> Self {
        self.default = Some(value);
        self
    }

    /// Marks as required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Sets the inline checkbox label.
    #[must_use]
    pub fn option_label(mut self, label: impl Into<String>) -> Self {
        self.options.label = Some(label.into());
        self
    }

    /// Sets help text.
    #[must_use]
    pub fn help_text(mut self, help_text: impl Into<String>) -> Self {
        self.options.help_text = Some(help_text.into());
        self
    }

    /// Builds the parameter.
    pub fn build(self) -> Checkbox<S> {
        let key = self.key;
        let name = self.name.unwrap_or_else(|| key.clone());
        let mut metadata = ParameterMetadata::new(key, name);
        metadata.description = self.description;
        metadata.required = self.required;

        Checkbox {
            metadata,
            default: self.default,
            options: if self.options.label.is_some() || self.options.help_text.is_some() {
                Some(self.options)
            } else {
                None
            },
            subtype: self.subtype,
            display: None,
            validation: Vec::new(),
        }
    }
}

impl<S: BooleanSubtype> crate::common::ParameterType for Checkbox<S> {
    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }

    fn metadata_mut(&mut self) -> &mut ParameterMetadata {
        &mut self.metadata
    }

    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn display_mut(&mut self) -> &mut Option<ParameterDisplay> {
        &mut self.display
    }

    fn validation_rules(&self) -> &[ValidationRule] {
        &self.validation
    }

    fn validation_rules_mut(&mut self) -> &mut Vec<ValidationRule> {
        &mut self.validation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subtype::std_subtypes::{Consent, Toggle};

    #[test]
    fn toggle_builder_applies_defaults() {
        let p = Checkbox::<Toggle>::builder("enabled")
            .label("Enabled")
            .build();
        assert_eq!(p.default, Some(false));
    }

    #[test]
    fn consent_builder_applies_help_text() {
        let p = Checkbox::<Consent>::builder("consent")
            .label("Consent")
            .build();
        assert!(
            p.options
                .as_ref()
                .and_then(|o| o.help_text.as_ref())
                .is_some()
        );
    }
}
