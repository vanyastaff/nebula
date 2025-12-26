//! File upload widget for FileParameter.

use crate::{ParameterTheme, ParameterWidget, UiExt, WidgetResponse};
use egui::Ui;
use nebula_parameter::core::{Describable, Parameter};
use nebula_parameter::types::{FileParameter, FileReference};
use std::path::PathBuf;

/// Widget for file selection/upload.
pub struct FileWidget {
    parameter: FileParameter,
    /// Currently selected file
    file_ref: Option<FileReference>,
    /// Drag-drop highlight state
    drag_hover: bool,
}

impl ParameterWidget for FileWidget {
    type Parameter = FileParameter;

    fn new(parameter: Self::Parameter) -> Self {
        Self {
            parameter,
            file_ref: None,
            drag_hover: false,
        }
    }

    fn parameter(&self) -> &Self::Parameter {
        &self.parameter
    }

    fn parameter_mut(&mut self) -> &mut Self::Parameter {
        &mut self.parameter
    }

    fn show(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> WidgetResponse {
        let mut response = WidgetResponse::default();

        // Label
        let metadata = self.parameter.metadata();
        ui.themed_label(theme, &metadata.name);

        // Description
        if !metadata.description.is_empty() {
            ui.themed_hint(theme, &metadata.description);
        }

        ui.add_space(4.0);

        // Drop zone
        let drop_zone_color = if self.drag_hover {
            theme.primary
        } else {
            theme.input_border
        };

        egui::Frame::none()
            .stroke(egui::Stroke::new(2.0, drop_zone_color))
            .rounding(theme.border_radius)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    if let Some(file_ref) = &self.file_ref {
                        // Show selected file
                        ui.label(&file_ref.name);

                        if let Some(size) = file_ref.size {
                            ui.themed_hint(theme, &format_file_size(size));
                        }

                        if let Some(mime) = &file_ref.mime_type {
                            ui.themed_hint(theme, mime);
                        }

                        ui.horizontal(|ui| {
                            // Clear button
                            if ui.button("Clear").clicked() {
                                self.file_ref = None;
                                response.changed = true;
                            }

                            // Change button
                            if ui.button("Change").clicked() {
                                self.open_file_dialog(&mut response);
                            }
                        });
                    } else {
                        // Empty state
                        ui.label("\u{1F4C1}"); // Folder icon
                        ui.label("Drop file here or click to browse");

                        // Accepted formats
                        if let Some(formats) = self.parameter.get_accepted_formats() {
                            ui.themed_hint(theme, &format!("Accepts: {}", formats.join(", ")));
                        }

                        // Max size
                        if let Some(max_size) = self.parameter.get_max_size() {
                            ui.themed_hint(
                                theme,
                                &format!("Max size: {}", format_file_size(max_size)),
                            );
                        }

                        // Browse button
                        if ui.button("Browse...").clicked() {
                            self.open_file_dialog(&mut response);
                        }
                    }
                });
            });

        // Handle drag and drop (egui doesn't have native file drop, so this is a placeholder)
        // In a real app, you'd integrate with the platform's file drop API
        let drop_response = ui.interact(
            ui.min_rect(),
            ui.id().with("file_drop"),
            egui::Sense::hover(),
        );

        self.drag_hover = drop_response.hovered();

        // Multiple files indicator
        if self.parameter.allows_multiple() {
            ui.themed_hint(theme, "Multiple files allowed");
        }

        // Show validation error
        if let Some(ref error) = response.error {
            ui.themed_error(theme, error);
        }

        response
    }
}

impl FileWidget {
    fn open_file_dialog(&mut self, response: &mut WidgetResponse) {
        // Note: egui doesn't have native file dialogs
        // In a real application, you would use rfd or another crate
        // For now, we'll just show a message

        // This is a placeholder - real implementation would use:
        // if let Some(path) = rfd::FileDialog::new().pick_file() {
        //     self.set_file(path);
        //     response.changed = true;
        // }

        // File dialog would open here - integrate with rfd crate for real functionality
        let _ = response;
    }

    /// Get the currently selected file reference.
    #[must_use]
    pub fn file(&self) -> Option<&FileReference> {
        self.file_ref.as_ref()
    }

    /// Get the file name.
    #[must_use]
    pub fn file_name(&self) -> Option<&str> {
        self.file_ref.as_ref().map(|f| f.name.as_str())
    }

    /// Get the file path.
    #[must_use]
    pub fn file_path(&self) -> Option<&PathBuf> {
        self.file_ref.as_ref().map(|f| &f.path)
    }

    /// Set a file from a path.
    pub fn set_file(&mut self, path: PathBuf) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        self.file_ref = Some(FileReference::new(path, name));
    }

    /// Set a file with full metadata.
    pub fn set_file_with_metadata(
        &mut self,
        path: PathBuf,
        name: String,
        size: Option<u64>,
        mime_type: Option<String>,
    ) {
        let mut file_ref = FileReference::new(path, name);

        if let Some(size) = size {
            file_ref = file_ref.with_size(size);
        }

        if let Some(mime) = mime_type {
            file_ref = file_ref.with_mime_type(mime);
        }

        self.file_ref = Some(file_ref);
    }

    /// Clear the selected file.
    pub fn clear(&mut self) {
        self.file_ref = None;
    }
}

fn format_file_size(size: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if size >= GB {
        format!("{:.2} GB", size as f64 / GB as f64)
    } else if size >= MB {
        format!("{:.2} MB", size as f64 / MB as f64)
    } else if size >= KB {
        format!("{:.2} KB", size as f64 / KB as f64)
    } else {
        format!("{} bytes", size)
    }
}
