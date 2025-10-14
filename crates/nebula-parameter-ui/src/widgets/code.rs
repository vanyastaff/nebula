//! Code editor widget for CodeParameter

use egui::{Response, TextEdit, Ui, FontId};
use nebula_parameter::{CodeParameter, Parameter, HasValue};
use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering CodeParameter
pub struct CodeWidget {
    parameter: CodeParameter,
    changed: bool,
}

impl CodeWidget {
    /// Create a new code widget from a parameter
    pub fn new(parameter: CodeParameter) -> Self {
        Self { parameter, changed: false }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &CodeParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut CodeParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> CodeParameter {
        self.parameter
    }
}

impl ParameterWidget for CodeWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.get().is_some();
        
        let readonly = self.parameter.options
            .as_ref()
            .map(|opts| opts.readonly)
            .unwrap_or(false);
        
        let placeholder = metadata.placeholder.as_deref().unwrap_or("Enter code...");
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            let mut code = self.parameter.get()
                .map(|t| t.as_str().to_string())
                .unwrap_or_default();
            
            // Use monospace font for code
            let text_edit = TextEdit::multiline(&mut code)
                .font(FontId::monospace(13.0))
                .desired_rows(10)
                .desired_width(f32::INFINITY)
                .hint_text(placeholder)
                .interactive(!readonly);
            
            let response = ui.add(text_edit);
            
            if response.changed() && !readonly {
                if let Err(e) = self.parameter.set(nebula_value::Text::from(code)) {
                    eprintln!("Failed to set code parameter: {}", e);
                } else {
                    self.changed = true;
                }
            }
            
            // Show language hint if available
            if let Some(options) = &self.parameter.options {
                if let Some(lang) = &options.language {
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new(format!("Language: {:?}", lang))
                        .size(11.0)
                        .italics());
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

