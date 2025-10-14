use egui::{Response, Ui, RichText, ScrollArea, Button, Color32};
use nebula_parameter::{ListParameter, ListValue, ListParameterOptions};
use crate::{
    ParameterWidget, ParameterTheme, ParameterContext, ValidationState,
};

/// Widget for dynamic list parameters
#[derive(Debug, Clone)]
pub struct ListWidget<'a> {
    parameter: ListParameter,
    context: ParameterContext<'a>,
    expanded: bool,
}

impl<'a> ListWidget<'a> {
    pub fn new(parameter: ListParameter) -> Self {
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

    fn get_min_items(&self) -> usize {
        self.parameter.options.as_ref()
            .and_then(|opts| opts.min_items)
            .unwrap_or(0)
    }

    fn get_max_items(&self) -> Option<usize> {
        self.parameter.options.as_ref()
            .and_then(|opts| opts.max_items)
    }

    fn can_add_item(&self) -> bool {
        if let Some(max) = self.get_max_items() {
            self.parameter.children.len() < max
        } else {
            true
        }
    }

    fn can_remove_item(&self) -> bool {
        let min = self.get_min_items();
        self.parameter.children.len() > min
    }
}

impl<'a> ParameterWidget for ListWidget<'a> {
    fn render(&mut self, ui: &mut Ui) -> Response {
        self.render_with_theme(ui, &ParameterTheme::default())
    }

    fn render_with_theme(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> Response {
        let mut response = ui.allocate_response(ui.available_size(), egui::Sense::click());
        let mut items_changed = false;

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

                // Item count
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} items", self.parameter.children.len()))
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

                // List items
                let items_count = self.parameter.children.len();
                
                if items_count > 0 {
                    // Create a frame for the list
                    egui::Frame::none()
                        .stroke(egui::Stroke::new(1.0, theme.colors.border))
                        .inner_margin(egui::Margin::same(8.0))
                        .show(ui, |ui| {
                            ScrollArea::vertical()
                                .max_height(300.0)
                                .show(ui, |ui| {
                                    for i in 0..items_count {
                                        ui.group(|ui| {
                                            ui.horizontal(|ui| {
                                                // Item number
                                                ui.label(
                                                    RichText::new(format!("#{}", i + 1))
                                                        .color(theme.colors.hint)
                                                        .strong()
                                                );

                                                // Item content placeholder
                                                // Note: In a real implementation, render the child parameter widget here
                                                ui.label(
                                                    RichText::new(format!("Item {}", i + 1))
                                                        .color(theme.colors.label)
                                                );

                                                // Remove button
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    if self.can_remove_item() {
                                                        if ui.small_button("🗑").clicked() {
                                                            // Mark for removal
                                                            items_changed = true;
                                                        }
                                                    }
                                                });
                                            });
                                        });

                                        ui.add_space(4.0);
                                    }
                                });
                        });
                } else {
                    // Empty state
                    ui.label(
                        RichText::new("No items in the list")
                            .color(theme.colors.placeholder)
                            .italic()
                    );
                }

                ui.add_space(8.0);

                // Add item button
                ui.horizontal(|ui| {
                    if self.can_add_item() {
                        if ui.button("➕ Add Item").clicked() {
                            items_changed = true;
                        }
                    } else if let Some(max) = self.get_max_items() {
                        ui.label(
                            RichText::new(format!("Maximum {} items reached", max))
                                .color(theme.colors.warning)
                                .italic()
                        );
                    }
                });

                // Constraints info
                let min = self.get_min_items();
                let max = self.get_max_items();
                if min > 0 || max.is_some() {
                    ui.add_space(4.0);
                    let constraint_text = match (min, max) {
                        (0, Some(m)) => format!("Maximum {} items", m),
                        (n, None) => format!("Minimum {} items", n),
                        (n, Some(m)) => format!("{} to {} items", n, m),
                    };
                    ui.label(
                        RichText::new(constraint_text)
                            .color(theme.colors.hint)
                            .font(theme.fonts.hint.clone())
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

        if items_changed {
            response.mark_changed();
        }

        response
    }
}

/// Helper function to create a list widget
pub fn list_widget(parameter: ListParameter) -> ListWidget<'static> {
    ListWidget::new(parameter)
}
