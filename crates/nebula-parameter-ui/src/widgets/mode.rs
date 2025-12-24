//! Mode widget for ModeParameter.
//!
//! Dropdown for mode selection with inline input field.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{ComboBox, CornerRadius, Frame, Stroke, Ui};
use nebula_parameter::core::Parameter;
use nebula_parameter::types::{ModeParameter, ModeValue};

/// Widget for mode selection with inline child input.
pub struct ModeWidget {
    parameter: ModeParameter,
    selected_mode: Option<String>,
    child_value: String,
    mode_value: Option<ModeValue>,
}

impl ParameterWidget for ModeWidget {
    type Parameter = ModeParameter;

    fn new(parameter: Self::Parameter) -> Self {
        // Initialize from default value or default mode
        let (selected_mode, child_value, mode_value) = if let Some(default) = &parameter.default {
            let child_str = match &default.value {
                nebula_value::Value::Text(t) => t.to_string(),
                nebula_value::Value::Null => String::new(),
                other => format!("{:?}", other),
            };
            (Some(default.key.clone()), child_str, Some(default.clone()))
        } else if let Some(default_mode) = parameter.default_mode() {
            (Some(default_mode.key.clone()), String::new(), None)
        } else {
            (None, String::new(), None)
        };

        Self {
            parameter,
            selected_mode,
            child_value,
            mode_value,
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
        let description = metadata.description.clone();
        let key = metadata.key.clone();
        let required = metadata.required;

        // Collect modes
        let modes: Vec<_> = self
            .parameter
            .modes
            .iter()
            .map(|m| (m.key.clone(), m.name.clone()))
            .collect();

        if modes.is_empty() {
            ui.label(egui::RichText::new(&name).color(theme.label_color));
            ui.label(egui::RichText::new("No modes available").color(theme.hint_color));
            return response;
        }

        // Header
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&name).color(theme.label_color).strong());
            if required {
                ui.label(egui::RichText::new("*").color(theme.error));
            }
        });

        ui.add_space(4.0);

        let current_mode_name = self
            .selected_mode
            .as_ref()
            .and_then(|key| modes.iter().find(|(k, _)| k == key))
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| "Select...".to_string());

        // Input frame with dropdown and text field
        let frame_response = Frame::none()
            .fill(theme.input_bg)
            .stroke(Stroke::new(1.0, theme.input_border))
            .rounding(CornerRadius::same(theme.border_radius as u8))
            .inner_margin(egui::Margin::symmetric(4, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(24.0);

                    // Mode dropdown
                    let combo_id = ui.make_persistent_id(format!("{}_mode", key));
                    ComboBox::from_id_salt(combo_id)
                        .selected_text(&current_mode_name)
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for (mode_key, mode_name) in &modes {
                                let is_selected = self.selected_mode.as_ref() == Some(mode_key);
                                if ui.selectable_label(is_selected, mode_name).clicked()
                                    && !is_selected
                                {
                                    self.selected_mode = Some(mode_key.clone());
                                    self.child_value.clear();
                                    self.mode_value = Some(ModeValue::text(mode_key.clone(), ""));
                                    response.changed = true;
                                }
                            }
                        });

                    ui.separator();

                    // Text input
                    if self.selected_mode.is_some() {
                        let text_edit = egui::TextEdit::singleline(&mut self.child_value)
                            .hint_text("Enter value...")
                            .frame(false)
                            .desired_width(ui.available_width() - 8.0);

                        let text_response = ui.add(text_edit);

                        if text_response.changed() {
                            if let Some(ref selected_key) = self.selected_mode {
                                self.mode_value =
                                    Some(ModeValue::text(selected_key.clone(), &self.child_value));
                                response.changed = true;
                            }
                        }

                        if text_response.lost_focus() {
                            response.lost_focus = true;
                        }
                    } else {
                        ui.label(egui::RichText::new("Select mode").color(theme.placeholder_color));
                    }
                });
            });

        // Hover effect
        if frame_response.response.hovered() {
            ui.painter().rect_stroke(
                frame_response.response.rect,
                CornerRadius::same(theme.border_radius as u8),
                Stroke::new(1.5, theme.input_border_focused),
                egui::StrokeKind::Outside,
            );
        }

        // Description
        if !description.is_empty() {
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(&description)
                    .small()
                    .color(theme.hint_color),
            );
        }

        // Error
        if let Some(ref error) = response.error {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(error).small().color(theme.error));
        }

        response
    }
}

impl ModeWidget {
    #[must_use]
    pub fn selected_mode(&self) -> Option<&str> {
        self.selected_mode.as_deref()
    }

    pub fn set_mode(&mut self, mode_key: &str) -> Result<(), String> {
        if self.parameter.has_mode(mode_key) {
            self.selected_mode = Some(mode_key.to_string());
            self.child_value.clear();
            self.mode_value = Some(ModeValue::text(mode_key, ""));
            Ok(())
        } else {
            Err(format!("Mode '{}' not found", mode_key))
        }
    }

    #[must_use]
    pub fn value(&self) -> Option<&ModeValue> {
        self.mode_value.as_ref()
    }

    #[must_use]
    pub fn child_value(&self) -> &str {
        &self.child_value
    }

    pub fn set_child_value(&mut self, value: impl Into<String>) {
        self.child_value = value.into();
        if let Some(ref mode_key) = self.selected_mode {
            self.mode_value = Some(ModeValue::text(mode_key.clone(), &self.child_value));
        }
    }

    #[must_use]
    pub fn available_modes(&self) -> Vec<&str> {
        self.parameter
            .modes
            .iter()
            .map(|m| m.key.as_str())
            .collect()
    }
}
