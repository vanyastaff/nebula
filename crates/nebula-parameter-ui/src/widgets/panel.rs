use egui::{Response, Ui, RichText, ScrollArea, Color32};
use nebula_parameter::{PanelParameter, Panel, PanelParameterOptions};
use crate::{
    ParameterWidget, ParameterTheme, ParameterContext, ValidationState,
};

/// Widget for panel/tabbed interface parameters
#[derive(Debug, Clone)]
pub struct PanelWidget<'a> {
    parameter: PanelParameter,
    context: ParameterContext<'a>,
    active_panel: Option<String>,
}

impl<'a> PanelWidget<'a> {
    pub fn new(parameter: PanelParameter) -> Self {
        let default_panel = parameter.options.as_ref()
            .and_then(|opts| opts.default_panel.clone())
            .or_else(|| parameter.panels.first().map(|p| p.key.clone()));

        Self {
            parameter,
            context: ParameterContext::default(),
            active_panel: default_panel,
        }
    }

    pub fn with_context(mut self, context: ParameterContext) -> Self {
        self.context = context;
        self
    }

    fn get_panel_icon(&self, panel: &Panel) -> &str {
        panel.icon.as_deref().unwrap_or("📄")
    }
}

impl<'a> ParameterWidget for PanelWidget<'a> {
    fn render(&mut self, ui: &mut Ui) -> Response {
        self.render_with_theme(ui, &ParameterTheme::default())
    }

    fn render_with_theme(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> Response {
        let mut response = ui.allocate_response(ui.available_size(), egui::Sense::click());
        let mut panel_changed = false;

        ui.vertical(|ui| {
            // Header
            ui.horizontal(|ui| {
                // Label
                ui.label(
                    RichText::new(&self.parameter.metadata.name)
                        .color(theme.colors.label)
                        .font(theme.fonts.label.clone())
                        .strong()
                );

                // Panel count
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} panels", self.parameter.panels.len()))
                            .color(theme.colors.description)
                            .font(theme.fonts.description.clone())
                    );
                });
            });

            // Description
            if let Some(description) = &self.parameter.metadata.description {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(description)
                        .color(theme.colors.description)
                        .font(theme.fonts.description.clone())
                );
            }

            ui.add_space(8.0);

            // Tab buttons
            ui.horizontal(|ui| {
                for panel in &self.parameter.panels {
                    if !panel.enabled {
                        continue;
                    }

                    let is_active = self.active_panel.as_ref() == Some(&panel.key);
                    let icon = self.get_panel_icon(panel);
                    let label = format!("{} {}", icon, panel.label);

                    // Style for active/inactive tabs
                    let button = if is_active {
                        egui::Button::new(
                            RichText::new(&label)
                                .color(theme.colors.label)
                                .strong()
                        )
                        .fill(theme.colors.background_hover)
                        .stroke(egui::Stroke::new(2.0, theme.colors.border_focused))
                    } else {
                        egui::Button::new(
                            RichText::new(&label)
                                .color(theme.colors.description)
                        )
                        .fill(theme.colors.background)
                        .stroke(egui::Stroke::new(1.0, theme.colors.border))
                    };

                    if ui.add(button).clicked() {
                        self.active_panel = Some(panel.key.clone());
                        panel_changed = true;
                    }
                }
            });

            ui.add_space(8.0);

            // Active panel content
            if let Some(active_key) = &self.active_panel {
                if let Some(active_panel) = self.parameter.panels.iter()
                    .find(|p| &p.key == active_key) 
                {
                    // Panel content frame
                    egui::Frame::none()
                        .stroke(egui::Stroke::new(1.0, theme.colors.border))
                        .inner_margin(egui::Margin::same(12.0))
                        .show(ui, |ui| {
                            // Panel description
                            if let Some(desc) = &active_panel.description {
                                ui.label(
                                    RichText::new(desc)
                                        .color(theme.colors.description)
                                        .font(theme.fonts.description.clone())
                                );
                                ui.add_space(8.0);
                            }

                            // Panel content
                            ScrollArea::vertical()
                                .max_height(400.0)
                                .show(ui, |ui| {
                                    if active_panel.children.is_empty() {
                                        ui.label(
                                            RichText::new("No parameters in this panel")
                                                .color(theme.colors.placeholder)
                                                .italic()
                                        );
                                    } else {
                                        // Note: In a real implementation, render child parameter widgets here
                                        ui.label(
                                            RichText::new(format!(
                                                "{} parameters in this panel",
                                                active_panel.children.len()
                                            ))
                                            .color(theme.colors.label)
                                        );

                                        ui.add_space(8.0);

                                        for (i, _child) in active_panel.children.iter().enumerate() {
                                            ui.group(|ui| {
                                                ui.label(
                                                    RichText::new(format!("Parameter {}", i + 1))
                                                        .color(theme.colors.label)
                                                );
                                            });
                                            ui.add_space(4.0);
                                        }
                                    }
                                });
                        });
                }
            } else {
                // No active panel
                ui.label(
                    RichText::new("Select a panel to view its contents")
                        .color(theme.colors.placeholder)
                        .italic()
                );
            }
        });

        if panel_changed {
            response.mark_changed();
        }

        response
    }
}

/// Helper function to create a panel widget
pub fn panel_widget(parameter: PanelParameter) -> PanelWidget<'static> {
    PanelWidget::new(parameter)
}
