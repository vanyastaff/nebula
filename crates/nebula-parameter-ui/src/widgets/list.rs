//! List widget for ListParameter.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::Ui;
use nebula_parameter::core::{Describable, Parameter};
use nebula_parameter::types::ListParameter;

/// Widget for dynamic list of values.
pub struct ListWidget {
    parameter: ListParameter,
    /// String representations of list items for editing
    item_buffers: Vec<String>,
}

impl ParameterWidget for ListWidget {
    type Parameter = ListParameter;

    fn new(parameter: Self::Parameter) -> Self {
        let item_buffers = parameter
            .default
            .as_ref()
            .map(|arr| arr.iter().map(|v| value_to_string(v)).collect())
            .unwrap_or_default();

        Self {
            parameter,
            item_buffers,
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
            .unwrap_or_else(|| "Enter value...".to_string());

        // Header with count
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&name).color(theme.label_color));
            if required {
                ui.label(egui::RichText::new("*").color(theme.error));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!("{} items", self.item_buffers.len()))
                        .small()
                        .color(theme.hint_color),
                );
            });
        });

        ui.add_space(2.0);

        let options = self.parameter.options.clone();
        let allow_reorder = options.as_ref().is_some_and(|o| o.allow_reorder);
        let max_items = options.as_ref().and_then(|o| o.max_items);

        // Track actions to apply after iteration
        let mut to_remove: Option<usize> = None;
        let mut to_move: Option<(usize, usize)> = None;
        let mut any_changed = false;
        let mut add_item = false;

        let item_count = self.item_buffers.len();

        // List items - flat, no container frame
        if self.item_buffers.is_empty() {
            // Empty state
            ui.label(
                egui::RichText::new("No items")
                    .color(theme.hint_color)
                    .italics(),
            );
        } else {
            // List items
            for (index, buffer) in self.item_buffers.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    // Item number
                    ui.label(
                        egui::RichText::new(format!("{}.", index + 1))
                            .color(theme.hint_color)
                            .small(),
                    );

                    // Item value input - flat
                    let edit = egui::TextEdit::singleline(buffer)
                        .hint_text(&placeholder)
                        .desired_width(ui.available_width() - 70.0);

                    if ui.add(edit).changed() {
                        any_changed = true;
                    }

                    // Move buttons (for reordering)
                    if allow_reorder {
                        ui.add_enabled_ui(index > 0, |ui| {
                            if ui.small_button("^").clicked() {
                                to_move = Some((index, index - 1));
                            }
                        });

                        ui.add_enabled_ui(index < item_count - 1, |ui| {
                            if ui.small_button("v").clicked() {
                                to_move = Some((index, index + 1));
                            }
                        });
                    }

                    // Remove button
                    if ui.small_button("x").clicked() {
                        to_remove = Some(index);
                    }
                });
            }
        }

        // Add item button
        let can_add = max_items.is_none_or(|max| item_count < max);
        if can_add {
            ui.add_space(4.0);
            if ui.small_button("+ Add item").clicked() {
                add_item = true;
            }
        }

        // Handle item removal
        if let Some(index) = to_remove {
            self.item_buffers.remove(index);
            response.changed = true;
        }

        // Handle item movement
        if let Some((from, to)) = to_move {
            self.item_buffers.swap(from, to);
            response.changed = true;
        }

        // Handle add item
        if add_item {
            self.item_buffers.push(String::new());
            response.changed = true;
        }

        // Handle any content changes
        if any_changed {
            response.changed = true;
        }

        // Constraints info
        if let Some(opts) = &options {
            let mut constraints = Vec::new();
            if let Some(min) = opts.min_items {
                constraints.push(format!("min: {}", min));
            }
            if let Some(max) = opts.max_items {
                constraints.push(format!("max: {}", max));
            }
            if !constraints.is_empty() {
                ui.label(
                    egui::RichText::new(constraints.join(", "))
                        .small()
                        .color(theme.hint_color),
                );
            }
        }

        // Hint (help text below field)
        if let Some(hint_text) = hint {
            if !hint_text.is_empty() {
                ui.label(
                    egui::RichText::new(&hint_text)
                        .small()
                        .color(theme.hint_color),
                );
            }
        }

        // Error
        if let Some(ref error) = response.error {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(error).small().color(theme.error));
        }

        response
    }
}

impl ListWidget {
    /// Get the current list as an array of values.
    #[must_use]
    pub fn to_array(&self) -> nebula_value::Array {
        let values: Vec<nebula_value::Value> = self
            .item_buffers
            .iter()
            .map(|s| nebula_value::Value::text(s))
            .collect();

        nebula_value::Array::from_nebula_values(values)
    }

    /// Get the current list values.
    #[must_use]
    pub fn values(&self) -> &[String] {
        &self.item_buffers
    }

    /// Get the number of items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.item_buffers.len()
    }

    /// Check if the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.item_buffers.is_empty()
    }

    /// Add an item to the list.
    pub fn add_item(&mut self, value: &str) {
        self.item_buffers.push(value.to_string());
    }

    /// Remove an item by index.
    pub fn remove_item(&mut self, index: usize) -> Option<String> {
        if index < self.item_buffers.len() {
            Some(self.item_buffers.remove(index))
        } else {
            None
        }
    }

    /// Clear all items.
    pub fn clear(&mut self) {
        self.item_buffers.clear();
    }
}

/// Convert a nebula_value::Value to string.
fn value_to_string(value: &nebula_value::Value) -> String {
    match value {
        nebula_value::Value::Text(t) => t.to_string(),
        nebula_value::Value::Integer(i) => i.value().to_string(),
        nebula_value::Value::Float(f) => f.value().to_string(),
        nebula_value::Value::Boolean(b) => b.to_string(),
        nebula_value::Value::Null => String::new(),
        _ => format!("{:?}", value),
    }
}
