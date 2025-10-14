//! Theme and styling for parameter widgets

use egui::{Color32, FontId, CornerRadius, Stroke};

/// Visual theme for parameter widgets
#[derive(Debug, Clone)]
pub struct ParameterTheme {
    /// Colors used in the theme
    pub colors: ThemeColors,
    
    /// Fonts used in the theme
    pub fonts: ThemeFonts,
    
    /// Spacing values
    pub spacing: ThemeSpacing,
    
    /// Visual styling
    pub visuals: ThemeVisuals,
}

impl Default for ParameterTheme {
    fn default() -> Self {
        Self::dark()
    }
}

impl ParameterTheme {
    /// Create a dark theme
    pub fn dark() -> Self {
        Self {
            colors: ThemeColors {
                label: Color32::from_rgb(230, 230, 230),
                required: Color32::from_rgb(255, 100, 100),
                description: Color32::from_rgb(160, 160, 160),
                hint: Color32::from_rgb(120, 140, 180),
                error: Color32::from_rgb(240, 80, 80),
                success: Color32::from_rgb(80, 200, 120),
                warning: Color32::from_rgb(255, 180, 60),
                placeholder: Color32::from_rgb(120, 120, 120),
                disabled: Color32::from_rgb(100, 100, 100),
                border: Color32::from_rgb(60, 60, 70),
                border_focused: Color32::from_rgb(100, 140, 200),
                border_error: Color32::from_rgb(200, 60, 60),
                background: Color32::from_rgb(35, 35, 40),
                background_hover: Color32::from_rgb(45, 45, 50),
                info: Color32::from_rgb(100, 150, 255),
            },
            fonts: ThemeFonts {
                label: FontId::proportional(14.0),
                description: FontId::proportional(12.0),
                hint: FontId::proportional(11.0),
                error: FontId::proportional(11.0),
                input: FontId::proportional(13.0),
            },
            spacing: ThemeSpacing {
                field_spacing: 12.0,
                label_spacing: 4.0,
                description_spacing: 2.0,
                hint_spacing: 2.0,
                error_spacing: 2.0,
                group_padding: 8.0,
                input_padding: 6.0,
            },
            visuals: ThemeVisuals {
                rounding: CornerRadius::same(4),
                input_rounding: CornerRadius::same(4),
                border_width: 1.0,
                focused_border_width: 2.0,
            },
        }
    }
    
    /// Create a light theme
    pub fn light() -> Self {
        Self {
            colors: ThemeColors {
                label: Color32::from_rgb(30, 30, 30),
                required: Color32::from_rgb(200, 50, 50),
                description: Color32::from_rgb(100, 100, 100),
                hint: Color32::from_rgb(70, 90, 140),
                error: Color32::from_rgb(200, 40, 40),
                success: Color32::from_rgb(40, 160, 80),
                warning: Color32::from_rgb(200, 140, 20),
                placeholder: Color32::from_rgb(140, 140, 140),
                disabled: Color32::from_rgb(160, 160, 160),
                border: Color32::from_rgb(200, 200, 200),
                border_focused: Color32::from_rgb(80, 140, 220),
                border_error: Color32::from_rgb(220, 80, 80),
                background: Color32::from_rgb(250, 250, 250),
                background_hover: Color32::from_rgb(240, 240, 245),
                info: Color32::from_rgb(80, 120, 200),
            },
            fonts: ThemeFonts {
                label: FontId::proportional(14.0),
                description: FontId::proportional(12.0),
                hint: FontId::proportional(11.0),
                error: FontId::proportional(11.0),
                input: FontId::proportional(13.0),
            },
            spacing: ThemeSpacing {
                field_spacing: 12.0,
                label_spacing: 4.0,
                description_spacing: 2.0,
                hint_spacing: 2.0,
                error_spacing: 2.0,
                group_padding: 8.0,
                input_padding: 6.0,
            },
            visuals: ThemeVisuals {
                rounding: CornerRadius::same(4),
                input_rounding: CornerRadius::same(4),
                border_width: 1.0,
                focused_border_width: 2.0,
            },
        }
    }
}

/// Color palette for the theme
#[derive(Debug, Clone)]
pub struct ThemeColors {
    /// Label text color
    pub label: Color32,
    /// Required field indicator color
    pub required: Color32,
    /// Description text color
    pub description: Color32,
    /// Hint text color
    pub hint: Color32,
    /// Error message color
    pub error: Color32,
    /// Success message color
    pub success: Color32,
    /// Warning message color
    pub warning: Color32,
    /// Placeholder text color
    pub placeholder: Color32,
    /// Disabled element color
    pub disabled: Color32,
    /// Default border color
    pub border: Color32,
    /// Focused border color
    pub border_focused: Color32,
    /// Error border color
    pub border_error: Color32,
    /// Background color
    pub background: Color32,
    /// Hover background color
    pub background_hover: Color32,
    /// Info message color
    pub info: Color32,
}

/// Font configuration
#[derive(Debug, Clone)]
pub struct ThemeFonts {
    /// Label font
    pub label: FontId,
    /// Description font
    pub description: FontId,
    /// Hint font
    pub hint: FontId,
    /// Error message font
    pub error: FontId,
    /// Input field font
    pub input: FontId,
}

/// Spacing configuration
#[derive(Debug, Clone)]
pub struct ThemeSpacing {
    /// Space between fields
    pub field_spacing: f32,
    /// Space after label
    pub label_spacing: f32,
    /// Space before/after description
    pub description_spacing: f32,
    /// Space before/after hint
    pub hint_spacing: f32,
    /// Space before/after error
    pub error_spacing: f32,
    /// Padding inside groups
    pub group_padding: f32,
    /// Padding inside inputs
    pub input_padding: f32,
}

/// Visual styling configuration
#[derive(Debug, Clone)]
pub struct ThemeVisuals {
    /// Border rounding
    pub rounding: CornerRadius,
    /// Input field rounding
    pub input_rounding: CornerRadius,
    /// Border width
    pub border_width: f32,
    /// Focused border width
    pub focused_border_width: f32,
}

/// Helper to get stroke for different states
impl ThemeVisuals {
    /// Get stroke for normal state
    pub fn normal_stroke(&self, colors: &ThemeColors) -> Stroke {
        Stroke::new(self.border_width, colors.border)
    }
    
    /// Get stroke for focused state
    pub fn focused_stroke(&self, colors: &ThemeColors) -> Stroke {
        Stroke::new(self.focused_border_width, colors.border_focused)
    }
    
    /// Get stroke for error state
    pub fn error_stroke(&self, colors: &ThemeColors) -> Stroke {
        Stroke::new(self.focused_border_width, colors.border_error)
    }
}

