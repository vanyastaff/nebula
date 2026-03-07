//! Generic Hidden parameter for values not shown in UI.

use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// A hidden parameter that carries a value but is not shown in the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hidden {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl Hidden {
    /// Create hidden builder.
    #[must_use]
    pub fn builder(key: impl Into<String>) -> HiddenBuilder {
        HiddenBuilder::new(key)
    }

    /// Create minimal hidden parameter.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            display: None,
            validation: Vec::new(),
        }
    }
}

/// Builder for `Hidden`.
#[derive(Debug)]
pub struct HiddenBuilder {
    metadata: ParameterMetadata,
    default: Option<serde_json::Value>,
    display: Option<ParameterDisplay>,
    validation: Vec<ValidationRule>,
}

impl HiddenBuilder {
    fn new(key: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, ""),
            default: None,
            display: None,
            validation: Vec::new(),
        }
    }

    /// Set display label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.metadata.name = label.into();
        self
    }

    /// Set description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.metadata.description = Some(description.into());
        self
    }

    /// Set default value.
    #[must_use]
    pub fn default_value(mut self, value: serde_json::Value) -> Self {
        self.default = Some(value);
        self
    }

    /// Add validation rule.
    #[must_use]
    pub fn validation(mut self, rule: ValidationRule) -> Self {
        self.validation.push(rule);
        self
    }

    /// Build hidden parameter.
    #[must_use]
    pub fn build(self) -> Hidden {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        Hidden {
            metadata,
            default: self.default,
            display: self.display,
            validation: self.validation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builder_creates_hidden() {
        let h = Hidden::builder("internal")
            .label("Internal")
            .default_value(json!({"v": 1}))
            .build();

        assert_eq!(h.metadata.key, "internal");
        assert_eq!(h.default, Some(json!({"v": 1})));
    }
}
