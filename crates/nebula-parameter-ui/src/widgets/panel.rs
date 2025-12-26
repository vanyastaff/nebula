//! Panel widget for PanelParameter.

use crate::{ParameterTheme, ParameterWidget, UiExt, WidgetResponse};
use egui::Ui;
use nebula_parameter::core::{Describable, Parameter};
use nebula_parameter::types::PanelParameter;

/// Widget for tabbed/accordion panel containers.
pub struct PanelWidget {
    parameter: PanelParameter,
    /// Currently active panel key
    active_panel: Option<String>,
    /// Open panels (for accordion mode)
    open_panels: std::collections::HashSet<String>,
}

impl ParameterWidget for PanelWidget {
    type Parameter = PanelParameter;

    fn new(parameter: Self::Parameter) -> Self {
        let active_panel = parameter.get_default_panel().map(String::from);
        let mut open_panels = std::collections::HashSet::new();

        // In single mode, only the active panel is open
        if let Some(ref panel) = active_panel {
            open_panels.insert(panel.clone());
        }

        Self {
            parameter,
            active_panel,
            open_panels,
        }
    }

    fn parameter(&self) -> &Self::Parameter {
        &self.parameter
    }

    fn parameter_mut(&mut self) -> &mut Self::Parameter {
        &mut self.parameter
    }

    fn show(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> WidgetResponse {
        let response = WidgetResponse::default();

        // Label
        let metadata = self.parameter.metadata();
        ui.themed_label(theme, &metadata.name);

        // Description
        if !metadata.description.is_empty() {
            ui.themed_hint(theme, &metadata.description);
        }

        ui.add_space(4.0);

        let allows_multiple = self.parameter.allows_multiple_open();

        if allows_multiple {
            // Accordion mode - multiple panels can be open
            self.show_accordion(ui, theme);
        } else {
            // Tab mode - only one panel at a time
            self.show_tabs(ui, theme);
        }

        response
    }
}

impl PanelWidget {
    fn show_tabs(&mut self, ui: &mut Ui, theme: &ParameterTheme) {
        // Tab bar
        ui.horizontal(|ui| {
            for panel in &self.parameter.panels {
                if !panel.enabled {
                    continue;
                }

                let is_active = self.active_panel.as_ref() == Some(&panel.key);
                let label = if let Some(icon) = &panel.icon {
                    format!("{} {}", icon, panel.label)
                } else {
                    panel.label.clone()
                };

                let button = if is_active {
                    egui::Button::new(egui::RichText::new(&label).strong()).fill(theme.primary)
                } else {
                    egui::Button::new(&label)
                };

                if ui.add(button).clicked() {
                    self.active_panel = Some(panel.key.clone());
                    self.open_panels.clear();
                    self.open_panels.insert(panel.key.clone());
                }
            }
        });

        ui.add_space(4.0);

        // Active panel content
        if let Some(active_key) = &self.active_panel.clone() {
            if let Some(panel) = self.parameter.get_panel(active_key) {
                egui::Frame::none()
                    .stroke(egui::Stroke::new(1.0, theme.input_border))
                    .rounding(theme.border_radius)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        if let Some(desc) = &panel.description {
                            ui.themed_hint(theme, desc);
                            ui.add_space(4.0);
                        }

                        // Child parameters would be rendered here
                        // For now, show placeholder
                        ui.label(format!("{} child parameters", panel.children.len()));

                        // Note: To properly render child parameters, you would need
                        // a dynamic widget dispatch system, which is beyond this basic implementation
                        for child in &panel.children {
                            ui.label(format!("- {} ({:?})", child.metadata().name, child.kind()));
                        }
                    });
            }
        }
    }

    fn show_accordion(&mut self, ui: &mut Ui, theme: &ParameterTheme) {
        for panel in &self.parameter.panels {
            if !panel.enabled {
                continue;
            }

            let is_open = self.open_panels.contains(&panel.key);
            let header_label = if let Some(icon) = &panel.icon {
                format!("{} {}", icon, panel.label)
            } else {
                panel.label.clone()
            };

            let header = egui::CollapsingHeader::new(&header_label)
                .default_open(is_open)
                .show(ui, |ui| {
                    if let Some(desc) = &panel.description {
                        ui.themed_hint(theme, desc);
                        ui.add_space(4.0);
                    }

                    // Child parameters placeholder
                    ui.label(format!("{} child parameters", panel.children.len()));

                    for child in &panel.children {
                        ui.label(format!("- {} ({:?})", child.metadata().name, child.kind()));
                    }
                });

            // Track open state
            if header.fully_open() {
                self.open_panels.insert(panel.key.clone());
            } else {
                self.open_panels.remove(&panel.key);
            }
        }
    }

    /// Get the currently active panel key.
    #[must_use]
    pub fn active_panel(&self) -> Option<&str> {
        self.active_panel.as_deref()
    }

    /// Set the active panel by key.
    pub fn set_active_panel(&mut self, key: &str) {
        if self.parameter.get_panel(key).is_some() {
            self.active_panel = Some(key.to_string());
            if !self.parameter.allows_multiple_open() {
                self.open_panels.clear();
            }
            self.open_panels.insert(key.to_string());
        }
    }

    /// Check if a panel is open.
    #[must_use]
    pub fn is_panel_open(&self, key: &str) -> bool {
        self.open_panels.contains(key)
    }

    /// Toggle a panel's open state (accordion mode).
    pub fn toggle_panel(&mut self, key: &str) {
        if self.open_panels.contains(key) {
            self.open_panels.remove(key);
        } else {
            if !self.parameter.allows_multiple_open() {
                self.open_panels.clear();
            }
            self.open_panels.insert(key.to_string());
            self.active_panel = Some(key.to_string());
        }
    }
}
