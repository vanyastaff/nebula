//! Checkbox widget for CheckboxParameter.
//!
//! Uses nested Flex containers for CSS-like layout control.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{RichText, Ui};
use egui_flex::{Flex, FlexAlign, item};
use nebula_parameter::core::{Describable, Parameter};
use nebula_parameter::types::CheckboxParameter;

/// Widget for boolean checkbox input.
pub struct CheckboxWidget {
    parameter: CheckboxParameter,
    checked: bool,
}

impl ParameterWidget for CheckboxWidget {
    type Parameter = CheckboxParameter;

    fn new(parameter: Self::Parameter) -> Self {
        // Use default value from parameter schema if available
        let checked = parameter
            .default
            .as_ref()
            .map(|b| b.value())
            .unwrap_or(false);
        Self { parameter, checked }
    }

    fn parameter(&self) -> &Self::Parameter {
        &self.parameter
    }

    fn parameter_mut(&mut self) -> &mut Self::Parameter {
        &mut self.parameter
    }

    fn show(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> WidgetResponse {
        let mut response = WidgetResponse::default();

        let metadata = self.parameter.metadata();
        let description = metadata.description.clone();

        let label = self
            .parameter
            .options
            .as_ref()
            .and_then(|o| o.label.clone())
            .unwrap_or_else(|| metadata.name.clone());

        let help_text = self
            .parameter
            .options
            .as_ref()
            .and_then(|o| o.help_text.clone());

        // Outer Flex: vertical container
        Flex::vertical()
            .w_full()
            .gap(egui::vec2(0.0, theme.spacing_xs))
            .show(ui, |flex| {
                // Row 1: Checkbox + Label (nested horizontal Flex)
                flex.add_ui(item().grow(1.0), |ui| {
                    Flex::horizontal()
                        .w_full()
                        .align_items(FlexAlign::Center)
                        .gap(egui::vec2(theme.spacing_sm, 0.0))
                        .show(ui, |row| {
                            // Checkbox - left aligned, takes only needed space
                            row.add_ui(item(), |ui| {
                                let checkbox_response = ui.checkbox(&mut self.checked, "");

                                if checkbox_response.changed() {
                                    // Value is stored in the widget, not in the parameter
                                    response.changed = true;
                                }
                            });

                            // Label
                            row.add_ui(item(), |ui| {
                                ui.label(
                                    RichText::new(&label)
                                        .size(theme.label_font_size)
                                        .color(theme.label_color),
                                );
                            });

                            // Spacer to push to left
                            row.add_ui(item().grow(1.0), |_ui| {});
                        });
                });

                // Row 2: Description (nested horizontal Flex)
                if !description.is_empty() {
                    flex.add_ui(item().grow(1.0), |ui| {
                        Flex::horizontal()
                            .w_full()
                            .align_items(FlexAlign::Center)
                            .show(ui, |row| {
                                row.add_ui(item(), |ui| {
                                    ui.label(
                                        RichText::new(&description)
                                            .size(theme.hint_font_size)
                                            .color(theme.hint_color),
                                    );
                                });
                                row.add_ui(item().grow(1.0), |_ui| {});
                            });
                    });
                }

                // Row 3: Help text (nested horizontal Flex)
                if let Some(help) = &help_text {
                    flex.add_ui(item().grow(1.0), |ui| {
                        Flex::horizontal()
                            .w_full()
                            .align_items(FlexAlign::Center)
                            .show(ui, |row| {
                                row.add_ui(item(), |ui| {
                                    ui.label(
                                        RichText::new(help)
                                            .size(theme.hint_font_size)
                                            .color(theme.hint_color),
                                    );
                                });
                                row.add_ui(item().grow(1.0), |_ui| {});
                            });
                    });
                }

                // Error (nested horizontal Flex)
                if let Some(ref error) = response.error {
                    flex.add_ui(item().grow(1.0), |ui| {
                        Flex::horizontal()
                            .w_full()
                            .align_items(FlexAlign::Center)
                            .show(ui, |row| {
                                row.add_ui(item(), |ui| {
                                    ui.label(
                                        RichText::new(error)
                                            .size(theme.hint_font_size)
                                            .color(theme.error),
                                    );
                                });
                                row.add_ui(item().grow(1.0), |_ui| {});
                            });
                    });
                }
            });

        response
    }
}

impl CheckboxWidget {
    #[must_use]
    pub fn is_checked(&self) -> bool {
        self.checked
    }

    pub fn set_checked(&mut self, checked: bool) {
        self.checked = checked;
    }

    pub fn toggle(&mut self) {
        self.set_checked(!self.checked);
    }
}
