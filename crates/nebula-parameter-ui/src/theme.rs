//! Theme configuration for parameter widgets.

use egui::Color32;

/// Theme configuration for parameter widgets.
#[derive(Debug, Clone)]
pub struct ParameterTheme {
    // === Colors ===
    /// Primary accent color
    pub primary: Color32,
    /// Secondary accent color
    pub secondary: Color32,
    /// Success/valid state color
    pub success: Color32,
    /// Warning state color
    pub warning: Color32,
    /// Error/invalid state color
    pub error: Color32,
    /// Info notice color
    pub info: Color32,
    /// Background color for inputs
    pub input_bg: Color32,
    /// Border color for inputs
    pub input_border: Color32,
    /// Border color for focused inputs
    pub input_border_focused: Color32,
    /// Text color for labels
    pub label_color: Color32,
    /// Text color for hints/descriptions
    pub hint_color: Color32,
    /// Text color for placeholders
    pub placeholder_color: Color32,
    /// Disabled state color
    pub disabled: Color32,
    /// Surface/panel background color
    pub surface: Color32,

    // === Sizing ===
    /// Default height for controls (buttons, inputs)
    pub control_height: f32,
    /// Border radius for inputs and controls
    pub border_radius: f32,
    /// Padding inside inputs
    pub input_padding: f32,
    /// Border width for focused/error inputs
    pub input_border_width_focused: f32,
    /// Height of slider track
    pub slider_height: f32,
    /// Size of slider handle
    pub slider_handle_size: f32,
    /// Width for drag value controls
    pub drag_value_width: f32,
    /// Width for text input in SliderText mode
    pub slider_text_input_width: f32,

    // === Typography ===
    /// Font size for labels
    pub label_font_size: f32,
    /// Font size for input text
    pub input_font_size: f32,
    /// Font size for hints and descriptions
    pub hint_font_size: f32,

    // === Spacing (consistent units based on 4px) ===
    /// Extra small spacing (4px)
    pub spacing_xs: f32,
    /// Small spacing (8px)
    pub spacing_sm: f32,
    /// Medium spacing (12px)
    pub spacing_md: f32,
    /// Large spacing (16px)
    pub spacing_lg: f32,
    /// Extra large spacing (24px)
    pub spacing_xl: f32,

    /// Legacy spacing field (use spacing_sm instead)
    pub spacing: f32,
}

impl Default for ParameterTheme {
    fn default() -> Self {
        Self::light()
    }
}

impl ParameterTheme {
    /// Light theme preset.
    #[must_use]
    pub fn light() -> Self {
        Self {
            // Colors
            primary: Color32::from_rgb(64, 64, 64),
            secondary: Color32::from_rgb(107, 114, 128),
            success: Color32::from_rgb(34, 197, 94),
            warning: Color32::from_rgb(234, 179, 8),
            error: Color32::from_rgb(220, 60, 60),
            info: Color32::from_rgb(100, 100, 100),
            input_bg: Color32::WHITE,
            input_border: Color32::from_rgb(200, 200, 200),
            input_border_focused: Color32::from_rgb(100, 100, 100),
            label_color: Color32::from_rgb(30, 30, 30),
            hint_color: Color32::from_rgb(120, 120, 120),
            placeholder_color: Color32::from_rgb(160, 160, 160),
            disabled: Color32::from_rgb(230, 230, 230),
            surface: Color32::from_rgb(245, 245, 245),

            // Sizing
            control_height: 28.0,
            border_radius: 4.0,
            input_padding: 8.0,
            input_border_width_focused: 1.5,
            slider_height: 4.0,
            slider_handle_size: 16.0,
            drag_value_width: 200.0,
            slider_text_input_width: 60.0,

            // Typography
            label_font_size: 14.0,
            input_font_size: 14.0,
            hint_font_size: 12.0,

            // Spacing
            spacing_xs: 4.0,
            spacing_sm: 8.0,
            spacing_md: 12.0,
            spacing_lg: 16.0,
            spacing_xl: 24.0,
            spacing: 8.0,
        }
    }

    /// Dark theme preset (Fluent Dark).
    #[must_use]
    pub fn dark() -> Self {
        Self {
            // Colors - Fluent Dark
            primary: Color32::from_rgb(96, 205, 245), // #60CDF5 Fluent Blue
            secondary: Color32::from_rgb(150, 150, 150),
            success: Color32::from_rgb(74, 222, 128),
            warning: Color32::from_rgb(250, 204, 21),
            error: Color32::from_rgb(248, 113, 113),
            info: Color32::from_rgb(180, 180, 180),
            input_bg: Color32::from_rgb(43, 43, 43), // #2B2B2B
            input_border: Color32::from_rgb(80, 80, 80), // #505050
            input_border_focused: Color32::from_rgb(96, 205, 245), // #60CDF5
            label_color: Color32::from_rgb(255, 255, 255), // #FFFFFF
            hint_color: Color32::from_rgb(180, 180, 180), // #B4B4B4
            placeholder_color: Color32::from_rgb(128, 128, 128), // #808080
            disabled: Color32::from_rgb(128, 128, 128), // #808080
            surface: Color32::from_rgb(45, 45, 45),  // #2D2D2D

            // Sizing
            control_height: 32.0,
            border_radius: 4.0,
            input_padding: 8.0,
            input_border_width_focused: 1.5,
            slider_height: 4.0,
            slider_handle_size: 16.0,
            drag_value_width: 200.0,
            slider_text_input_width: 60.0,

            // Typography
            label_font_size: 12.0,
            input_font_size: 14.0,
            hint_font_size: 11.0,

            // Spacing
            spacing_xs: 4.0,
            spacing_sm: 8.0,
            spacing_md: 12.0,
            spacing_lg: 16.0,
            spacing_xl: 24.0,
            spacing: 8.0,
        }
    }

    /// Create an input frame with consistent styling.
    /// Use `focused` to indicate focus state, `error` for validation errors.
    #[must_use]
    pub fn input_frame(&self, focused: bool, error: bool) -> egui::Frame {
        let (border_color, border_width) = if error {
            (self.error, self.input_border_width_focused)
        } else if focused {
            (self.input_border_focused, self.input_border_width_focused)
        } else {
            (self.input_border, 1.0)
        };

        egui::Frame::new()
            .fill(self.input_bg)
            .stroke(egui::Stroke::new(border_width, border_color))
            .corner_radius(self.border_radius)
            .inner_margin(egui::Margin::symmetric(self.input_padding as i8, 6))
    }

    /// Create a popup/dropdown frame with consistent styling.
    #[must_use]
    pub fn popup_frame(&self) -> egui::Frame {
        egui::Frame::new()
            .fill(self.surface)
            .stroke(egui::Stroke::new(1.0, self.input_border))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::same(12))
            .shadow(egui::epaint::Shadow {
                offset: [0, 2],
                blur: 8,
                spread: 0,
                color: Color32::from_black_alpha(40),
            })
    }

    /// Get color for notice type.
    #[must_use]
    pub fn notice_color(&self, notice_type: &nebula_parameter::types::NoticeType) -> Color32 {
        use nebula_parameter::types::NoticeType;
        match notice_type {
            NoticeType::Info => self.info,
            NoticeType::Warning => self.warning,
            NoticeType::Error => self.error,
            NoticeType::Success => self.success,
        }
    }

    /// Get background color for notice type (lighter version).
    #[must_use]
    pub fn notice_bg_color(&self, notice_type: &nebula_parameter::types::NoticeType) -> Color32 {
        let color = self.notice_color(notice_type);
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 25)
    }
}
