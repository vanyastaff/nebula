//! Group widget for GroupParameter.

use crate::{ParameterTheme, ParameterWidget, UiExt, WidgetResponse};
use egui::Ui;
use nebula_parameter::core::{Describable, Parameter};
use nebula_parameter::types::{GroupFieldType, GroupParameter, GroupValue};

/// Widget for grouped parameter fields.
pub struct GroupWidget {
    parameter: GroupParameter,
    field_buffers: std::collections::HashMap<String, String>,
    group_value: GroupValue,
    collapsed: bool,
}

impl ParameterWidget for GroupWidget {
    type Parameter = GroupParameter;

    fn new(parameter: Self::Parameter) -> Self {
        let mut field_buffers = std::collections::HashMap::new();
        let mut group_value = GroupValue::new();

        // Initialize from default value if present
        if let Some(default) = &parameter.default {
            for field in &parameter.fields {
                if let Some(val) = default.get_field(&field.key) {
                    field_buffers.insert(field.key.clone(), value_to_string(&val));
                    group_value.set_field(&field.key, val);
                }
            }
        }

        // Fill in missing fields with their defaults
        for field in &parameter.fields {
            if !field_buffers.contains_key(&field.key) {
                let default_str = field
                    .default_value
                    .as_ref()
                    .map(value_to_string)
                    .unwrap_or_default();
                field_buffers.insert(field.key.clone(), default_str);

                if let Some(default_val) = &field.default_value {
                    group_value.set_field(&field.key, default_val.clone());
                }
            }
        }

        Self {
            parameter,
            field_buffers,
            group_value,
            collapsed: false,
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

        ui.themed_label(theme, &name);

        if !description.is_empty() {
            ui.themed_hint(theme, &description);
        }

        ui.add_space(4.0);

        egui::Frame::none()
            .stroke(egui::Stroke::new(1.0, theme.input_border))
            .corner_radius(theme.border_radius)
            .inner_margin(12.0)
            .show(ui, |ui| {
                self.show_fields(ui, theme, &mut response);
            });

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
                let label = if field.required {
                    format!("{}*", field.name)
                } else {
                    field.name.clone()
                };
                ui.label(&label);

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
                    self.update_group_value(&field.key, &field.field_type);
                    response.changed = true;
                }
            });

            if let Some(desc) = &field.description {
                ui.themed_hint(theme, desc);
            }

            ui.add_space(4.0);
        }
    }

    fn update_group_value(&mut self, key: &str, field_type: &GroupFieldType) {
        if let Some(buffer) = self.field_buffers.get(key) {
            let value = string_to_value(buffer, field_type);
            self.group_value.set_field(key, value);
        }
    }

    /// Get the current group value.
    #[must_use]
    pub fn value(&self) -> &GroupValue {
        &self.group_value
    }

    #[must_use]
    pub fn get_field(&self, key: &str) -> Option<&String> {
        self.field_buffers.get(key)
    }

    pub fn set_field(&mut self, key: &str, value: &str) {
        self.field_buffers
            .insert(key.to_string(), value.to_string());
    }

    #[must_use]
    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

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
