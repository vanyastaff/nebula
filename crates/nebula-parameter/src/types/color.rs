//! Color parameter type for color selection

use serde::{Deserialize, Serialize};

use crate::core::{
    Describable, Displayable, ParameterDisplay, ParameterError, ParameterKind, ParameterMetadata,
    ParameterValidation, Validatable,
};
use nebula_value::{Value, ValueKind};

/// Parameter for color selection
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_parameter::prelude::*;
///
/// let param = ColorParameter::builder()
///     .key("bg_color")
///     .name("Background Color")
///     .description("Choose a background color")
///     .default("#ffffff")
///     .options(
///         ColorParameterOptions::builder()
///             .format(ColorFormat::Hex)
///             .allow_alpha(true)
///             .build()
///     )
///     .build()?;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorParameter {
    /// Parameter metadata (key, name, description, etc.)
    #[serde(flatten)]
    pub metadata: ParameterMetadata,

    /// Default value if parameter is not set
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<nebula_value::Text>,

    /// Configuration options for this parameter type
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ColorParameterOptions>,

    /// Display conditions controlling when this parameter is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ParameterDisplay>,

    /// Validation rules for this parameter
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ParameterValidation>,
}

/// Configuration options for color parameters
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColorParameterOptions {
    /// Color format: "hex", "rgb", "hsl", "hsv"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ColorFormat>,

    /// Whether to show an alpha/opacity channel
    #[serde(default)]
    pub allow_alpha: bool,

    /// Predefined color palette
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<Vec<String>>,
}

/// Color format types
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

// =============================================================================
// ColorParameter Builder
// =============================================================================

/// Builder for `ColorParameter`
#[derive(Debug, Default)]
pub struct ColorParameterBuilder {
    // Metadata fields
    key: Option<String>,
    name: Option<String>,
    description: String,
    required: bool,
    placeholder: Option<String>,
    hint: Option<String>,
    // Parameter fields
    default: Option<nebula_value::Text>,
    options: Option<ColorParameterOptions>,
    display: Option<ParameterDisplay>,
    validation: Option<ParameterValidation>,
}

impl ColorParameter {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> ColorParameterBuilder {
        ColorParameterBuilder::new()
    }
}

impl ColorParameterBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            key: None,
            name: None,
            description: String::new(),
            required: false,
            placeholder: None,
            hint: None,
            default: None,
            options: None,
            display: None,
            validation: None,
        }
    }

    // -------------------------------------------------------------------------
    // Metadata methods
    // -------------------------------------------------------------------------

    /// Set the parameter key (required)
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Set the display name (required)
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the description
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set whether the parameter is required
    #[must_use]
    pub fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    /// Set placeholder text
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    /// Set hint text
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    // -------------------------------------------------------------------------
    // Parameter-specific methods
    // -------------------------------------------------------------------------

    /// Set the default value
    #[must_use]
    pub fn default(mut self, default: impl Into<nebula_value::Text>) -> Self {
        self.default = Some(default.into());
        self
    }

    /// Set the options
    #[must_use]
    pub fn options(mut self, options: ColorParameterOptions) -> Self {
        self.options = Some(options);
        self
    }

    /// Set display conditions
    #[must_use]
    pub fn display(mut self, display: ParameterDisplay) -> Self {
        self.display = Some(display);
        self
    }

    /// Set validation rules
    #[must_use]
    pub fn validation(mut self, validation: ParameterValidation) -> Self {
        self.validation = Some(validation);
        self
    }

    // -------------------------------------------------------------------------
    // Build
    // -------------------------------------------------------------------------

    /// Build the `ColorParameter`
    ///
    /// # Errors
    ///
    /// Returns error if required fields are missing or key format is invalid.
    pub fn build(self) -> Result<ColorParameter, ParameterError> {
        let metadata = ParameterMetadata::builder()
            .key(
                self.key
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "key".into(),
                    })?,
            )
            .name(
                self.name
                    .ok_or_else(|| ParameterError::BuilderMissingField {
                        field: "name".into(),
                    })?,
            )
            .description(self.description)
            .required(self.required)
            .build()?;

        let mut metadata = metadata;
        metadata.placeholder = self.placeholder;
        metadata.hint = self.hint;

        Ok(ColorParameter {
            metadata,
            default: self.default,
            options: self.options,
            display: self.display,
            validation: self.validation,
        })
    }
}

