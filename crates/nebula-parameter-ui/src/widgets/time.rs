//! Time picker widget for TimeParameter

use egui::{Response, TextEdit, Ui};
use nebula_parameter::{Parameter, TimeParameter, HasValue};
use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering TimeParameter
pub struct TimeWidget {
    parameter: TimeParameter,
    changed: bool,
}

impl TimeWidget {
    /// Create a new time widget from a parameter
    pub fn new(parameter: TimeParameter) -> Self {
        Self { parameter, changed: false }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &TimeParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut TimeParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> TimeParameter {
        self.parameter
    }
}

impl ParameterWidget for TimeWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.get().is_some();
        
        let placeholder = metadata.placeholder
            .as_deref()
            .unwrap_or("HH:MM:SS");
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            let mut time_str = self.parameter.get()
                .map(|t| t.as_str().to_string())
                .unwrap_or_default();
            
            let response = ui.add(
                TextEdit::singleline(&mut time_str)
                    .hint_text(placeholder)
                    .desired_width(f32::INFINITY)
            );
            
            if response.changed() {
                if let Err(e) = self.parameter.set(nebula_value::Text::from(time_str)) {
                    eprintln!("Failed to set time parameter: {}", e);
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
