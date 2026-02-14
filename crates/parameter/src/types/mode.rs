use serde::{Deserialize, Serialize};

use crate::def::ParameterDef;
use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// How the mode selector is rendered in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeSelectorStyle {
    Dropdown,
    Radio,
    Tabs,
}

/// A single variant within a mode parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeVariant {
    /// Unique key for this variant.
    pub key: String,

    /// Display name for this variant.
    pub name: String,

    /// Optional description shown in the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The parameters shown when this variant is active.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<ParameterDef>,
}

impl ModeVariant {
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            description: None,
            parameters: Vec::new(),
        }
    }

    /// Add a parameter to this variant (builder-style).
    #[must_use]
    pub fn with_parameter(mut self, param: ParameterDef) -> Self {
        self.parameters.push(param);
        self
    }
}

/// Options specific to mode parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeOptions {
    /// How the variant selector is rendered.
    #[serde(default = "default_selector_style")]
    pub selector_style: ModeSelectorStyle,
}

fn default_selector_style() -> ModeSelectorStyle {
    ModeSelectorStyle::Dropdown
}

/// A mutually exclusive set of parameter groups.
///
/// Use case: Auth method (API Key vs OAuth shows different fields).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeParameter {
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

impl ModeParameter {
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

    /// Look up a variant by key.
    #[must_use]
    pub fn get_variant(&self, key: &str) -> Option<&ModeVariant> {
        self.variants.iter().find(|v| v.key == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SecretParameter, TextParameter};

    #[test]
    fn new_creates_minimal_mode() {
        let p = ModeParameter::new("auth", "Authentication");
        assert_eq!(p.metadata.key, "auth");
        assert_eq!(p.metadata.name, "Authentication");
        assert!(p.variants.is_empty());
        assert!(p.default_variant.is_none());
        assert!(p.options.is_none());
        assert!(p.display.is_none());
        assert!(p.validation.is_empty());
    }

    #[test]
    fn variant_builder() {
        let v = ModeVariant::new("api_key", "API Key")
            .with_parameter(ParameterDef::Secret(SecretParameter::new("key", "Key")));

        assert_eq!(v.key, "api_key");
        assert_eq!(v.name, "API Key");
        assert_eq!(v.parameters.len(), 1);
        assert_eq!(v.parameters[0].key(), "key");
    }

    #[test]
    fn get_variant_finds_by_key() {
        let mut p = ModeParameter::new("auth", "Auth");
        p.variants.push(
            ModeVariant::new("api_key", "API Key")
                .with_parameter(ParameterDef::Secret(SecretParameter::new("key", "Key"))),
        );
        p.variants.push(
            ModeVariant::new("oauth", "OAuth").with_parameter(ParameterDef::Text(
                TextParameter::new("client_id", "Client ID"),
            )),
        );

        assert!(p.get_variant("api_key").is_some());
        assert_eq!(p.get_variant("oauth").unwrap().name, "OAuth");
        assert!(p.get_variant("missing").is_none());
    }

    #[test]
    fn serde_round_trip() {
        let mut p = ModeParameter::new("auth", "Auth Method");
        p.default_variant = Some("api_key".into());
        p.options = Some(ModeOptions {
            selector_style: ModeSelectorStyle::Tabs,
        });
        p.variants
            .push(
                ModeVariant::new("api_key", "API Key").with_parameter(ParameterDef::Secret(
                    SecretParameter::new("api_key", "API Key"),
                )),
            );
        p.variants.push(ModeVariant::new("oauth", "OAuth"));

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: ModeParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "auth");
        assert_eq!(deserialized.variants.len(), 2);
        assert_eq!(deserialized.default_variant.as_deref(), Some("api_key"));
        assert_eq!(
            deserialized.options.unwrap().selector_style,
            ModeSelectorStyle::Tabs
        );
    }

    #[test]
    fn selector_style_serde() {
        let styles = [
            ModeSelectorStyle::Dropdown,
            ModeSelectorStyle::Radio,
            ModeSelectorStyle::Tabs,
        ];

        for style in &styles {
            let json = serde_json::to_string(style).unwrap();
            let deserialized: ModeSelectorStyle = serde_json::from_str(&json).unwrap();
            assert_eq!(*style, deserialized);
        }
    }
}
