//! Textarea widget for TextareaParameter

use egui::{Response, TextEdit, Ui};
use nebula_parameter::{HasValue, Parameter, TextareaParameter};

use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering TextareaParameter
pub struct TextareaWidget {
    parameter: TextareaParameter,
    changed: bool,
}

impl TextareaWidget {
    /// Create a new textarea widget from a parameter
    pub fn new(parameter: TextareaParameter) -> Self {
        Self {
            parameter,
            changed: false,
        }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &TextareaParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut TextareaParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> TextareaParameter {
        self.parameter
    }
}

impl ParameterWidget for TextareaWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.has_value();
        
        // Get placeholder from metadata or use default
        let placeholder = metadata.placeholder.as_deref().unwrap_or("Enter text...");
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            let mut text = self.parameter.get()
                .map(|t| t.as_str().to_string())
                .unwrap_or_default();
            
            let response = ui.add(
                TextEdit::multiline(&mut text)
                    .desired_rows(5)
                    .desired_width(f32::INFINITY)
                    .hint_text(placeholder)
            );
            
            if response.changed() {
                if let Err(e) = self.parameter.set(nebula_value::Text::from(text)) {
                    eprintln!("Failed to set textarea parameter: {}", e);
                } else {
                    self.changed = true;
                }
            }
            
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
