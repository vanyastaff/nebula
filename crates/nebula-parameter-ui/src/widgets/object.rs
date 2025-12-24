//! Object widget for ObjectParameter.
//!
//! N8N-style nested object editor with collapsible sections and "Add Parameter" functionality.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{RichText, Ui};
use egui_flex::{Flex, FlexAlign, item};
use egui_phosphor::regular::{CARET_DOWN, CARET_RIGHT, PLUS, TRASH};
use nebula_parameter::core::{HasValue, Parameter, ParameterKind};
use nebula_parameter::types::{ObjectParameter, ObjectValue};
use std::collections::HashSet;

/// Widget for structured object data with n8n-style UI.
///
/// Features:
/// - Required parameters are always shown
/// - Optional parameters can be added via "Add Parameter" button
/// - Added optional parameters can be removed with trash icon
pub struct ObjectWidget {
    parameter: ObjectParameter,
    /// Whether the main section is expanded
    expanded: bool,
    /// Which optional parameters have been added (by key)
    added_optional: HashSet<String>,
    /// Text buffers for field editing (key -> value)
    field_buffers: std::collections::HashMap<String, String>,
    /// Checkbox states (key -> checked)
    checkbox_states: std::collections::HashMap<String, bool>,
    /// Focus state per field
    field_focus: std::collections::HashMap<String, bool>,
    /// Whether the "Add Parameter" popup is open
    add_popup_open: bool,
}

impl ParameterWidget for ObjectWidget {
    type Parameter = ObjectParameter;

    fn new(parameter: Self::Parameter) -> Self {
        let mut field_buffers = std::collections::HashMap::new();
        let mut checkbox_states = std::collections::HashMap::new();

        // Initialize buffers from current value
        if let Some(value) = parameter.get() {
            for (key, val) in value.entries() {
                match val {
                    nebula_value::Value::Boolean(b) => {
                        checkbox_states.insert(key.clone(), *b);
                    }
                    _ => {
                        field_buffers.insert(key.clone(), value_to_string(val));
                    }
                }
            }
        }

        // Initialize from child parameter definitions
        for (key, child) in parameter.children() {
            match child.kind() {
                ParameterKind::Checkbox => {
                    checkbox_states.entry(key.clone()).or_insert(false);
                }
                _ => {
                    field_buffers.entry(key.clone()).or_default();
                }
            }
        }

        Self {
            parameter,
            expanded: true,
            added_optional: HashSet::new(),
            field_buffers,
            checkbox_states,
            field_focus: std::collections::HashMap::new(),
            add_popup_open: false,
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

        Flex::vertical()
            .w_full()
            .align_items(FlexAlign::Start)
            .gap(egui::vec2(0.0, theme.spacing_sm))
            .show(ui, |flex| {
                // Header with expand/collapse
                flex.add_ui(item().grow(1.0), |ui| {
                    self.show_header(ui, theme, &name);
                });

                if self.expanded {
                    // Children container
                    flex.add_ui(item().grow(1.0), |ui| {
                        self.show_children(ui, theme, &mut response);
                    });

                    // "Add Parameter" button
                    flex.add_ui(item().grow(1.0), |ui| {
                        self.show_add_parameter_button(ui, theme);
                    });
                }

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

impl ObjectWidget {
    /// Show collapsible section header.
    fn show_header(&mut self, ui: &mut Ui, theme: &ParameterTheme, name: &str) {
        let icon = if self.expanded {
            CARET_DOWN
        } else {
            CARET_RIGHT
        };

        ui.horizontal(|ui| {
            // Expand/collapse button
            let btn = egui::Button::new(RichText::new(icon).size(14.0).color(theme.hint_color))
                .frame(false);

            if ui.add(btn).clicked() {
                self.expanded = !self.expanded;
            }

            // Section name (bold)
            ui.label(
                RichText::new(name)
                    .size(theme.label_font_size)
                    .color(theme.label_color)
                    .strong(),
            );

            // Field count badge - show visible count
            let visible_count = self.get_visible_field_count();
            if visible_count > 0 {
                ui.label(
                    RichText::new(format!("({})", visible_count))
                        .size(theme.hint_font_size)
                        .color(theme.hint_color),
                );
            }
        });
    }

    /// Get count of currently visible fields (required + added optional).
    fn get_visible_field_count(&self) -> usize {
        let required_count = self.parameter.get_required_children().count();
        let added_count = self.added_optional.len();
        required_count + added_count
    }

    /// Get list of optional parameters that haven't been added yet.
    fn get_available_optional(&self) -> Vec<(String, String)> {
        self.parameter
            .get_optional_children()
            .filter(|(key, _)| !self.added_optional.contains(*key))
            .map(|(key, child)| (key.clone(), child.metadata().name.clone()))
            .collect()
    }

    /// Show all child parameters (required + added optional).
    fn show_children(
        &mut self,
        ui: &mut Ui,
        theme: &ParameterTheme,
        response: &mut WidgetResponse,
    ) {
        // Collect required keys
        let required_keys: Vec<String> = self
            .parameter
            .get_required_children()
            .map(|(k, _)| k.clone())
            .collect();

        // Collect added optional keys
        let optional_keys: Vec<String> = self.added_optional.iter().cloned().collect();

        let has_fields = !required_keys.is_empty() || !optional_keys.is_empty();

        if !has_fields {
            // Empty state
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(
                    RichText::new("No parameters configured")
                        .size(theme.hint_font_size)
                        .color(theme.hint_color)
                        .italics(),
                );
            });
            return;
        }

        // Container with left border (n8n style indent)
        let indent = 16.0;

        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: indent as i8,
                ..Default::default()
            })
            .show(ui, |ui| {
                // Draw left border line
                let rect = ui.available_rect_before_wrap();
                ui.painter().vline(
                    rect.left() + 4.0,
                    rect.top()..=rect.bottom(),
                    egui::Stroke::new(1.0, theme.input_border),
                );

                // Show required parameters first
                for key in &required_keys {
                    self.show_child_field(ui, theme, key, false, response);
                    ui.add_space(theme.spacing_sm);
                }

                // Show added optional parameters
                for key in &optional_keys {
                    self.show_child_field(ui, theme, key, true, response);
                    ui.add_space(theme.spacing_sm);
                }
            });
    }

    /// Show a single child field based on its ParameterKind.
    fn show_child_field(
        &mut self,
        ui: &mut Ui,
        theme: &ParameterTheme,
        key: &str,
        is_removable: bool,
        response: &mut WidgetResponse,
    ) {
        let child = match self.parameter.children().get(key) {
            Some(c) => c,
            None => return,
        };

        let child_name = child.metadata().name.clone();
        let child_kind = child.kind();
        let is_required = child.metadata().required;

        // Field label row with optional remove button
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&child_name)
                    .size(theme.label_font_size)
                    .color(theme.label_color)
                    .strong(),
            );

