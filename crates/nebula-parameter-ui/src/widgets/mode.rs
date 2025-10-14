use egui::{Response, Ui, RichText, ComboBox, ScrollArea};
use nebula_parameter::{ModeParameter, ModeItem, ModeValue};
use crate::{
    ParameterWidget, ParameterTheme, ParameterContext, ValidationState,
};

/// Widget for mode selection with dynamic parameter switching
#[derive(Debug, Clone)]
pub struct ModeWidget<'a> {
    parameter: ModeParameter,
    context: ParameterContext<'a>,
}

impl<'a> ModeWidget<'a> {
    pub fn new(parameter: ModeParameter) -> Self {
        Self {
            parameter,
            context: ParameterContext::default(),
        }
    }

    pub fn with_context(mut self, context: ParameterContext) -> Self {
        self.context = context;
        self
    }

    fn get_current_mode(&self) -> Option<&ModeItem> {
        let current_key = self.parameter.value.as_ref()?.key();
        self.parameter.modes.iter().find(|mode| mode.key == current_key)
    }

    fn get_mode_display_name(&self, mode: &ModeItem) -> String {
        if mode.description.is_some() {
            format!("{} - {}", mode.name, mode.description.as_ref().unwrap())
        } else {
            mode.name.clone()
        }
    }
}

impl<'a> ParameterWidget for ModeWidget<'a> {
    fn render(&mut self, ui: &mut Ui) -> Response {
        self.render_with_theme(ui, &ParameterTheme::default())
    }

    fn render_with_theme(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> Response {
        let mut response = ui.allocate_response(ui.available_size(), egui::Sense::click());

        ui.vertical(|ui| {
            // Mode selection dropdown
            ui.horizontal(|ui| {
                // Label
                let label_text = if self.parameter.metadata.required {
                    format!("{} *", self.parameter.metadata.name)
                } else {
                    self.parameter.metadata.name.clone()
                };

                ui.label(
                    RichText::new(label_text)
                        .color(theme.colors.label)
                        .font(theme.fonts.label.clone())
                );

                if self.parameter.metadata.required {
                    ui.label(
                        RichText::new("*")
                            .color(theme.colors.required)
                            .font(theme.fonts.label.clone())
                    );
                }
            });

            // Mode selector
            let current_mode = self.get_current_mode();
            let current_mode_name = current_mode
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "Select mode...".to_string());

            let mut changed = false;
            ComboBox::from_id_source(&format!("mode_{}", self.parameter.metadata.key))
                .selected_text(&current_mode_name)
                .show_ui(ui, |ui| {
                    for mode in &self.parameter.modes {
                        let mode_text = self.get_mode_display_name(mode);
                        let is_selected = current_mode.map(|m| m.key == mode.key).unwrap_or(false);
                        
                        if ui.selectable_label(is_selected, &mode_text).clicked() {
                            self.parameter.value = Some(ModeValue::new(mode.key.clone()));
                            changed = true;
                        }
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

            // Mode-specific parameters
            if let Some(mode) = current_mode {
                ui.add_space(8.0);
                
                // Mode parameters section
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{} Parameters", mode.name))
                                .color(theme.colors.label)
                                .strong()
                        );
                    });

                    ui.add_space(4.0);

                    // Note: In a real implementation, you would render the mode's parameters here
                    // This requires access to parameter widgets and a way to dynamically render them
                    // For now, we'll show a placeholder
                    ui.label(
                        RichText::new(format!("Parameters for mode: {}", mode.name))
                            .color(theme.colors.description)
                            .italic()
                    );

                    if let Some(mode_description) = &mode.description {
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(mode_description)
                                .color(theme.colors.description)
                                .font(theme.fonts.description.clone())
                        );
                    }
                });
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

        if changed {
            response.mark_changed();
        }

        response
    }
}

/// Helper function to create a mode widget
pub fn mode_widget(parameter: ModeParameter) -> ModeWidget<'static> {
    ModeWidget::new(parameter)
}
