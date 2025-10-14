use egui::{Response, Ui, RichText, ScrollArea};
use nebula_parameter::{ObjectParameter, ObjectValue, ObjectParameterOptions};
use crate::{
    ParameterWidget, ParameterTheme, ParameterContext, ValidationState,
};

/// Widget for object/structured parameters
#[derive(Debug, Clone)]
pub struct ObjectWidget<'a> {
    parameter: ObjectParameter,
    context: ParameterContext<'a>,
    expanded: bool,
}

impl<'a> ObjectWidget<'a> {
    pub fn new(parameter: ObjectParameter) -> Self {
        Self {
            parameter,
            context: ParameterContext::default(),
            expanded: true,
        }
    }

    pub fn with_context(mut self, context: ParameterContext) -> Self {
        self.context = context;
        self
    }

    fn allows_additional_properties(&self) -> bool {
        self.parameter.options.as_ref()
            .map(|opts| opts.allow_additional_properties)
            .unwrap_or(false)
    }
}

impl<'a> ParameterWidget for ObjectWidget<'a> {
    fn render(&mut self, ui: &mut Ui) -> Response {
        self.render_with_theme(ui, &ParameterTheme::default())
    }

    fn render_with_theme(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> Response {
        let mut response = ui.allocate_response(ui.available_size(), egui::Sense::click());

        ui.vertical(|ui| {
            // Header with expand/collapse button
            ui.horizontal(|ui| {
                // Expand/collapse button
                let icon = if self.expanded { "▼" } else { "▶" };
                if ui.small_button(icon).clicked() {
                    self.expanded = !self.expanded;
                }

                // Label
                let label_text = if self.parameter.metadata.required {
                    format!("{} *", self.parameter.metadata.name)
                } else {
                    self.parameter.metadata.name.clone()
                };

                ui.label(
                    RichText::new(label_text)
                        .color(theme.colors.label)
                        .font(theme.fonts.label.clone())
                );

                if self.parameter.metadata.required {
                    ui.label(
                        RichText::new("*")
                            .color(theme.colors.required)
                            .font(theme.fonts.label.clone())
                    );
                }

                // Properties count
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} properties", self.parameter.children.len()))
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

            // Expanded content
            if self.expanded {
                ui.add_space(8.0);

                // Object properties
                if !self.parameter.children.is_empty() {
                    // Create a frame for the object
                    egui::Frame::none()
                        .stroke(egui::Stroke::new(1.0, theme.colors.border))
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ScrollArea::vertical()
                                .max_height(400.0)
                                .show(ui, |ui| {
                                    // Sort keys for consistent display
                                    let mut keys: Vec<_> = self.parameter.children.keys().collect();
                                    keys.sort();

                                    for key in keys {
                                        ui.group(|ui| {
                                            ui.vertical(|ui| {
                                                // Property name
                                                ui.label(
                                                    RichText::new(key)
                                                        .color(theme.colors.hint)
                                                        .strong()
                                                );

                                                ui.add_space(4.0);

                                                // Property value placeholder
                                                // Note: In a real implementation, render the child parameter widget here
                                                ui.label(
                                                    RichText::new(format!("Value for {}", key))
                                                        .color(theme.colors.label)
                                                );
                                            });
                                        });

                                        ui.add_space(4.0);
                                    }
                                });
                        });

                    // Additional properties info
                    if self.allows_additional_properties() {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("✓")
                                    .color(theme.colors.success)
                            );
                            ui.label(
                                RichText::new("Additional properties allowed")
                                    .color(theme.colors.hint)
                                    .font(theme.fonts.hint.clone())
                            );
                        });
                    }
                } else {
                    // Empty state
                    ui.label(
                        RichText::new("No properties defined")
                            .color(theme.colors.placeholder)
                            .italic()
                    );
                }
            }

            // Validation state
            if let Some(validation) = &self.parameter.validation {
                if !validation.is_valid() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(validation.error_message())
                            .color(theme.colors.error)
                            .font(theme.fonts.error.clone())
                    );
                }
            }
        });

        response
    }
}

/// Helper function to create an object widget
pub fn object_widget(parameter: ObjectParameter) -> ObjectWidget<'static> {
    ObjectWidget::new(parameter)
}
