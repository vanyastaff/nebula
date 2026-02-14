use serde::{Deserialize, Serialize};

use crate::display::ParameterDisplay;
use crate::metadata::ParameterMetadata;
use crate::validation::ValidationRule;

/// Supported color representation formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorFormat {
    Hex,
    Rgb,
    Hsl,
}

/// Options specific to color parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorOptions {
    /// The color format to use.
    pub format: ColorFormat,
}

/// A color picker parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColorParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ColorOptions>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub validation: Vec<ValidationRule>,
}

impl ColorParameter {
    #[must_use]
    pub fn new(key: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            metadata: ParameterMetadata::new(key, name),
            default: None,
            options: None,
            display: None,
            validation: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_minimal_color() {
        let p = ColorParameter::new("bg_color", "Background Color");
        assert_eq!(p.metadata.key, "bg_color");
        assert!(p.default.is_none());
    }

    #[test]
    fn serde_round_trip() {
        let p = ColorParameter {
            metadata: ParameterMetadata::new("accent", "Accent Color"),
            default: Some("#ff5500".into()),
            options: Some(ColorOptions {
                format: ColorFormat::Hex,
            }),
            display: None,
            validation: vec![],
        };

        let json = serde_json::to_string(&p).unwrap();
        let deserialized: ColorParameter = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.metadata.key, "accent");
        assert_eq!(deserialized.default.as_deref(), Some("#ff5500"));
        assert_eq!(
            deserialized.options.as_ref().unwrap().format,
            ColorFormat::Hex
        );
    }

    #[test]
    fn color_format_serde() {
        for fmt in [ColorFormat::Hex, ColorFormat::Rgb, ColorFormat::Hsl] {
            let json = serde_json::to_string(&fmt).unwrap();
            let deserialized: ColorFormat = serde_json::from_str(&json).unwrap();
            assert_eq!(fmt, deserialized);
        }
    }
}
