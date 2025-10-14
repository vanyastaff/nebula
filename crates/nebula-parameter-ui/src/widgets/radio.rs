//! Radio button widget for RadioParameter

use egui::{Response, Ui};
use nebula_parameter::{RadioParameter, Parameter, HasValue};
use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering RadioParameter
pub struct RadioWidget {
    parameter: RadioParameter,
    changed: bool,
}

impl RadioWidget {
    /// Create a new radio widget from a parameter
    pub fn new(parameter: RadioParameter) -> Self {
        Self { parameter, changed: false }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &RadioParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut RadioParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> RadioParameter {
        self.parameter
    }
}

impl ParameterWidget for RadioWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.get().is_some();
        
        // Get current value
        let current_value = self.parameter.get()
            .map(|v| v.as_str().to_string());
        
        // Get options (clone them to avoid borrow issues)
        let options = self.parameter.options.clone();
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            let mut selected_value = current_value.clone();
            let mut response = ui.label("");  // Placeholder
            
            for option in &options {
                let is_selected = selected_value.as_ref() == Some(&option.value);
                let radio_response = ui.radio(is_selected, &option.name);
                
                if radio_response.clicked() {
                    selected_value = Some(option.value.clone());
                    response = radio_response;
                }
                
                // Add description if available
                if let Some(desc) = &option.description {
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(desc.as_ref()).italics().size(11.0));
                }
                
                ui.add_space(4.0);
            }
            
            if selected_value != current_value {
                if let Some(val) = selected_value {
                    if let Err(e) = self.parameter.set(nebula_value::Text::from(val)) {
                        eprintln!("Failed to set radio parameter: {}", e);
                    } else {
                        self.changed = true;
                    }
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
