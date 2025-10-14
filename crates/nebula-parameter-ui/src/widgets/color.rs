//! Color picker widget for ColorParameter

use egui::{Color32, Response, Ui};
use nebula_parameter::{ColorParameter, Parameter, HasValue};
use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering ColorParameter
pub struct ColorWidget {
    parameter: ColorParameter,
    changed: bool,
}

impl ColorWidget {
    /// Create a new color widget from a parameter
    pub fn new(parameter: ColorParameter) -> Self {
        Self { parameter, changed: false }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &ColorParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut ColorParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> ColorParameter {
        self.parameter
    }
}

impl ParameterWidget for ColorWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.get().is_some();
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            // Get current color or default to black
            let current_color = self.parameter.get()
                .map(|c| parse_color_string(c.as_str()))
                .unwrap_or(Color32::BLACK);
            
            let mut color = current_color;
            
            // Color picker - horizontal layout with full width
            let response = ui.horizontal(|ui| {
                let resp = ui.color_edit_button_srgba(&mut color);
                ui.allocate_space(ui.available_size());
                resp
            }).inner;
            
            if response.changed() && color != current_color {
                let color_string = format_color(color);
                if let Err(e) = self.parameter.set(nebula_value::Text::from(color_string)) {
                    eprintln!("Failed to set color parameter: {}", e);
                } else {
                    self.changed = true;
                }
            }
            
            // Show hex value
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Hex:");
                ui.label(format_color(color));
            });
            
            response
        })
    }

    fn has_changed(&self) -> bool {
        self.changed
    }

    fn reset_changed(&mut self) {
        self.changed = false;
    }
}

/// Parse a color string (hex format) to Color32
fn parse_color_string(s: &str) -> Color32 {
    let s = s.trim().trim_start_matches('#');
    
    if s.len() == 6 {
        // RGB format: #RRGGBB
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&s[0..2], 16),
            u8::from_str_radix(&s[2..4], 16),
            u8::from_str_radix(&s[4..6], 16),
        ) {
            return Color32::from_rgb(r, g, b);
        }
    } else if s.len() == 8 {
        // RGBA format: #RRGGBBAA
        if let (Ok(r), Ok(g), Ok(b), Ok(a)) = (
            u8::from_str_radix(&s[0..2], 16),
            u8::from_str_radix(&s[2..4], 16),
            u8::from_str_radix(&s[4..6], 16),
            u8::from_str_radix(&s[6..8], 16),
        ) {
            return Color32::from_rgba_unmultiplied(r, g, b, a);
        }
    }
    
    // Fallback to black if parsing fails
    Color32::BLACK
}

/// Format a Color32 to hex string
fn format_color(color: Color32) -> String {
    if color.a() == 255 {
        // RGB format
        format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b())
    } else {
        // RGBA format
        format!("#{:02X}{:02X}{:02X}{:02X}", color.r(), color.g(), color.b(), color.a())
    }
}
