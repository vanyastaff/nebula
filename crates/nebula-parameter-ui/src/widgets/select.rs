//! Select widget for SelectParameter.
//!
//! Uses nested Flex containers for CSS-like layout control.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{ComboBox, RichText, Ui};
use egui_flex::{Flex, FlexAlign, item};
use nebula_parameter::core::{HasValue, Parameter};
use nebula_parameter::types::SelectParameter;

/// Widget for single-choice dropdown selection.
/// ```text
/// ┌─────────────────────────────────────┐
/// │ Label *                             │  <- Row 1: label
/// │ [▼ Selected option            ]     │  <- Row 2: dropdown (full width)
/// │ Hint text                           │  <- Row 3: hint
/// └─────────────────────────────────────┘
/// ```
pub struct SelectWidget {
    parameter: SelectParameter,
    selected: Option<String>,
    search_filter: String,
}

impl ParameterWidget for SelectWidget {
    type Parameter = SelectParameter;

    fn new(parameter: Self::Parameter) -> Self {
        let selected = parameter.get().map(|t| t.to_string());
        Self {
            parameter,
            selected,
            search_filter: String::new(),
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
                    .get_option_by_value(v)
                    .map(|o| o.name.clone())
            })
            .unwrap_or_else(|| placeholder.clone());

        let options = self.parameter.options.clone();
        let is_searchable = self
            .parameter
            .select_options
            .as_ref()
            .is_some_and(|o| o.searchable);

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

                // Row 2: ComboBox (full width) with consistent styling
                flex.add_ui(item().grow(1.0), |ui| {
                    let width = ui.available_width();
                    let combo_id = ui.make_persistent_id(&key);

                    // Apply consistent styling to ComboBox
                    ui.style_mut().visuals.widgets.inactive.bg_fill = theme.input_bg;
                    ui.style_mut().visuals.widgets.inactive.bg_stroke =
                        egui::Stroke::new(1.0, theme.input_border);
                    ui.style_mut().visuals.widgets.hovered.bg_fill = theme.input_bg;
                    ui.style_mut().visuals.widgets.hovered.bg_stroke = egui::Stroke::new(
                        theme.input_border_width_focused,
                        theme.input_border_focused,
                    );
                    ui.style_mut().visuals.widgets.active.bg_fill = theme.input_bg;
                    ui.style_mut().visuals.widgets.active.bg_stroke = egui::Stroke::new(
                        theme.input_border_width_focused,
                        theme.input_border_focused,
                    );
                    ui.style_mut().visuals.widgets.open.bg_fill = theme.input_bg;
                    ui.style_mut().visuals.widgets.open.bg_stroke = egui::Stroke::new(
                        theme.input_border_width_focused,
                        theme.input_border_focused,
                    );

                    // Set corner radius
                    ui.style_mut().visuals.widgets.inactive.corner_radius =
                        egui::CornerRadius::same(theme.border_radius as u8);
                    ui.style_mut().visuals.widgets.hovered.corner_radius =
                        egui::CornerRadius::same(theme.border_radius as u8);
                    ui.style_mut().visuals.widgets.active.corner_radius =
                        egui::CornerRadius::same(theme.border_radius as u8);
                    ui.style_mut().visuals.widgets.open.corner_radius =
                        egui::CornerRadius::same(theme.border_radius as u8);

                    // Set minimum height via spacing
                    ui.style_mut().spacing.combo_height = theme.control_height;
                    ui.style_mut().spacing.button_padding = egui::vec2(theme.input_padding, 6.0);

                    ComboBox::from_id_salt(combo_id)
                        .selected_text(&display_text)
                        .width(width)
                        .show_ui(ui, |ui| {
                            // Style popup
                            ui.style_mut().visuals.widgets.inactive.bg_fill = theme.surface;

                            if is_searchable {
                                ui.text_edit_singleline(&mut self.search_filter);
                                ui.add_space(4.0);
                            }

                            for option in &options {
                                if is_searchable
                                    && !self.search_filter.is_empty()
                                    && !option
                                        .name
                                        .to_lowercase()
                                        .contains(&self.search_filter.to_lowercase())
                                {
                                    continue;
                                }

                                let is_selected = self.selected.as_ref() == Some(&option.value);
                                if ui.selectable_label(is_selected, &option.name).clicked() {
                                    self.selected = Some(option.value.clone());
                                    response.changed = true;
                                }
                            }
                        });
                });

                // Row 3: Hint
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

        // Update parameter
        if response.changed {
            if let Some(ref selected) = self.selected {
                if let Err(e) = self
                    .parameter
                    .set(nebula_value::Text::from(selected.as_str()))
                {
                    response.error = Some(e.to_string());
                    response.changed = false;
                }
            }
        }

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
        let _ = self.parameter.set(nebula_value::Text::from(value));
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.parameter.clear();
    }

    #[must_use]
    pub fn selected_display_name(&self) -> Option<String> {
        self.parameter.get_display_name()
    }
}
