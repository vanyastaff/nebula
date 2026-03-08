//! Generic text parameter with trait-based subtypes.

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::subtype::traits::TextSubtype;
use crate::validation::ValidationRule;

/// Options for text parameters.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextOptions {
    /// Minimum allowed text length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    /// Maximum allowed text length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    /// Regex pattern for validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
}

/// Generic text parameter with type-safe subtype.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Text<S: TextSubtype> {
    /// Common parameter metadata (`key`, `name`, `description`, flags).
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default text value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    /// Text-specific options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<TextOptions>,

    /// Semantic subtype.
    #[serde(rename = "subtype")]
    pub subtype: S,

    /// UI display rules.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules applied to this parameter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl<S: TextSubtype> Text<S> {
    /// Creates a new text parameter with the given key and name.
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

    /// Creates a builder for this parameter.
    #[must_use]
    pub fn builder(key: impl Into<String>) -> TextBuilder<S> {
        TextBuilder::new(key)
    }

    /// Returns the subtype.
    pub fn subtype(&self) -> &S {
        &self.subtype
    }
}

/// Builder for generic text parameters.
#[derive(Debug, Clone)]
pub struct TextBuilder<S: TextSubtype> {
    key: String,
    name: Option<String>,
    description: Option<String>,
    default: Option<String>,
    options: TextOptions,
    subtype: S,
    required: bool,
    sensitive: bool,
    validation: Vec<ValidationRule>,
}

impl<S: TextSubtype> TextBuilder<S> {
    /// Creates a new builder with the given key.
    pub fn new(key: impl Into<String>) -> Self {
        let subtype = S::default();

        let mut builder = Self {
            key: key.into(),
            name: None,
            description: None,
            default: None,
            options: TextOptions::default(),
            subtype,
            required: false,
            sensitive: S::is_sensitive(),
            validation: Vec::new(),
        };

        if let Some(pattern) = S::pattern() {
            builder.options.pattern = Some(pattern.to_string());
            ValidationRule::replace_in(&mut builder.validation, ValidationRule::pattern(pattern));
        }

        builder
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
    pub fn default_value(mut self, value: impl Into<String>) -> Self {
        self.default = Some(value.into());
        self
    }

    /// Marks the parameter as required.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Sets minimum length constraint.
    #[must_use]
    pub fn min_length(mut self, len: usize) -> Self {
        self.options.min_length = Some(len);
        ValidationRule::replace_in(&mut self.validation, ValidationRule::min_length(len));
        self
    }

    /// Sets maximum length constraint.
    #[must_use]
    pub fn max_length(mut self, len: usize) -> Self {
        self.options.max_length = Some(len);
        ValidationRule::replace_in(&mut self.validation, ValidationRule::max_length(len));
        self
    }

    /// Sets a custom pattern (overrides subtype pattern).
    #[must_use]
    pub fn pattern(mut self, pattern: impl Into<String>) -> Self {
        let pattern = pattern.into();
        self.options.pattern = Some(pattern.clone());
        ValidationRule::replace_in(&mut self.validation, ValidationRule::pattern(pattern));

        self
    }

    /// Builds the parameter.
    pub fn build(self) -> Text<S> {
        let key = self.key;
        let name = self.name.unwrap_or_else(|| key.clone());
        let mut metadata = ParameterMetadata::new(key, name);
        metadata.description = self.description;
        metadata.required = self.required;
        metadata.sensitive = self.sensitive;

        Text {
            metadata,
            default: self.default,
            options: if self.options.min_length.is_some()
                || self.options.max_length.is_some()
                || self.options.pattern.is_some()
            {
                Some(self.options)
            } else {
                None
            },
            subtype: self.subtype,
            display: None,
            validation: self.validation,
        }
    }
}

impl<S: TextSubtype> crate::common::ParameterType for Text<S> {
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
    use crate::subtype::std_subtypes::{Email, Password, Plain};

    #[test]
    fn test_plain_text() {
        let text = Text::<Plain>::new("name", "Name");
        assert_eq!(text.metadata.key, "name");
        assert!(!text.metadata.sensitive);
    }

    #[test]
    fn test_email_with_auto_validation() {
        let email = Text::<Email>::builder("email").label("Email").build();

        assert_eq!(email.metadata.key, "email");
        assert!(email.options.is_some());
        assert!(email.options.as_ref().unwrap().pattern.is_some());
        assert!(!email.validation.is_empty());
    }

    #[test]
    fn test_password_auto_sensitive() {
        let password = Text::<Password>::builder("pass").label("Password").build();

        assert!(password.metadata.sensitive);
    }

    #[test]
    fn test_pattern_replaces_existing_pattern_rule() {
        let text = Text::<Plain>::builder("email")
            .pattern("^first$")
            .pattern("^second$")
            .build();

        assert_eq!(text.validation, vec![ValidationRule::pattern("^second$")]);
    }
}
