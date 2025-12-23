//! Secret input widget for SecretParameter.
//!
//! Uses nested Flex containers for CSS-like layout control.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{RichText, TextEdit, Ui};
use egui_flex::{Flex, FlexAlign, item};
use egui_phosphor::regular::{EYE, EYE_SLASH};
use nebula_parameter::core::{HasValue, Parameter};
use nebula_parameter::types::{SecretParameter, SecretString};

/// Widget for password/secret input.
/// ```text
/// ┌─────────────────────────────────────┐
/// │ Label *                             │  <- Row 1: label
/// │ [••••••••••••••••••••••••] [Show]   │  <- Row 2: input + toggle
/// │ Current: ••••••••                   │  <- Row 3: masked indicator
/// │ Hint text                           │  <- Row 4: hint
/// └─────────────────────────────────────┘
/// ```
pub struct SecretWidget {
    parameter: SecretParameter,
    buffer: String,
    show_password: bool,
    /// Track if the input is currently focused.
    focused: bool,
}

impl ParameterWidget for SecretWidget {
    type Parameter = SecretParameter;

    fn new(parameter: Self::Parameter) -> Self {
        Self {
            parameter,
            buffer: String::new(),
            show_password: false,
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
        let hint = metadata.hint.clone();
        let required = metadata.required;

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

                // Row 2: Password input with embedded toggle icon
                flex.add_ui(item().grow(1.0), |ui| {
                    let width = ui.available_width();
                    let has_error = response.error.is_some();

                    // Apply consistent input frame styling
                    let frame = theme.input_frame(self.focused, has_error);
                    frame.show(ui, |ui| {
                        ui.set_width(width - 20.0); // Account for frame margins
                        ui.horizontal(|ui| {
                            ui.style_mut().visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                            ui.style_mut().visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
                            ui.style_mut().visuals.widgets.active.bg_stroke = egui::Stroke::NONE;

                            // Text input - no frame, we use our own
                            let text_edit = if self.show_password {
                                TextEdit::singleline(&mut self.buffer).frame(false)
                            } else {
                                TextEdit::singleline(&mut self.buffer)
                                    .password(true)
                                    .frame(false)
                            };

                            let edit_response =
                                ui.add(text_edit.desired_width(ui.available_width() - 30.0));

                            // Track focus state
                            if edit_response.gained_focus() {
                                self.focused = true;
                            }
                            if edit_response.lost_focus() {
                                self.focused = false;
                                response.lost_focus = true;
                            }

                            if edit_response.changed() {
                                if let Err(e) = self.parameter.set(SecretString::new(&self.buffer))
                                {
                                    response.error = Some(e.to_string());
                                } else {
                                    response.changed = true;
                                }
                            }

                            // Eye icon button - inside the frame
                            let icon = if self.show_password { EYE_SLASH } else { EYE };
                            let icon_btn = ui.add(egui::Button::new(icon).frame(false));
                            if icon_btn.clicked() {
                                self.show_password = !self.show_password;
                            }
                        });
                    });
                });

                // Row 3: Masked value indicator (if has existing secret)
                if self.parameter.has_secret() && self.buffer.is_empty() {
                    if let Some(masked) = self.parameter.masked_value() {
                        flex.add_ui(item().grow(1.0), |ui| {
                            ui.label(
                                RichText::new(format!("Current: {}", masked))
                                    .size(theme.hint_font_size)
                                    .color(theme.hint_color),
                            );
                        });
                    }
                }

                // Row 4: Hint
                if let Some(hint_text) = &hint {
                    if !hint_text.is_empty() {
                        flex.add_ui(item().grow(1.0), |ui| {
                            ui.label(
                                RichText::new(hint_text)
                                    .size(theme.hint_font_size)
                                    .color(theme.hint_color),
                            );
                        });
                    }
                }

                // Error
                if let Some(ref error) = response.error {
                    flex.add_ui(item().grow(1.0), |ui| {
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

impl SecretWidget {
    #[must_use]
    pub fn has_value(&self) -> bool {
        self.parameter.has_secret()
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.parameter.clear();
    }

    pub fn toggle_visibility(&mut self) {
        self.show_password = !self.show_password;
    }
}
