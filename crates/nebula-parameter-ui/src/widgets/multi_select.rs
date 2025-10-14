//! Multi-select widget for MultiSelectParameter

use egui::{Response, Ui};
use nebula_parameter::{MultiSelectParameter, Parameter, HasValue};
use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering MultiSelectParameter
pub struct MultiSelectWidget {
    parameter: MultiSelectParameter,
    changed: bool,
}

impl MultiSelectWidget {
    /// Create a new multi-select widget from a parameter
    pub fn new(parameter: MultiSelectParameter) -> Self {
        Self { parameter, changed: false }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &MultiSelectParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut MultiSelectParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> MultiSelectParameter {
        self.parameter
    }
}

impl ParameterWidget for MultiSelectWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.get().is_some();
        
        // Get current selected values
        let current_values = self.parameter.get()
            .cloned()
            .unwrap_or_default();
        
        // Get options (clone them to avoid borrow issues)
        let options = self.parameter.options.clone();
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            let mut selected_values = current_values.clone();
            let mut response = ui.label("");  // Placeholder
            let mut any_changed = false;
            
            for option in &options {
                let is_selected = selected_values.contains(&option.value);
                let mut checked = is_selected;
                
                let checkbox_response = ui.checkbox(&mut checked, &option.name);
                
                if checkbox_response.changed() {
                    if checked && !is_selected {
                        // Add to selection
                        selected_values.push(option.value.clone());
                        any_changed = true;
                    } else if !checked && is_selected {
                        // Remove from selection
                        selected_values.retain(|v| v != &option.value);
                        any_changed = true;
                    }
                    response = checkbox_response;
                }
                
                // Add description if available
                if let Some(desc) = &option.description {
                    ui.add_space(2.0);
                    ui.indent(option.name.clone(), |ui| {
                        ui.label(egui::RichText::new(desc.as_ref()).italics().size(11.0));
                    });
                }
                
                ui.add_space(4.0);
            }
            
            if any_changed {
                if let Err(e) = self.parameter.set(selected_values) {
                    eprintln!("Failed to set multi-select parameter: {}", e);
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
