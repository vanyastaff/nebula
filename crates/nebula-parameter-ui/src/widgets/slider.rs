//! Slider widget for NumberParameter with slider display

use egui::{Response, Slider, Ui};
use nebula_parameter::{NumberParameter, Parameter, HasValue};
use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering NumberParameter as a slider
pub struct SliderWidget {
    parameter: NumberParameter,
    changed: bool,
}

impl SliderWidget {
    /// Create a new slider widget from a parameter
    pub fn new(parameter: NumberParameter) -> Self {
        Self { parameter, changed: false }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &NumberParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut NumberParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> NumberParameter {
        self.parameter
    }
}

impl ParameterWidget for SliderWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.get().is_some();
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            let mut value = self.parameter.get().copied().unwrap_or(0.0);
            
            // Get min/max from options or use defaults
            let (min, max) = if let Some(options) = &self.parameter.options {
                (
                    options.min.unwrap_or(0.0),
                    options.max.unwrap_or(100.0)
                )
            } else {
                (0.0, 100.0)
            };
            
            // Create slider with range
            let slider = Slider::new(&mut value, min..=max)
                .text(self.parameter.metadata().name.as_str());
            
            let response = ui.add_sized([ui.available_width(), 0.0], slider);
            
            if response.changed() {
                if let Err(e) = self.parameter.set(value) {
                    eprintln!("Failed to set slider parameter: {}", e);
                } else {
                    self.changed = true;
                }
            }
            
            // Show current value
            ui.add_space(2.0);
            ui.label(format!("Value: {:.2}", value));
            
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
