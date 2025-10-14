use egui::{Response, Ui, RichText, TextEdit, DragValue, Checkbox};
use nebula_parameter::{GroupParameter, GroupField, GroupFieldType, GroupValue};
use nebula_value::Value;
use crate::{
    ParameterWidget, ParameterTheme, ParameterContext, ValidationState,
};
use std::collections::HashMap;

/// Widget for grouped parameters with structured fields
#[derive(Debug, Clone)]
pub struct GroupWidget<'a> {
    parameter: GroupParameter,
    context: ParameterContext<'a>,
    field_values: HashMap<String, Value>,
}

impl<'a> GroupWidget<'a> {
    pub fn new(parameter: GroupParameter) -> Self {
        let field_values = if let Some(value) = &parameter.value {
            value.fields().clone()
        } else {
            HashMap::new()
        };

        Self {
            parameter,
            context: ParameterContext::default(),
            field_values,
        }
    }

    pub fn with_context(mut self, context: ParameterContext) -> Self {
        self.context = context;
        self
    }

    fn render_field(&mut self, ui: &mut Ui, field: &GroupField, theme: &ParameterTheme) -> bool {
        let mut changed = false;
        
        ui.vertical(|ui| {
            // Field label
            ui.horizontal(|ui| {
                let label_text = if field.required {
                    format!("{} *", field.name)
                } else {
                    field.name.clone()
                };

                ui.label(
                    RichText::new(label_text)
                        .color(theme.colors.label)
                        .font(theme.fonts.label.clone())
                );

                if field.required {
                    ui.label(
                        RichText::new("*")
                            .color(theme.colors.required)
                            .font(theme.fonts.label.clone())
                    );
                }
            });

            ui.add_space(4.0);

            // Field input based on type
            match &field.field_type {
                GroupFieldType::Text | GroupFieldType::Email | GroupFieldType::Url => {
                    let mut text_value = self.field_values
                        .get(&field.key)
                        .and_then(|v| v.as_text().ok())
                        .map(|t| t.to_string())
                        .unwrap_or_default();

                    let response = ui.add(
                        TextEdit::singleline(&mut text_value)
                            .hint_text(&format!("Enter {}...", field.name.to_lowercase()))
                    );

                    if response.changed() {
                        self.field_values.insert(
                            field.key.clone(),
                            Value::from(nebula_value::Text::new(&text_value))
                        );
                        changed = true;
                    }
                }

                GroupFieldType::Number => {
                    let mut number_value = self.field_values
                        .get(&field.key)
                        .and_then(|v| v.as_f64().ok())
                        .unwrap_or(0.0);

                    let response = ui.add(
                        DragValue::new(&mut number_value)
                            .speed(0.1)
                    );

                    if response.changed() {
                        self.field_values.insert(
                            field.key.clone(),
                            Value::from(number_value)
                        );
                        changed = true;
                    }
                }

                GroupFieldType::Boolean => {
                    let mut bool_value = self.field_values
                        .get(&field.key)
                        .and_then(|v| v.as_bool().ok())
                        .unwrap_or(false);

                    let response = ui.add(
                        Checkbox::new(&mut bool_value, "")
                    );

                    if response.changed() {
                        self.field_values.insert(
                            field.key.clone(),
                            Value::from(nebula_value::Boolean::new(bool_value))
                        );
                        changed = true;
                    }
                }

                GroupFieldType::Select { options } => {
                    let current_value = self.field_values
                        .get(&field.key)
                        .and_then(|v| v.as_text().ok())
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "Select...".to_string());

                    egui::ComboBox::from_id_source(&format!("group_select_{}", field.key))
                        .selected_text(&current_value)
                        .show_ui(ui, |ui| {
                            for option in options {
                                if ui.selectable_label(&current_value == option, option).clicked() {
                                    self.field_values.insert(
                                        field.key.clone(),
                                        Value::from(nebula_value::Text::new(option))
                                    );
                                    changed = true;
                                }
                            }
                        });
                }

                GroupFieldType::Date => {
                    let mut date_value = self.field_values
                        .get(&field.key)
                        .and_then(|v| v.as_text().ok())
                        .map(|t| t.to_string())
                        .unwrap_or_default();

                    let response = ui.add(
                        TextEdit::singleline(&mut date_value)
                            .hint_text("YYYY-MM-DD")
                    );

                    if response.changed() {
                        self.field_values.insert(
                            field.key.clone(),
                            Value::from(nebula_value::Text::new(&date_value))
                        );
                        changed = true;
                    }
                }
            }

            // Field description
            if let Some(description) = &field.description {
                ui.add_space(2.0);
                ui.label(
                    RichText::new(description)
                        .color(theme.colors.description)
                        .font(theme.fonts.description.clone())
                );
            }
        });

        changed
    }
}

impl<'a> ParameterWidget for GroupWidget<'a> {
    fn render(&mut self, ui: &mut Ui) -> Response {
        self.render_with_theme(ui, &ParameterTheme::default())
    }

    fn render_with_theme(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> Response {
        let mut response = ui.allocate_response(ui.available_size(), egui::Sense::click());
        let mut changed = false;

        ui.vertical(|ui| {
            // Group header
            ui.horizontal(|ui| {
                let label_text = if self.parameter.metadata.required {
                    format!("{} *", self.parameter.metadata.name)
                } else {
                    self.parameter.metadata.name.clone()
                };

                ui.label(
                    RichText::new(label_text)
                        .color(theme.colors.label)
                        .font(theme.fonts.label.clone())
                        .strong()
                );

                if self.parameter.metadata.required {
                    ui.label(
                        RichText::new("*")
                            .color(theme.colors.required)
                            .font(theme.fonts.label.clone())
                    );
                }
            });

            // Description
            if let Some(description) = &self.parameter.metadata.description {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(description)
                        .color(theme.colors.description)
                        .font(theme.fonts.description.clone())
                );
            }

            ui.add_space(8.0);

            // Group fields in a frame
            let frame = egui::Frame::none()
                .fill(theme.colors.background)
                .stroke(egui::Stroke::new(1.0, theme.colors.border))
                .inner_margin(egui::Margin::symmetric(12.0, 8.0));

            frame.show(ui, |ui| {
                for (i, field) in self.parameter.fields.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);
                    }

                    if self.render_field(ui, field, theme) {
                        changed = true;
                    }
                }
            });

            // Update parameter value if changed
            if changed {
                self.parameter.value = Some(GroupValue::new(self.field_values.clone()));
                response.mark_changed();
            }

            // Validation state
            if let Some(validation) = &self.parameter.validation {
                if !validation.is_valid() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(validation.error_message())
                            .color(theme.colors.error)
                            .font(theme.fonts.error.clone())
                    );
                }
            }
        });

        response
    }
}

/// Helper function to create a group widget
pub fn group_widget(parameter: GroupParameter) -> GroupWidget<'static> {
    GroupWidget::new(parameter)
}
