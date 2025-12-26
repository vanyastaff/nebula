//! Select widget for SelectParameter.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{ComboBox, RichText, Ui};
use egui_flex::{Flex, FlexAlign, item};
use nebula_parameter::core::{Describable, Parameter};
use nebula_parameter::types::SelectParameter;

/// Widget for single-choice dropdown selection.
pub struct SelectWidget {
    parameter: SelectParameter,
    selected: Option<String>,
}

impl ParameterWidget for SelectWidget {
    type Parameter = SelectParameter;

    fn new(parameter: Self::Parameter) -> Self {
        // Use default value from parameter schema if available
        let selected = parameter.default.as_ref().map(|t| t.to_string());
        Self {
            parameter,
            selected,
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
        let key = metadata.key.clone();
        let required = metadata.required;

        let placeholder = self
            .parameter
            .select_options
            .as_ref()
            .and_then(|o| o.placeholder.clone())
            .or_else(|| Some(metadata.description.clone()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Select...".to_string());

        let display_text = self
            .selected
            .as_ref()
            .and_then(|v| {
                self.parameter
                    .options
                    .iter()
                    .find(|o| &o.value == v)
                    .map(|o| o.name.clone())
            })
            .unwrap_or_else(|| placeholder.clone());

        let options = self.parameter.options.clone();

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

                // Row 2: ComboBox
                flex.add_ui(item().grow(1.0), |ui| {
                    let width = ui.available_width();
                    let combo_id = ui.make_persistent_id(&key);

                    ui.style_mut().visuals.widgets.inactive.bg_fill = theme.input_bg;
                    ui.style_mut().visuals.widgets.inactive.bg_stroke =
                        egui::Stroke::new(1.0, theme.input_border);
                    ui.style_mut().spacing.combo_height = theme.control_height;

                    ComboBox::from_id_salt(combo_id)
                        .selected_text(&display_text)
                        .width(width)
                        .show_ui(ui, |ui| {
                            for option in &options {
                                let is_selected = self.selected.as_ref() == Some(&option.value);
                                if ui.selectable_label(is_selected, &option.name).clicked() {
                                    self.selected = Some(option.value.clone());
                                    response.changed = true;
                                }
                            }
                        });
                });

                // Hint
                if let Some(hint_text) = &hint {
                    if !hint_text.is_empty() {
                        flex.add_ui(item(), |ui| {
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

impl SelectWidget {
    #[must_use]
    pub fn selected_value(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn set_selected(&mut self, value: &str) {
        self.selected = Some(value.to_string());
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
    }
}
