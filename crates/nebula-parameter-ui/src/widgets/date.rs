//! Date picker widget for DateParameter

use egui::{Response, TextEdit, Ui};
use nebula_parameter::{DateParameter, Parameter, HasValue};

use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering DateParameter
pub struct DateWidget {
    parameter: DateParameter,
    changed: bool,
}

impl DateWidget {
    /// Create a new date widget from a parameter
    pub fn new(parameter: DateParameter) -> Self {
        Self {
            parameter,
            changed: false,
        }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &DateParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut DateParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> DateParameter {
        self.parameter
    }
}

impl ParameterWidget for DateWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.get().is_some();
        
        let placeholder = metadata.placeholder
            .as_deref()
            .unwrap_or("YYYY-MM-DD");
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            let mut date_str = self.parameter.get()
                .map(|d| d.to_string())
                .unwrap_or_default();
            
            let response = ui.add(
                TextEdit::singleline(&mut date_str)
                    .hint_text(placeholder)
                    .desired_width(f32::INFINITY)
            );
            
            if response.changed() {
                if let Err(e) = self.parameter.set(date_str) {
                    eprintln!("Failed to set date parameter: {}", e);
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
