//! Checkbox widget for CheckboxParameter

use egui::{Response, Ui};
use nebula_parameter::{CheckboxParameter, HasValue, Parameter};

use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering CheckboxParameter
pub struct CheckboxWidget {
    parameter: CheckboxParameter,
    changed: bool,
}

impl CheckboxWidget {
    /// Create a new checkbox widget from a parameter
    pub fn new(parameter: CheckboxParameter) -> Self {
        Self {
            parameter,
            changed: false,
        }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &CheckboxParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut CheckboxParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> CheckboxParameter {
        self.parameter
    }
}

impl ParameterWidget for CheckboxWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.has_value();
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            let mut checked = self.parameter.get()
                .map(|b| b.value())
                .unwrap_or(false);
            
            let response = ui.checkbox(&mut checked, &metadata.name);
            
            if response.changed() {
                if let Err(e) = self.parameter.set(nebula_value::Boolean::from(checked)) {
                    eprintln!("Failed to set checkbox parameter: {}", e);
                } else {
                    self.changed = true;
                }
            }
            
            // Help text from options
            if let Some(options) = &self.parameter.options {
                if let Some(help_text) = &options.help_text {
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(help_text).italics().size(11.0));
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