// =============================================================================
// ColorParameterOptions Builder
// =============================================================================

/// Builder for `ColorParameterOptions`
#[derive(Debug, Default)]
pub struct ColorParameterOptionsBuilder {
    format: Option<ColorFormat>,
    allow_alpha: bool,
    palette: Option<Vec<String>>,
}

impl ColorParameterOptions {
    /// Create a new builder
    #[must_use]
    pub fn builder() -> ColorParameterOptionsBuilder {
        ColorParameterOptionsBuilder::default()
    }
}

impl ColorParameterOptionsBuilder {
    /// Set color format
    #[must_use]
    pub fn format(mut self, format: ColorFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Set whether to allow alpha channel
    #[must_use]
    pub fn allow_alpha(mut self, allow_alpha: bool) -> Self {
        self.allow_alpha = allow_alpha;
        self
    }

    /// Set predefined color palette
    #[must_use]
    pub fn palette(mut self, palette: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.palette = Some(palette.into_iter().map(Into::into).collect());
        self
    }

    /// Build the options
    #[must_use]
    pub fn build(self) -> ColorParameterOptions {
        ColorParameterOptions {
            format: self.format,
            allow_alpha: self.allow_alpha,
            palette: self.palette,
        }
    }
}

// =============================================================================
// Trait Implementations
// =============================================================================

impl Describable for ColorParameter {
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
    fn expected_kind(&self) -> Option<ValueKind> {
        Some(ValueKind::String)
    }

    fn validate_sync(&self, value: &Value) -> Result<(), ParameterError> {
        // Type check
        if let Some(expected) = self.expected_kind() {
            let actual = value.kind();
            if actual != ValueKind::Null && actual != expected {
                return Err(ParameterError::InvalidType {
                    key: self.metadata.key.clone(),
                    expected_type: expected.name().to_string(),
                    actual_details: actual.name().to_string(),
                });
            }
        }

        // Required check
        if self.is_required() && self.is_empty(value) {
            return Err(ParameterError::MissingValue {
                key: self.metadata.key.clone(),
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
        value.is_null() || value.as_text().is_some_and(|s| s.is_empty())
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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_parameter_builder() {
        let param = ColorParameter::builder()
            .key("bg_color")
            .name("Background Color")
            .description("Choose a background color")
            .required(true)
            .build()
            .unwrap();

        assert_eq!(param.metadata.key.as_str(), "bg_color");
        assert_eq!(param.metadata.name, "Background Color");
        assert!(param.metadata.required);
    }

    #[test]
    fn test_color_parameter_with_options() {
        let param = ColorParameter::builder()
            .key("theme_color")
            .name("Theme Color")
            .default("#3366ff")
            .options(
                ColorParameterOptions::builder()
                    .format(ColorFormat::Hex)
                    .allow_alpha(true)
                    .palette(["#ff0000", "#00ff00", "#0000ff"])
                    .build(),
            )
            .build()
            .unwrap();

        let opts = param.options.unwrap();
        assert_eq!(opts.format, Some(ColorFormat::Hex));
        assert!(opts.allow_alpha);
        assert_eq!(opts.palette.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_color_parameter_missing_key() {
        let result = ColorParameter::builder().name("Test").build();

        assert!(matches!(
            result,
            Err(ParameterError::BuilderMissingField { field }) if field == "key"
        ));
    }

    #[test]
    fn test_color_validation() {
        let param = ColorParameter::builder()
            .key("color")
            .name("Color")
            .build()
            .unwrap();

        assert!(param.is_valid_color("#fff"));
        assert!(param.is_valid_color("#ffffff"));
        assert!(param.is_valid_color("#ffffffff"));
        assert!(!param.is_valid_color("invalid"));
    }
}
