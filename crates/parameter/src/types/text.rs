use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::subtype::TextSubtype;
use crate::validation::ValidationRule;

/// Options specific to text parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextOptions {
    /// Regex pattern the value must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,

    /// Maximum allowed character count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,

    /// Minimum required character count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
}

/// A single-line text input parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<TextOptions>,

    /// Semantic subtype for this text parameter
    #[serde(default, skip_serializing_if = "is_default_subtype")]
    pub subtype: TextSubtype,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl TextParameter {
    /// Create a new text parameter with minimal fields.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            options: None,
            subtype: TextSubtype::default(),
            display: None,
            validation: Vec::new(),
        }
    }

    /// Set default value (builder-style).
    #[must_use]
    pub fn default_value(mut self, value: impl Into<String>) -> Self {
        self.default = Some(value.into());
        self
    }

    /// Set minimum length constraint.
    #[must_use]
    pub fn min_length(mut self, length: usize) -> Self {
        self.options
            .get_or_insert_with(TextOptions::default)
            .min_length = Some(length);
        ValidationRule::replace_in(&mut self.validation, ValidationRule::min_length(length));
        self
    }

    /// Set maximum length constraint.
    #[must_use]
    pub fn max_length(mut self, length: usize) -> Self {
        self.options
            .get_or_insert_with(TextOptions::default)
            .max_length = Some(length);
        ValidationRule::replace_in(&mut self.validation, ValidationRule::max_length(length));
        self
    }

    /// Set pattern constraint (regex).
    #[must_use]
    pub fn pattern(mut self, pattern: impl Into<String>) -> Self {
        let pattern = pattern.into();
        self.options
            .get_or_insert_with(TextOptions::default)
            .pattern = Some(pattern.clone());
        ValidationRule::replace_in(&mut self.validation, ValidationRule::pattern(pattern));
        self
    }

    /// Set semantic subtype for this text parameter.
    #[must_use]
    pub fn subtype(mut self, subtype: TextSubtype) -> Self {
        self.subtype = subtype;

        // Auto-apply validation pattern if available
        if let Some(pattern) = subtype.validation_pattern() {
            self.options
                .get_or_insert_with(TextOptions::default)
                .pattern = Some(pattern.to_string());
            ValidationRule::replace_in(&mut self.validation, ValidationRule::pattern(pattern));
        }

        // Auto-mark as sensitive if needed
        if subtype.is_sensitive() {
            self.metadata.sensitive = true;
        }

        self
    }

    /// Convenience: create an email parameter.
    #[must_use]
    pub fn email(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(key, name).subtype(TextSubtype::Email)
    }

    /// Convenience: create a URL parameter.
    #[must_use]
    pub fn url(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(key, name).subtype(TextSubtype::Url)
    }

    /// Convenience: create a password parameter.
    #[must_use]
    pub fn password(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(key, name).subtype(TextSubtype::Password)
    }
}

// Helper for serde skip_serializing_if
fn is_default_subtype(subtype: &TextSubtype) -> bool {
    *subtype == TextSubtype::default()
}

impl crate::common::ParameterType for TextParameter {
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

    #[test]
    fn new_creates_minimal_text() {
        let p = TextParameter::new("username", "Username");
        assert_eq!(p.metadata.key, "username");
        assert_eq!(p.metadata.name, "Username");
        assert!(p.default.is_none());
        assert!(p.options.is_none());
        assert!(p.display.is_none());
        assert!(p.validation.is_empty());
    }

    #[test]
    fn serde_round_trip() {
        let p = TextParameter {
            metadata: ParameterMetadata::new("email", "Email"),
            default: Some("user@example.com".into()),
            options: Some(TextOptions {
                pattern: Some(r"^.+@.+\..+$".into()),
                max_length: Some(255),
                min_length: Some(5),
            }),
            subtype: TextSubtype::Plain,
            display: None,
            validation: vec![ValidationRule::min_length(5)],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: TextParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "email");
        assert_eq!(deserialized.default.as_deref(), Some("user@example.com"));
        assert!(deserialized.options.is_some());
        assert_eq!(deserialized.validation.len(), 1);
    }

    #[test]
    fn pattern_replaces_existing_pattern_rule() {
        let parameter = TextParameter::new("email", "Email")
            .pattern("^first$")
            .pattern("^second$");

        assert_eq!(
            parameter.validation,
            vec![ValidationRule::pattern("^second$")]
        );
    }

    #[test]
    fn subtype_pattern_replaces_manual_pattern_rule() {
        let parameter = TextParameter::new("email", "Email")
            .pattern("^custom$")
            .subtype(TextSubtype::Email);

        assert_eq!(
            parameter
                .validation
                .iter()
                .filter(|rule| matches!(rule, ValidationRule::Pattern { .. }))
                .count(),
            1
        );
    }
}
