//! Multi-select widget for MultiSelectParameter.
//!
//! Displays as a dropdown with checkboxes inside the popup.
//! Uses nested Flex containers for CSS-like layout control.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{ComboBox, RichText, Ui};
use egui_flex::{Flex, FlexAlign, FlexAlignContent, item};
use nebula_parameter::core::{Describable, Parameter};
use nebula_parameter::types::MultiSelectParameter;

/// Widget for multiple selection with dropdown containing checkboxes.
/// ```text
/// ┌─────────────────────────────────────┐
/// │ Label *                  2 selected │  <- Row 1: label + count
/// │ [▼ Item1, Item2                   ] │  <- Row 2: dropdown (full width)
/// │ min: 1, max: 5                      │  <- Row 3: constraints (optional)
/// │ Hint text                           │  <- Row 4: hint
/// └─────────────────────────────────────┘
/// ```
pub struct MultiSelectWidget {
    parameter: MultiSelectParameter,
    selected: Vec<String>,
}

impl ParameterWidget for MultiSelectWidget {
    type Parameter = MultiSelectParameter;

    fn new(parameter: Self::Parameter) -> Self {
        let selected = parameter.default.clone().unwrap_or_default();
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
        let options = self.parameter.options.clone();

        // Build display text
        let display_text = if self.selected.is_empty() {
            "Select options...".to_string()
        } else if self.selected.len() <= 2 {
            let names: Vec<_> = self
                .selected
                .iter()
                .filter_map(|v| {
                    options
                        .iter()
                        .find(|o| &o.value == v)
                        .map(|o| o.name.as_str())
                })
                .collect();
            names.join(", ")
        } else {
            format!("{} items selected", self.selected.len())
        };

        // Outer Flex: vertical container (left-aligned)
        Flex::vertical()
            .w_full()
            .align_items(FlexAlign::Start)
            .gap(egui::vec2(0.0, theme.spacing_sm))
            .show(ui, |flex| {
                // Row 1: Label ... Count (nested horizontal Flex)
                flex.add_ui(item().grow(1.0), |ui| {
                    Flex::horizontal()
                        .w_full()
                        .align_items(FlexAlign::Center)
                        .show(ui, |row| {
                            // Left group: Label + required marker
                            row.add_ui(item(), |ui| {
                                Flex::horizontal()
                                    .align_items(FlexAlign::Center)
                                    .gap(egui::vec2(2.0, 0.0))
                                    .show(ui, |label_group| {
                                        label_group.add_ui(item(), |ui| {
                                            ui.label(
                                                RichText::new(&name)
                                                    .size(theme.label_font_size)
                                                    .color(theme.label_color),
                                            );
                                        });
                                        if required {
                                            label_group.add_ui(item(), |ui| {
                                                ui.label(
                                                    RichText::new("*")
                                                        .size(theme.label_font_size)
                                                        .color(theme.error),
                                                );
                                            });
                                        }
                                    });
                            });

                            // Spacer - grows to push count to the right
                            row.add_ui(item().grow(1.0), |_ui| {});

                            // Right: Selection count
                            row.add_ui(item(), |ui| {
                                ui.label(
                                    RichText::new(format!("{} selected", self.selected.len()))
                                        .size(theme.hint_font_size)
                                        .color(theme.hint_color),
                                );
                            });
                        });
                });

                // Row 2: ComboBox dropdown (full width, nested horizontal Flex)
                flex.add_ui(item().grow(1.0), |ui| {
                    Flex::horizontal()
                        .w_full()
                        .align_items(FlexAlign::Center)
                        .align_content(FlexAlignContent::Stretch)
                        .show(ui, |row| {
                            // ComboBox - GROWS to fill all space
                            row.add_ui(item().grow(1.0).basis(100.0), |ui| {
                                let width = ui.available_width();
                                let combo_id = ui.make_persistent_id(&key);

                                ComboBox::from_id_salt(combo_id)
                                    .selected_text(&display_text)
                                    .width(width)
                                    .show_ui(ui, |ui| {
                                        for option in &options {
                                            let mut is_selected =
                                                self.selected.contains(&option.value);
                                            let old_selected = is_selected;

                                            // Use checkbox instead of selectable_label
                                            if ui.checkbox(&mut is_selected, &option.name).changed()
                                            {
                                                if is_selected && !old_selected {
                                                    self.selected.push(option.value.clone());
                                                } else if !is_selected && old_selected {
                                                    self.selected.retain(|v| v != &option.value);
                                                }
                                                response.changed = true;
                                            }
                                        }
                                    });
                            });
                        });
                });

                // Row 3: Constraints info (optional)
                if let Some(opts) = &self.parameter.multi_select_options {
                    let mut constraints = Vec::new();
                    if let Some(min) = opts.min_selections {
                        constraints.push(format!("min: {}", min));
                    }
                    if let Some(max) = opts.max_selections {
                        constraints.push(format!("max: {}", max));
                    }
                    if !constraints.is_empty() {
                        flex.add_ui(item().grow(1.0), |ui| {
                            ui.label(
                                RichText::new(constraints.join(", "))
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

impl MultiSelectWidget {
    #[must_use]
    pub fn selected_values(&self) -> &[String] {
        &self.selected
    }

    #[must_use]
    pub fn is_selected(&self, value: &str) -> bool {
        self.selected.contains(&value.to_string())
    }

    pub fn add_selection(&mut self, value: &str) {
        if !self.selected.contains(&value.to_string()) {
            self.selected.push(value.to_string());
        }
    }

    pub fn remove_selection(&mut self, value: &str) {
        self.selected.retain(|v| v != value);
    }

    pub fn toggle_selection(&mut self, value: &str) {
        if self.is_selected(value) {
            self.remove_selection(value);
        } else {
            self.add_selection(value);
        }
    }

    pub fn clear_selections(&mut self) {
        self.selected.clear();
    }

    #[must_use]
    pub fn selection_count(&self) -> usize {
        self.selected.len()
    }
}
