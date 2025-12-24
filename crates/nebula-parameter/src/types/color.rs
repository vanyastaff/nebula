use serde::{Deserialize, Serialize};

use crate::core::{
    Displayable, Parameter, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::Value;

/// Parameter for color selection
#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct ColorParameter {
    #[serde(flatten)]
    /// Parameter metadata including key, name, description
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Default value if parameter is not set
    pub default: Option<nebula_value::Text>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Configuration options for this parameter type
    pub options: Option<ColorParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display rules controlling when this parameter is shown
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    /// Validation rules for this parameter
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, bon::Builder, Serialize, Deserialize)]
pub struct ColorParameterOptions {
    /// Color format: "hex", "rgb", "hsl", "hsv"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ColorFormat>,

    /// Whether to show an alpha/opacity channel
    #[builder(default)]
    #[serde(default)]
    pub allow_alpha: bool,

    /// Predefined color palette
    #[serde(skip_serializing_if = "Option::is_none")]
    pub palette: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum ColorFormat {
    #[serde(rename = "hex")]
    #[default]
    Hex,
    #[serde(rename = "rgb")]
    Rgb,
    #[serde(rename = "hsl")]
    Hsl,
    #[serde(rename = "hsv")]
    Hsv,
}

impl Parameter for ColorParameter {
    fn kind(&self) -> ParameterKind {
        ParameterKind::Color
    }

    fn metadata(&self) -> &ParameterMetadata {
        &self.metadata
    }
}

impl std::fmt::Display for ColorParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ColorParameter({})", self.metadata.name)
    }
}

impl Validatable for ColorParameter {
    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Check required
        if self.is_required() && self.is_empty(value) {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
            });
        }

        // Type check - allow null or text
        if !value.is_null() && value.as_text().is_none() {
            return Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected text value".to_string(),
            });
        }

        // Validate color format
        if let Some(text) = value.as_text()
            && !self.is_valid_color(text.as_str())
        {
            return Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: format!("Invalid color format: {}", text.as_str()),
            });
        }

        Ok(())
    }

    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn is_empty(&self, value: &Value) -> bool {
        value.is_null() || value.as_text().map(|s| s.is_empty()).unwrap_or(false)
    }
}

impl Displayable for ColorParameter {
    fn display(&self) -> Option<&ParameterDisplay> {
        self.display.as_ref()
    }

    fn set_display(&mut self, display: Option<ParameterDisplay>) {
        self.display = display;
    }
}

impl ColorParameter {
    /// Validate if a string is a valid color
    fn is_valid_color(&self, color: &str) -> bool {
        if color.is_empty() {
            return false;
        }

        // Check for expressions (start with {{ and end with }})
        if color.starts_with("{{") && color.ends_with("}}") {
            return true;
        }

        let format = self
            .options
            .as_ref()
            .and_then(|opts| opts.format.as_ref())
            .unwrap_or(&ColorFormat::Hex);

        match format {
            ColorFormat::Hex => self.is_valid_hex_color(color),
            ColorFormat::Rgb => self.is_valid_rgb_color(color),
            ColorFormat::Hsl => self.is_valid_hsl_color(color),
            ColorFormat::Hsv => self.is_valid_hsv_color(color),
        }
    }

    /// Check if string is valid hex color (#RRGGBB or #RGB)
    fn is_valid_hex_color(&self, color: &str) -> bool {
        if !color.starts_with('#') {
            return false;
        }

        let hex = &color[1..];
        match hex.len() {
            3 | 6 | 8 => hex.chars().all(|c| c.is_ascii_hexdigit()),
            _ => false,
        }
    }

    /// Check if string is valid RGB color (rgb(r,g,b) or rgba(r,g,b,a))
    fn is_valid_rgb_color(&self, color: &str) -> bool {
        color.starts_with("rgb(") && color.ends_with(')')
            || color.starts_with("rgba(") && color.ends_with(')')
    }

    /// Check if string is valid HSL color (hsl(h,s%,l%) or hsla(h,s%,l%,a))
    fn is_valid_hsl_color(&self, color: &str) -> bool {
        color.starts_with("hsl(") && color.ends_with(')')
            || color.starts_with("hsla(") && color.ends_with(')')
    }

    /// Check if string is valid HSV color
    fn is_valid_hsv_color(&self, color: &str) -> bool {
        color.starts_with("hsv(") && color.ends_with(')')
            || color.starts_with("hsva(") && color.ends_with(')')
    }

    /// Convert color to specified format (basic implementation)
    #[must_use]
    pub fn convert_to_format(
        &self,
        color: &nebula_value::Text,
        format: ColorFormat,
    ) -> Option<String> {
        // This is a simplified implementation
        // In a real application, you'd use a proper color conversion library
        match format {
            ColorFormat::Hex if !color.starts_with("#") => Some(format!("#{color}")),
            _ => Some(color.to_string()),
        }
    }
}
