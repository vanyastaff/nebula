//! Generic Mode parameter for mutually-exclusive parameter groups.

use serde::{Deserialize, Serialize};

use crate::def::ParameterDef;
use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::types::mode::{ModeOptions, ModeSelectorStyle, ModeVariant};
use crate::validation::ValidationRule;

/// A mutually exclusive set of parameter groups.
///
/// Use case: Auth method (API Key vs OAuth shows different fields).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mode {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// The available variants.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<ModeVariant>,

    /// Key of the initially active variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_variant: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ModeOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl Mode {
    /// Create a mode builder.
    #[must_use]
    pub fn builder(key: impl Into<String>) -> ModeBuilder {
        ModeBuilder::new(key)
    }

    /// Create a minimal mode parameter.
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            variants: Vec::new(),
            default_variant: None,
            options: None,
            display: None,
            validation: Vec::new(),
        }
    }

    /// Find variant by key.
    #[must_use]
    pub fn get_variant(&self, key: &str) -> Option<&ModeVariant> {
        self.variants.iter().find(|v| v.key == key)
    }
}

/// Builder for `Mode`.
#[derive(Debug)]
pub struct ModeBuilder {
    metadata: ParameterMetadata,
    variants: Vec<ModeVariant>,
    default_variant: Option<String>,
    options: Option<ModeOptions>,
    display: Option<ParameterDisplay>,
    validation: Vec<ValidationRule>,
}

impl ModeBuilder {
    fn new(key: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, ""),
            variants: Vec::new(),
            default_variant: None,
            options: None,
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

    /// Add variant.
    #[must_use]
    pub fn variant(mut self, variant: ModeVariant) -> Self {
        self.variants.push(variant);
        self
    }

    /// Add multiple variants.
    #[must_use]
    pub fn variants(mut self, variants: impl IntoIterator<Item = ModeVariant>) -> Self {
        self.variants.extend(variants);
        self
    }

    /// Set default variant key.
    #[must_use]
    pub fn default_variant(mut self, key: impl Into<String>) -> Self {
        self.default_variant = Some(key.into());
        self
    }

    /// Set selector style.
    #[must_use]
    pub fn selector_style(mut self, style: ModeSelectorStyle) -> Self {
        self.options
            .get_or_insert_with(|| ModeOptions {
                selector_style: ModeSelectorStyle::Dropdown,
            })
            .selector_style = style;
        self
    }

    /// Add validation rule.
    #[must_use]
    pub fn validation(mut self, rule: ValidationRule) -> Self {
        self.validation.push(rule);
        self
    }

    /// Build mode parameter.
    #[must_use]
    pub fn build(self) -> Mode {
        let mut metadata = self.metadata;
        if metadata.name.is_empty() {
            metadata.name = metadata.key.clone();
        }

        Mode {
            metadata,
            variants: self.variants,
            default_variant: self.default_variant,
            options: self.options,
            display: self.display,
            validation: self.validation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ModeVariant, SecretParameter};

    #[test]
    fn builder_creates_mode() {
        let api_key = ModeVariant::new("api_key", "API Key")
            .with_parameter(ParameterDef::Secret(SecretParameter::new("key", "Key")));

        let oauth = ModeVariant::new("oauth", "OAuth");

        let mode = Mode::builder("auth")
            .label("Authentication")
            .variant(api_key)
            .variant(oauth)
            .default_variant("api_key")
            .selector_style(ModeSelectorStyle::Tabs)
            .build();

        assert_eq!(mode.metadata.key, "auth");
        assert_eq!(mode.variants.len(), 2);
        assert_eq!(mode.default_variant.as_deref(), Some("api_key"));
        assert_eq!(
            mode.options.as_ref().map(|o| o.selector_style),
            Some(ModeSelectorStyle::Tabs)
        );
    }
}
