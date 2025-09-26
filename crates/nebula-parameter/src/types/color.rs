use bon::Builder;
use serde::{Deserialize, Serialize};

use crate::core::{
    ParameterDisplay, ParameterError, ParameterMetadata, ParameterValidation,
    ParameterValue, ParameterType, HasValue, Validatable, Displayable, ParameterKind,
};

/// Parameter for color selection
#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct ColorParameter {
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ColorParameterOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

#[derive(Debug, Clone, Builder, Serialize, Deserialize)]
pub struct ColorParameterOptions {
    /// Color format: "hex", "rgb", "hsl", "hsv"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ColorFormat>,

    /// Whether to show an alpha/opacity channel
    #[serde(default)]
    pub allow_alpha: bool,

    /// Predefined color palette
    #[serde(skip_serializing_if = "Option::is_none")]
    pub palette: Option<Vec<String>>,

    /// Show color name alongside value
    #[serde(default)]
    pub show_color_name: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ColorFormat {
    #[serde(rename = "hex")]
    Hex,
    #[serde(rename = "rgb")]
    Rgb,
    #[serde(rename = "hsl")]
    Hsl,
    #[serde(rename = "hsv")]
    Hsv,
}

impl Default for ColorFormat {
    fn default() -> Self {
        ColorFormat::Hex
    }
}

impl ParameterType for ColorParameter {
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

impl HasValue for ColorParameter {
    type Value = String;

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
        self.value.as_ref().map(|s| ParameterValue::Value(nebula_value::Value::String(s.clone().into())))
    }

    fn set_parameter_value(&mut self, value: ParameterValue) -> Result<(), ParameterError> {
        match value {
            ParameterValue::Value(nebula_value::Value::String(s)) => {
                // Validate color format
                if self.is_valid_color(&s.to_string()) {
                    self.value = Some(s.to_string());
                    Ok(())
                } else {
                    Err(ParameterError::InvalidValue {
                        key: self.metadata.key.clone(),
                        reason: format!("Invalid color format: {}", s),
                    })
                }
            },
            ParameterValue::Expression(expr) => {
                // Allow expressions for dynamic colors
                self.value = Some(expr);
                Ok(())
            },
            _ => Err(ParameterError::InvalidValue {
                key: self.metadata.key.clone(),
                reason: "Expected string value for color".to_string(),
            }),
        }
    }
}

impl Validatable for ColorParameter {
    fn validation(&self) -> Option<&ParameterValidation> {
        self.validation.as_ref()
    }

    fn value_to_json(&self, value: &Self::Value) -> serde_json::Value {
        serde_json::Value::String(value.clone())
    }

    fn is_empty_value(&self, value: &Self::Value) -> bool {
        value.is_empty()
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

        let format = self.options.as_ref()
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
        color.starts_with("rgb(") && color.ends_with(")")
            || color.starts_with("rgba(") && color.ends_with(")")
    }

    /// Check if string is valid HSL color (hsl(h,s%,l%) or hsla(h,s%,l%,a))
    fn is_valid_hsl_color(&self, color: &str) -> bool {
        color.starts_with("hsl(") && color.ends_with(")")
            || color.starts_with("hsla(") && color.ends_with(")")
    }

    /// Check if string is valid HSV color
    fn is_valid_hsv_color(&self, color: &str) -> bool {
        color.starts_with("hsv(") && color.ends_with(")")
            || color.starts_with("hsva(") && color.ends_with(")")
    }

    /// Convert color to specified format (basic implementation)
    pub fn convert_to_format(&self, format: ColorFormat) -> Option<String> {
        let current_value = self.value.as_ref()?;

        // This is a simplified implementation
        // In a real application, you'd use a proper color conversion library
        match format {
            ColorFormat::Hex if !current_value.starts_with('#') => {
                Some(format!("#{}", current_value))
            }
            _ => Some(current_value.clone()),
        }
    }
}