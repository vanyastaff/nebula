//! Group widget for GroupParameter.

use crate::{ParameterTheme, ParameterWidget, UiExt, WidgetResponse};
use egui::Ui;
use nebula_parameter::core::{HasValue, Parameter};
use nebula_parameter::types::{GroupFieldType, GroupParameter, GroupValue};

/// Widget for grouped parameter fields.
pub struct GroupWidget {
    parameter: GroupParameter,
    /// Current field values (string representations for editing)
    field_buffers: std::collections::HashMap<String, String>,
    /// Whether the group is collapsed
    collapsed: bool,
}

impl ParameterWidget for GroupWidget {
    type Parameter = GroupParameter;

    fn new(parameter: Self::Parameter) -> Self {
        // Initialize buffers from current value or defaults
        let mut field_buffers = std::collections::HashMap::new();

        if let Some(value) = parameter.get() {
            for field in &parameter.fields {
                if let Some(val) = value.get_field(&field.key) {
                    field_buffers.insert(field.key.clone(), value_to_string(&val));
                }
            }
        }

        // Fill in missing fields with defaults
        for field in &parameter.fields {
            if !field_buffers.contains_key(&field.key) {
                let default_str = field
                    .default_value
                    .as_ref()
                    .map(value_to_string)
                    .unwrap_or_default();
                field_buffers.insert(field.key.clone(), default_str);
            }
        }

        let collapsed = parameter
            .options
            .as_ref()
            .is_some_and(|o| o.start_collapsed);

        Self {
            parameter,
            field_buffers,
            collapsed,
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

        // Clone metadata fields to avoid borrow conflicts
        let metadata = self.parameter.metadata();
        let name = metadata.name.clone();
        let description = metadata.description.clone();
        let is_collapsible = self
            .parameter
            .options
            .as_ref()
            .is_some_and(|o| o.collapsible);

        if is_collapsible {
            let header_response = ui.collapsing(&name, |ui| {
                self.show_fields(ui, theme, &mut response);
            });
            self.collapsed = !header_response.fully_open();
        } else {
            ui.themed_label(theme, &name);

            // Description
            if !description.is_empty() {
                ui.themed_hint(theme, &description);
            }

            ui.add_space(4.0);

            // Group frame
            egui::Frame::none()
                .stroke(egui::Stroke::new(1.0, theme.input_border))
                .rounding(theme.border_radius)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    self.show_fields(ui, theme, &mut response);
                });
        }

        // Show validation error
        if let Some(ref error) = response.error {
            ui.themed_error(theme, error);
        }

        response
    }
}

impl GroupWidget {
    fn show_fields(&mut self, ui: &mut Ui, theme: &ParameterTheme, response: &mut WidgetResponse) {
        let fields = self.parameter.fields.clone();

        for field in &fields {
            ui.horizontal(|ui| {
                // Field label
                let label = if field.required {
                    format!("{}*", field.name)
                } else {
                    field.name.clone()
                };
                ui.label(&label);

                // Field input based on type
                let buffer = self.field_buffers.entry(field.key.clone()).or_default();

                let changed = match &field.field_type {
                    GroupFieldType::Text | GroupFieldType::Email | GroupFieldType::Url => {
                        let edit = egui::TextEdit::singleline(buffer)
                            .hint_text(field.description.as_deref().unwrap_or(""));
                        ui.add(edit).changed()
                    }
                    GroupFieldType::Number => {
                        let mut num: f64 = buffer.parse().unwrap_or(0.0);
                        let changed = ui.add(egui::DragValue::new(&mut num)).changed();
                        if changed {
                            *buffer = num.to_string();
                        }
                        changed
                    }
                    GroupFieldType::Boolean => {
                        let mut checked = buffer == "true";
                        let changed = ui.checkbox(&mut checked, "").changed();
                        if changed {
                            *buffer = checked.to_string();
                        }
                        changed
                    }
                    GroupFieldType::Select { options } => {
                        let mut changed = false;
                        egui::ComboBox::from_id_salt(&field.key)
                            .selected_text(buffer.as_str())
                            .show_ui(ui, |ui| {
                                for opt in options {
                                    if ui.selectable_label(buffer == opt, opt).clicked() {
                                        *buffer = opt.clone();
                                        changed = true;
                                    }
                                }
                            });
                        changed
                    }
                    GroupFieldType::Date => {
                        let edit = egui::TextEdit::singleline(buffer).hint_text("YYYY-MM-DD");
                        ui.add(edit).changed()
                    }
                };

                if changed {
                    self.update_parameter_value(response);
                }
            });

            // Field description
            if let Some(desc) = &field.description {
                ui.themed_hint(theme, desc);
            }

            ui.add_space(4.0);
        }
    }

    fn update_parameter_value(&mut self, response: &mut WidgetResponse) {
        let mut group_value = GroupValue::new();

        for field in &self.parameter.fields {
            if let Some(buffer) = self.field_buffers.get(&field.key) {
                let value = string_to_value(buffer, &field.field_type);
                group_value.set_field(&field.key, value);
            }
        }

        if let Err(e) = self.parameter.set(group_value) {
            response.error = Some(e.to_string());
        } else {
            response.changed = true;
        }
    }

    /// Get the current group value.
    #[must_use]
    pub fn value(&self) -> Option<&GroupValue> {
        self.parameter.get()
    }

    /// Get a field value by key.
    #[must_use]
    pub fn get_field(&self, key: &str) -> Option<&String> {
        self.field_buffers.get(key)
    }

    /// Set a field value by key.
    pub fn set_field(&mut self, key: &str, value: &str) {
        self.field_buffers
            .insert(key.to_string(), value.to_string());
    }

    /// Check if the group is collapsed.
    #[must_use]
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

/// Convert a nebula_value::Value to string for editing.
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

/// Convert a string back to nebula_value::Value based on field type.
fn string_to_value(s: &str, field_type: &GroupFieldType) -> nebula_value::Value {
    match field_type {
        GroupFieldType::Text
        | GroupFieldType::Email
        | GroupFieldType::Url
        | GroupFieldType::Date => nebula_value::Value::text(s),
        GroupFieldType::Number => {
            if let Ok(n) = s.parse::<f64>() {
                nebula_value::Value::Float(nebula_value::Float::new(n))
            } else {
                nebula_value::Value::integer(0)
            }
        }
        GroupFieldType::Boolean => nebula_value::Value::boolean(s == "true"),
        GroupFieldType::Select { .. } => nebula_value::Value::text(s),
    }
}
