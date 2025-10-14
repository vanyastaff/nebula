//! File picker widget for FileParameter

use egui::{Response, Ui};
use nebula_parameter::{FileParameter, Parameter, HasValue};
use crate::ParameterWidget;
use crate::helpers::render_parameter_field_compat;

/// Widget for rendering FileParameter
pub struct FileWidget {
    parameter: FileParameter,
    changed: bool,
}

impl FileWidget {
    /// Create a new file widget from a parameter
    pub fn new(parameter: FileParameter) -> Self {
        Self { parameter, changed: false }
    }

    /// Get a reference to the underlying parameter
    pub fn parameter(&self) -> &FileParameter {
        &self.parameter
    }

    /// Get a mutable reference to the underlying parameter
    pub fn parameter_mut(&mut self) -> &mut FileParameter {
        &mut self.parameter
    }

    /// Consume the widget and return the parameter
    pub fn into_parameter(self) -> FileParameter {
        self.parameter
    }
}

impl ParameterWidget for FileWidget {
    fn render(&mut self, ui: &mut Ui) -> Response {
        let metadata = self.parameter.metadata().clone();
        let is_required = self.parameter.is_required();
        let has_value = self.parameter.get().is_some();
        
        render_parameter_field_compat(ui, &metadata, is_required, has_value, |ui| {
            ui.horizontal(|ui| {
                // Show current file path
                let current_path = self.parameter.get()
                    .map(|f| f.path.display().to_string())
                    .unwrap_or_else(|| "No file selected".to_string());
                
                ui.label(current_path);
                
                // File picker button
                if ui.button("📁 Browse...").clicked() {
                    // TODO: Implement native file picker using rfd or egui_file
                    // For now, just show a placeholder
                    eprintln!("File picker not yet implemented");
                }
                
                // Clear button if file is selected
                if self.parameter.get().is_some() {
                    if ui.small_button("✖").clicked() {
                        self.parameter.clear();
                        self.changed = true;
                    }
                }
                
                ui.label("")
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