            if is_required {
                ui.label(
                    RichText::new("*")
                        .size(theme.label_font_size)
                        .color(theme.error),
                );
            }

            // Remove button for optional parameters
            if is_removable {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let trash_btn =
                        egui::Button::new(RichText::new(TRASH).size(14.0).color(theme.hint_color))
                            .frame(false);

                    if ui
                        .add(trash_btn)
                        .on_hover_text("Remove parameter")
                        .clicked()
                    {
                        self.added_optional.remove(key);
                        self.field_buffers.remove(key);
                        self.checkbox_states.remove(key);
                        // Clear value from parameter
                        if let Some(value) = self.parameter.get_mut() {
                            value.remove_field(key);
                        }
                        response.changed = true;
                    }
                });
            }
        });

        // Field input based on kind
        match child_kind {
            ParameterKind::Text | ParameterKind::Secret => {
                self.show_text_field(ui, theme, key, response);
            }
            ParameterKind::Number => {
                self.show_number_field(ui, theme, key, response);
            }
            ParameterKind::Checkbox => {
                self.show_checkbox_field(ui, theme, key, response);
            }
            ParameterKind::Select => {
                self.show_text_field(ui, theme, key, response); // TODO: proper select
            }
            _ => {
                // Default: text input
                self.show_text_field(ui, theme, key, response);
            }
        }
    }

    /// Show a simple text field.
    fn show_text_field(
        &mut self,
        ui: &mut Ui,
        theme: &ParameterTheme,
        key: &str,
        response: &mut WidgetResponse,
    ) {
        let width = ui.available_width();
        let buffer = self.field_buffers.entry(key.to_string()).or_default();
        let focused = *self.field_focus.get(key).unwrap_or(&false);

        let frame = theme.input_frame(focused, false);
        let mut changed = false;
        let mut new_focus = false;

        frame.show(ui, |ui| {
            ui.set_width(width - 20.0);

            let edit = egui::TextEdit::singleline(buffer)
                .frame(false)
                .desired_width(ui.available_width());

            let edit_response = ui.add(edit);
            new_focus = edit_response.has_focus();

            if edit_response.changed() {
                changed = true;
            }
        });

        self.field_focus.insert(key.to_string(), new_focus);

        if changed {
            self.update_text_field_value(key, response);
        }
    }

    /// Show a number field.
    fn show_number_field(
        &mut self,
        ui: &mut Ui,
        theme: &ParameterTheme,
        key: &str,
        response: &mut WidgetResponse,
    ) {
        let width = ui.available_width();
        let buffer = self.field_buffers.entry(key.to_string()).or_default();
        let focused = *self.field_focus.get(key).unwrap_or(&false);

        let frame = theme.input_frame(focused, false);
        let mut changed = false;
        let mut new_focus = false;

        frame.show(ui, |ui| {
            ui.set_width(width - 20.0);

            let edit = egui::TextEdit::singleline(buffer)
                .frame(false)
                .desired_width(ui.available_width());

            let edit_response = ui.add(edit);
            new_focus = edit_response.has_focus();

            if edit_response.changed() {
                changed = true;
            }
        });

        self.field_focus.insert(key.to_string(), new_focus);

        if changed {
            self.update_number_field_value(key, response);
        }
    }

    /// Show a checkbox field.
    fn show_checkbox_field(
        &mut self,
        ui: &mut Ui,
        _theme: &ParameterTheme,
        key: &str,
        response: &mut WidgetResponse,
    ) {
        let mut checked = *self.checkbox_states.get(key).unwrap_or(&false);
        let _old_checked = checked;

        if ui.checkbox(&mut checked, "").changed() {
            self.checkbox_states.insert(key.to_string(), checked);
            self.update_checkbox_field_value(key, checked, response);
        }
    }

    /// Show "Add Parameter" button with popup.
    fn show_add_parameter_button(&mut self, ui: &mut Ui, theme: &ParameterTheme) {
        let available_optional = self.get_available_optional();

        if available_optional.is_empty() {
            return; // No more parameters to add
        }

        let popup_id = ui.make_persistent_id("add_param_popup");

        // "Add Parameter" button
        let btn = egui::Button::new(
            RichText::new(format!("{} Add Parameter", PLUS))
                .size(theme.label_font_size)
                .color(theme.primary),
        )
        .frame(false);

        let btn_response = ui.add(btn);

        if btn_response.clicked() {
            self.add_popup_open = !self.add_popup_open;
        }

        // Show popup below button
        if self.add_popup_open {
            egui::Area::new(popup_id)
                .fixed_pos(btn_response.rect.left_bottom() + egui::vec2(0.0, 4.0))
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    theme.popup_frame().show(ui, |ui| {
                        ui.set_min_width(200.0);

                        for (key, name) in &available_optional {
                            let item_response = ui.selectable_label(false, name);

                            if item_response.clicked() {
                                // Add this parameter
                                self.added_optional.insert(key.clone());
                                self.add_popup_open = false;

                                // Initialize buffer/state
                                if let Some(child) = self.parameter.children().get(key) {
                                    match child.kind() {
                                        ParameterKind::Checkbox => {
                                            self.checkbox_states.insert(key.clone(), false);
                                        }
                                        _ => {
                                            self.field_buffers.insert(key.clone(), String::new());
                                        }
                                    }
                                }
                            }
                        }
                    });
                });

            // Close popup when clicking outside
            if ui.input(|i| i.pointer.any_click()) && !btn_response.hovered() {
                // Check if click was outside popup area
                let popup_rect = egui::Rect::from_min_size(
                    btn_response.rect.left_bottom() + egui::vec2(0.0, 4.0),
                    egui::vec2(200.0, available_optional.len() as f32 * 24.0 + 16.0),
                );

                if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                    if !popup_rect.contains(pos) && !btn_response.rect.contains(pos) {
                        self.add_popup_open = false;
                    }
                }
            }
        }
    }

    /// Update a text field value in the parameter.
    fn update_text_field_value(&mut self, key: &str, response: &mut WidgetResponse) {
        if let Some(buffer) = self.field_buffers.get(key) {
            let value = nebula_value::Value::text(buffer);

            if let Err(e) = self.parameter.set_field_value(key, value) {
                response.error = Some(e.to_string());
            } else {
                response.changed = true;
            }
        }
    }

    /// Update a number field value in the parameter.
    fn update_number_field_value(&mut self, key: &str, response: &mut WidgetResponse) {
        if let Some(buffer) = self.field_buffers.get(key) {
            let value = if let Ok(num) = buffer.parse::<f64>() {
                nebula_value::Value::float(num)
            } else if let Ok(num) = buffer.parse::<i64>() {
                nebula_value::Value::integer(num)
            } else {
                // Keep as text if not parseable
                nebula_value::Value::text(buffer)
            };

            if let Err(e) = self.parameter.set_field_value(key, value) {
                response.error = Some(e.to_string());
            } else {
                response.changed = true;
            }
        }
    }

    /// Update a checkbox field value in the parameter.
    fn update_checkbox_field_value(
        &mut self,
        key: &str,
        checked: bool,
        response: &mut WidgetResponse,
    ) {
        let value = nebula_value::Value::boolean(checked);

        if let Err(e) = self.parameter.set_field_value(key, value) {
            response.error = Some(e.to_string());
        } else {
            response.changed = true;
        }
    }

    /// Get the current object value.
    #[must_use]
    pub fn value(&self) -> Option<&ObjectValue> {
        self.parameter.get()
    }

    /// Check if expanded.
    #[must_use]
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// Set expanded state.
    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }

    /// Check if a specific optional parameter has been added.
    #[must_use]
    pub fn is_optional_added(&self, key: &str) -> bool {
        self.added_optional.contains(key)
    }

    /// Programmatically add an optional parameter.
    pub fn add_optional(&mut self, key: &str) {
        if let Some(child) = self.parameter.children().get(key) {
            if !child.metadata().required {
                self.added_optional.insert(key.to_string());

                match child.kind() {
                    ParameterKind::Checkbox => {
                        self.checkbox_states.insert(key.to_string(), false);
                    }
                    _ => {
                        self.field_buffers.insert(key.to_string(), String::new());
                    }
                }
            }
        }
    }

    /// Programmatically remove an optional parameter.
    pub fn remove_optional(&mut self, key: &str) {
        self.added_optional.remove(key);
        self.field_buffers.remove(key);
        self.checkbox_states.remove(key);
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
