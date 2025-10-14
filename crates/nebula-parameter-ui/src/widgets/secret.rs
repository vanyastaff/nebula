//! Secret input widget for SecretParameter

use egui::{Response, TextEdit, Ui};
use nebula_parameter::{SecretParameter, Parameter, HasValue};
use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering SecretParameter with password masking
pub struct SecretWidget {
    parameter: SecretParameter,
    changed: bool,
    show_secret: bool,
}

impl SecretWidget {
    /// Create a new secret widget from a parameter
    pub fn new(parameter: SecretParameter) -> Self {
        Self { 
            parameter, 
            changed: false,
            show_secret: false,
        }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &SecretParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut SecretParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> SecretParameter {
        self.parameter
    }
}

impl ParameterWidget for SecretWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        
        // For secrets, we consider having any content as "has value"
        let secret_str = self.parameter.get()
            .map(|s| s.as_str().to_string())
            .unwrap_or_default();
        let has_value = !secret_str.is_empty();
        
        // Get placeholder from metadata or use default
        let placeholder = metadata.placeholder.as_deref().unwrap_or("Enter secret...");
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            ui.horizontal(|ui| {
                let mut text = secret_str.clone();
                
                let text_edit = if self.show_secret {
                    TextEdit::singleline(&mut text)
                } else {
                    TextEdit::singleline(&mut text).password(true)
                }
                .hint_text(placeholder)
                .desired_width(f32::INFINITY);
                
                let response = ui.add(text_edit);
                
                // Toggle visibility button
                let eye_icon = if self.show_secret { "👁" } else { "🔒" };
                if ui.small_button(eye_icon).clicked() {
                    self.show_secret = !self.show_secret;
                }
                
                if response.changed() && text != secret_str {
                    if let Err(e) = self.parameter.set(nebula_value::Text::from(text)) {
                        eprintln!("Failed to set secret parameter: {}", e);
                    } else {
                        self.changed = true;
                    }
                }
                
                response
            }).inner
        })
    }

    fn has_changed(&self) -> bool {
        self.changed
    }

    fn reset_changed(&mut self) {
        self.changed = false;
    }
}
