//! Textarea widget for TextareaParameter.
//!
//! Uses nested Flex containers for CSS-like layout control.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{RichText, TextEdit, Ui, Widget};
use egui_flex::{Flex, FlexAlign, item};
use nebula_parameter::core::{HasValue, Parameter};
use nebula_parameter::types::TextareaParameter;

/// Widget for multi-line text input.
pub struct TextareaWidget {
    parameter: TextareaParameter,
    buffer: String,
    /// Track if the input is currently focused.
    focused: bool,
}

impl ParameterWidget for TextareaWidget {
    type Parameter = TextareaParameter;

    fn new(parameter: Self::Parameter) -> Self {
        let buffer = parameter.get().map(|t| t.to_string()).unwrap_or_default();
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

        let rows = self
            .parameter
            .options
            .as_ref()
            .and_then(|o| o.rows)
            .unwrap_or(4) as usize;

        let placeholder = self
            .parameter
            .options
            .as_ref()
            .and_then(|o| o.placeholder.clone())
            .or_else(|| Some(metadata.description.clone()))
            .filter(|s| !s.is_empty())
            .unwrap_or_default();

        // Outer Flex: vertical container (left-aligned)
        Flex::vertical()
            .w_full()
            .align_items(FlexAlign::Start)
            .gap(egui::vec2(0.0, theme.spacing_sm))
            .show(ui, |flex| {
                // Row 1: Label (left-aligned, bold)
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

                // Row 2: Textarea with styled frame (full width)
                flex.add_ui(item().grow(1.0), |ui| {
                    let width = ui.available_width();
                    let has_error = response.error.is_some();

                    // Apply consistent input frame styling
                    let frame = theme.input_frame(self.focused, has_error);
                    let inner_response = frame.show(ui, |ui| {
                        ui.set_width(width - 20.0); // Account for frame margins
                        TextEdit::multiline(&mut self.buffer)
                            .hint_text(&placeholder)
                            .desired_rows(rows)
                            .frame(false) // We use our own frame
                            .desired_width(ui.available_width())
                            .ui(ui)
                    });

                    let edit_response = inner_response.inner;

                    // Track focus state
                    if edit_response.gained_focus() {
                        self.focused = true;
                    }
                    if edit_response.lost_focus() {
                        self.focused = false;
                        response.lost_focus = true;
                    }

                    if edit_response.changed() {
                        if let Err(e) = self
                            .parameter
                            .set(nebula_value::Text::from(self.buffer.as_str()))
                        {
                            response.error = Some(e.to_string());
                        } else {
                            response.changed = true;
                        }
                    }
                });

                // Row 3: Hint + Character count
                flex.add_ui(item().grow(1.0), |ui| {
                    Flex::horizontal()
                        .w_full()
                        .align_items(FlexAlign::Center)
                        .show(ui, |row| {
                            // Hint (left)
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

                            // Character count (right)
                            row.add_ui(item(), |ui| {
                                if let Some(remaining) = self.parameter.remaining_characters() {
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
        let _ = self.parameter.set(nebula_value::Text::from(value));
    }
}
