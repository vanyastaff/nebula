//! Select dropdown widget for SelectParameter

use egui::{ComboBox, Response, Ui};
use nebula_parameter::{HasValue, Parameter, SelectParameter};

use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering SelectParameter
pub struct SelectWidget {
    parameter: SelectParameter,
    changed: bool,
}

impl SelectWidget {
    /// Create a new select widget from a parameter
    pub fn new(parameter: SelectParameter) -> Self {
        Self {
            parameter,
            changed: false,
        }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &SelectParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut SelectParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> SelectParameter {
        self.parameter
    }
}

impl ParameterWidget for SelectWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let key = metadata.key.clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.has_value();
        
        // Get current value
        let current_value = self.parameter.get()
            .map(|v| v.as_str().to_string());
        
        // Get options (clone them to avoid borrow issues)
        let options = self.parameter.options.clone();
        
        let selected_text = current_value
            .as_ref()
            .and_then(|val| {
                options.iter()
                    .find(|opt| &opt.value == val)
                    .map(|opt| opt.name.clone())
            })
            .unwrap_or_else(|| "Select...".to_string());
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            let mut selected_value = current_value.clone();
            
            let response = ComboBox::from_id_salt(&key)
                .selected_text(&selected_text)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    let mut changed = false;
                    for option in &options {
                        let is_selected = selected_value.as_ref() == Some(&option.value);
                        if ui.selectable_label(is_selected, &option.name).clicked() {
                            selected_value = Some(option.value.clone());
                            changed = true;
                        }
                    }
                    changed
                });
            
            if response.inner.unwrap_or(false) {
                if let Some(val) = selected_value {
                    if let Err(e) = self.parameter.set(nebula_value::Text::from(val)) {
                        eprintln!("Failed to set select parameter: {}", e);
                    } else {
                        self.changed = true;
                    }
                }
            }
            
            response.response
        })
    }

    fn has_changed(&self) -> bool {
        self.changed
    }

    fn reset_changed(&mut self) {
        self.changed = false;
    }
}
