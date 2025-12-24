//! Textarea widget for TextareaParameter.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{RichText, TextEdit, Ui, Widget};
use egui_flex::{Flex, FlexAlign, item};
use nebula_parameter::core::Parameter;
use nebula_parameter::types::TextareaParameter;

/// Widget for multi-line text input.
pub struct TextareaWidget {
    parameter: TextareaParameter,
    buffer: String,
    focused: bool,
}

impl ParameterWidget for TextareaWidget {
    type Parameter = TextareaParameter;

    fn new(parameter: Self::Parameter) -> Self {
        // Use default value from parameter schema if available
        let buffer = parameter
            .default
            .as_ref()
            .map(|t| t.to_string())
            .unwrap_or_default();
        Self {
            parameter,
            buffer,
            focused: false,
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
        let required = metadata.required;
        let hint = metadata.hint.clone();

        let placeholder = metadata
            .placeholder
            .clone()
            .or_else(|| Some(metadata.description.clone()))
            .filter(|s| !s.is_empty())
            .unwrap_or_default();

        Flex::vertical()
            .w_full()
            .align_items(FlexAlign::Start)
            .gap(egui::vec2(0.0, theme.spacing_sm))
            .show(ui, |flex| {
                // Row 1: Label
                flex.add_ui(item(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(&name)
                                .size(theme.label_font_size)
                                .color(theme.label_color)
                                .strong(),
                        );
                        if required {
                            ui.label(
                                RichText::new("*")
                                    .size(theme.label_font_size)
                                    .color(theme.error),
                            );
                        }
                    });
                });

                // Row 2: Textarea
                flex.add_ui(item().grow(1.0), |ui| {
                    let width = ui.available_width();
                    let has_error = response.error.is_some();

                    let frame = theme.input_frame(self.focused, has_error);
                    let inner_response = frame.show(ui, |ui| {
                        ui.set_width(width - 20.0);
                        TextEdit::multiline(&mut self.buffer)
                            .hint_text(&placeholder)
                            .desired_rows(4)
                            .frame(false)
                            .desired_width(ui.available_width())
                            .ui(ui)
                    });

                    let edit_response = inner_response.inner;

                    if edit_response.gained_focus() {
                        self.focused = true;
                    }
                    if edit_response.lost_focus() {
                        self.focused = false;
                        response.lost_focus = true;
                    }

                    if edit_response.changed() {
                        response.changed = true;
                    }
                });

                // Row 3: Hint + Character count
                flex.add_ui(item().grow(1.0), |ui| {
                    Flex::horizontal()
                        .w_full()
                        .align_items(FlexAlign::Center)
                        .show(ui, |row| {
                            // Hint
                            if let Some(hint_text) = &hint {
                                if !hint_text.is_empty() {
                                    row.add_ui(item(), |ui| {
                                        ui.label(
                                            RichText::new(hint_text)
                                                .size(theme.hint_font_size)
                                                .color(theme.hint_color),
                                        );
                                    });
                                }
                            }

                            // Spacer
                            row.add_ui(item().grow(1.0), |_ui| {});

                            // Character count
                            row.add_ui(item(), |ui| {
                                let current_value = nebula_value::Text::new(self.buffer.clone());
                                if let Some(remaining) =
                                    self.parameter.remaining_characters(&current_value)
                                {
                                    let color = if remaining < 0 {
                                        theme.error
                                    } else if remaining < 20 {
                                        theme.warning
                                    } else {
                                        theme.hint_color
                                    };
                                    ui.label(
                                        RichText::new(format!("{}", remaining))
                                            .size(theme.hint_font_size)
                                            .color(color),
                                    );
                                } else {
                                    let count = self.buffer.chars().count();
                                    ui.label(
                                        RichText::new(format!("{}", count))
                                            .size(theme.hint_font_size)
                                            .color(theme.hint_color),
                                    );
                                }
                            });
                        });
                });

                // Error
                if let Some(ref error) = response.error {
                    flex.add_ui(item(), |ui| {
                        ui.label(
                            RichText::new(error)
                                .size(theme.hint_font_size)
                                .color(theme.error),
                        );
                    });
                }
            });

        response
    }
}

impl TextareaWidget {
    #[must_use]
    pub fn value(&self) -> &str {
        &self.buffer
    }

    pub fn set_value(&mut self, value: &str) {
        self.buffer = value.to_string();
    }
}
