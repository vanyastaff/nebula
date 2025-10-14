//! Number input widget for NumberParameter

use egui::{DragValue, Response, Ui};
use nebula_parameter::{HasValue, NumberParameter, Parameter};

use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering NumberParameter
pub struct NumberWidget {
    parameter: NumberParameter,
    changed: bool,
}

impl NumberWidget {
    /// Create a new number widget from a parameter
    pub fn new(parameter: NumberParameter) -> Self {
        Self {
            parameter,
            changed: false,
        }
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

impl ParameterWidget for NumberWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.has_value();
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            let mut value = self.parameter.get().copied().unwrap_or(0.0);
            
            let mut drag = DragValue::new(&mut value);
            
            // Apply options if available
            if let Some(options) = &self.parameter.options {
                if let Some(min) = options.min {
                    drag = drag.range(min..=options.max.unwrap_or(f64::MAX));
                }
                if let Some(max) = options.max {
                    if options.min.is_none() {
                        drag = drag.range(f64::MIN..=max);
                    }
                }
                if let Some(speed) = options.step {
                    drag = drag.speed(speed);
                }
            }
            
            let response = ui.add_sized([ui.available_width(), 0.0], drag);
            
            if response.changed() {
                if let Err(e) = self.parameter.set(value) {
                    eprintln!("Failed to set number parameter: {}", e);
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
