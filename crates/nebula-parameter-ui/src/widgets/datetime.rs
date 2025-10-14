//! DateTime picker widget for DateTimeParameter

use egui::{Response, TextEdit, Ui};
use nebula_parameter::{DateTimeParameter, Parameter, HasValue};
use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering DateTimeParameter
pub struct DateTimeWidget {
    parameter: DateTimeParameter,
    changed: bool,
}

impl DateTimeWidget {
    /// Create a new datetime widget from a parameter
    pub fn new(parameter: DateTimeParameter) -> Self {
        Self { parameter, changed: false }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &DateTimeParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut DateTimeParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> DateTimeParameter {
        self.parameter
    }
}

impl ParameterWidget for DateTimeWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.get().is_some();
        
        let placeholder = metadata.placeholder
            .as_deref()
            .unwrap_or("YYYY-MM-DD HH:MM:SS");
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            let mut datetime_str = self.parameter.get()
                .map(|d| d.as_str().to_string())
                .unwrap_or_default();
            
            let response = ui.add(
                TextEdit::singleline(&mut datetime_str)
                    .hint_text(placeholder)
                    .desired_width(f32::INFINITY)
            );
            
            if response.changed() {
                if let Err(e) = self.parameter.set(nebula_value::Text::from(datetime_str)) {
                    eprintln!("Failed to set datetime parameter: {}", e);
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
