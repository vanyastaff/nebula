//! Number input widget for NumberParameter.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{DragValue, RichText, Ui};
use egui_flex::{Flex, FlexAlign, item};
use nebula_parameter::core::Parameter;
use nebula_parameter::types::NumberParameter;

/// Widget for numeric input.
pub struct NumberWidget {
    parameter: NumberParameter,
    value: f64,
}

impl ParameterWidget for NumberWidget {
    type Parameter = NumberParameter;

    fn new(parameter: Self::Parameter) -> Self {
        // Use default value from parameter schema if available
        let value = parameter.default.unwrap_or(0.0);
        Self { parameter, value }
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

        let min = self.parameter.get_min();
        let max = self.parameter.get_max();
        let step = self.parameter.get_step();
        let precision = self.parameter.get_precision();

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

                // Row 2: DragValue input
                flex.add_ui(item().grow(1.0), |ui| {
                    let width = ui.available_width();

                    let mut drag_value = DragValue::new(&mut self.value);

                    match (min, max) {
                        (Some(mn), Some(mx)) => drag_value = drag_value.range(mn..=mx),
                        (Some(mn), None) => drag_value = drag_value.range(mn..=f64::MAX),
                        (None, Some(mx)) => drag_value = drag_value.range(f64::MIN..=mx),
                        _ => {}
                    }

                    if let Some(s) = step {
                        drag_value = drag_value.speed(s * 0.1);
                    } else {
                        drag_value = drag_value.speed(0.1);
                    }

                    if let Some(p) = precision {
                        drag_value = drag_value.max_decimals(p as usize);
                    }

                    let drag_response = ui.add_sized([width, theme.control_height], drag_value);

                    if drag_response.changed() {
                        response.changed = true;
                    }
                    if drag_response.lost_focus() {
                        response.lost_focus = true;
                    }
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

impl NumberWidget {
    #[must_use]
    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn set_value(&mut self, value: f64) {
        self.value = value;
    }
}
