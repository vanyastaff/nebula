//! Radio button widget for RadioParameter.
//!
//! Uses nested Flex containers for CSS-like layout control.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{RichText, TextEdit, Ui};
use egui_flex::{Flex, FlexAlign, item};
use nebula_parameter::core::Parameter;
use nebula_parameter::types::RadioParameter;

/// Widget for radio button selection.
pub struct RadioWidget {
    parameter: RadioParameter,
    selected: Option<String>,
    other_value: String,
    other_selected: bool,
}

impl ParameterWidget for RadioWidget {
    type Parameter = RadioParameter;

    fn new(parameter: Self::Parameter) -> Self {
        // Use default value from parameter schema if available
        let selected = parameter.default.as_ref().map(|t| t.to_string());
        let other_selected = selected
            .as_ref()
            .is_some_and(|v| !parameter.options.iter().any(|o| &o.value == v));
        let other_value = if other_selected {
            selected.clone().unwrap_or_default()
        } else {
            String::new()
        };

        Self {
            parameter,
            selected,
            other_value,
            other_selected,
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

        let metadata = self.parameter.metadata();
        let name = metadata.name.clone();
        let hint = metadata.hint.clone();
        let required = metadata.required;
        let options = self.parameter.options.clone();

        // Outer Flex: vertical container
        Flex::vertical()
            .w_full()
            .gap(egui::vec2(0.0, theme.spacing_xs))
            .show(ui, |flex| {
                // Row 1: Label (nested horizontal Flex)
                flex.add_ui(item().grow(1.0), |ui| {
                    Flex::horizontal()
                        .w_full()
                        .align_items(FlexAlign::Center)
                        .show(ui, |row| {
                            row.add_ui(item(), |ui| {
                                ui.label(
                                    RichText::new(&name)
                                        .size(theme.label_font_size)
                                        .color(theme.label_color),
                                );
                            });
                            if required {
                                row.add_ui(item(), |ui| {
                                    ui.label(
                                        RichText::new("*")
                                            .size(theme.label_font_size)
                                            .color(theme.error),
                                    );
                                });
                            }
                            row.add_ui(item().grow(1.0), |_ui| {});
                        });
                });

                // Radio options - each in its own row
                for option in &options {
                    flex.add_ui(item().grow(1.0), |ui| {
                        Flex::horizontal()
                            .w_full()
                            .align_items(FlexAlign::Center)
                            .show(ui, |row| {
                                row.add_ui(item(), |ui| {
                                    let is_selected = !self.other_selected
                                        && self.selected.as_ref() == Some(&option.value);

                                    if ui.radio(is_selected, &option.name).clicked() {
                                        self.selected = Some(option.value.clone());
                                        self.other_selected = false;
                                        response.changed = true;
                                    }
                                });
                                row.add_ui(item().grow(1.0), |_ui| {});
                            });
                    });
                }

                // "Other" option
                if self.parameter.allows_other() {
                    let other_label = self.parameter.get_other_label();

                    flex.add_ui(item().grow(1.0), |ui| {
                        Flex::horizontal()
                            .w_full()
                            .align_items(FlexAlign::Center)
                            .gap(egui::vec2(theme.spacing_sm, 0.0))
                            .show(ui, |row| {
                                row.add_ui(item(), |ui| {
                                    if ui.radio(self.other_selected, &other_label).clicked() {
                                        self.other_selected = true;
                                        if !self.other_value.is_empty() {
                                            self.selected = Some(self.other_value.clone());
                                            response.changed = true;
                                        }
                                    }
                                });

                                if self.other_selected {
                                    row.add_ui(item().grow(1.0).basis(100.0), |ui| {
                                        let width = ui.available_width();
                                        let text_edit = TextEdit::singleline(&mut self.other_value)
                                            .hint_text("Enter value...")
                                            .desired_width(width);

                                        let edit_response = ui.add(text_edit);

                                        if edit_response.changed() && !self.other_value.is_empty() {
                                            self.selected = Some(self.other_value.clone());
                                            response.changed = true;
                                        }
                                    });
                                } else {
                                    row.add_ui(item().grow(1.0), |_ui| {});
                                }
                            });
                    });
                }

                // Hint
                if let Some(hint_text) = &hint {
                    if !hint_text.is_empty() {
                        flex.add_ui(item().grow(1.0), |ui| {
                            Flex::horizontal()
                                .w_full()
                                .align_items(FlexAlign::Center)
                                .show(ui, |row| {
                                    row.add_ui(item(), |ui| {
                                        ui.label(
                                            RichText::new(hint_text)
                                                .size(theme.hint_font_size)
                                                .color(theme.hint_color),
                                        );
                                    });
                                    row.add_ui(item().grow(1.0), |_ui| {});
                                });
                        });
                    }
                }

                // Error
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

impl RadioWidget {
    #[must_use]
    pub fn selected_value(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn set_selected(&mut self, value: &str) {
        let is_standard = self.parameter.options.iter().any(|o| o.value == value);
        self.other_selected = !is_standard;
        if !is_standard {
            self.other_value = value.to_string();
        }
        self.selected = Some(value.to_string());
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.other_selected = false;
        self.other_value.clear();
    }
}
