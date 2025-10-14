use egui::{Response, Ui, RichText, ProgressBar, Color32};
use chrono::{DateTime, Utc};
use nebula_parameter::{ExpirableParameter, ExpirableValue, ExpirableParameterOptions};
use crate::{
    ParameterWidget, ParameterTheme, ParameterContext, ValidationState,
};

/// Widget for expirable parameters with TTL (Time-To-Live)
#[derive(Debug, Clone)]
pub struct ExpirableWidget<'a> {
    parameter: ExpirableParameter,
    context: ParameterContext<'a>,
}

impl<'a> ExpirableWidget<'a> {
    pub fn new(parameter: ExpirableParameter) -> Self {
        Self {
            parameter,
            context: ParameterContext::default(),
        }
    }

    pub fn with_context(mut self, context: ParameterContext) -> Self {
        self.context = context;
        self
    }

    fn get_ttl_seconds(&self) -> u64 {
        self.parameter.options.as_ref()
            .map(|opts| opts.ttl)
            .unwrap_or(3600) // Default 1 hour
    }

    fn is_expired(&self) -> bool {
        if let Some(value) = &self.parameter.value {
            let now = Utc::now();
            value.expires_at() < now
        } else {
            false
        }
    }

    fn get_time_remaining(&self) -> Option<chrono::Duration> {
        if let Some(value) = &self.parameter.value {
            let now = Utc::now();
            let expires_at = value.expires_at();
            if expires_at > now {
                Some(expires_at - now)
            } else {
                Some(chrono::Duration::zero())
            }
        } else {
            None
        }
    }

    fn format_duration(&self, duration: chrono::Duration) -> String {
        let total_seconds = duration.num_seconds();
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;

        if hours > 0 {
            format!("{}h {}m {}s", hours, minutes, seconds)
        } else if minutes > 0 {
            format!("{}m {}s", minutes, seconds)
        } else {
            format!("{}s", seconds)
        }
    }

    fn get_expiration_color(&self, theme: &ParameterTheme) -> Color32 {
        if self.is_expired() {
            theme.colors.error
        } else if let Some(remaining) = self.get_time_remaining() {
            let ttl = self.get_ttl_seconds();
            let remaining_ratio = remaining.num_seconds() as f64 / ttl as f64;
            
            if remaining_ratio < 0.1 {
                theme.colors.error
            } else if remaining_ratio < 0.3 {
                theme.colors.warning
            } else {
                theme.colors.success
            }
        } else {
            theme.colors.info
        }
    }
}

impl<'a> ParameterWidget for ExpirableWidget<'a> {
    fn render(&mut self, ui: &mut Ui) -> Response {
        self.render_with_theme(ui, &ParameterTheme::default())
    }

    fn render_with_theme(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> Response {
        let mut response = ui.allocate_response(ui.available_size(), egui::Sense::click());

        ui.vertical(|ui| {
            // Header with expiration status
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

                // Expiration indicator
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let expiration_color = self.get_expiration_color(theme);
                    let status_text = if self.is_expired() {
                        "EXPIRED"
                    } else if let Some(remaining) = self.get_time_remaining() {
                        &self.format_duration(remaining)
                    } else {
                        "No value"
                    };

                    ui.label(
                        RichText::new(status_text)
                            .color(expiration_color)
                            .font(theme.fonts.description.clone())
                    );
                });
            });

            // Progress bar showing expiration
            if let Some(remaining) = self.get_time_remaining() {
                let ttl = self.get_ttl_seconds();
                let progress = if ttl > 0 {
                    (remaining.num_seconds() as f32 / ttl as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };

                let progress_color = self.get_expiration_color(theme);
                let progress_bar = ProgressBar::new(progress)
                    .fill(progress_color.gamma_multiply(0.7))
                    .show_percentage();
                
                ui.add(progress_bar);
            }

            // Value display
            ui.add_space(4.0);
            if let Some(value) = &self.parameter.value {
                if self.is_expired() {
                    ui.label(
                        RichText::new("Value has expired")
                            .color(theme.colors.error)
                            .italic()
                    );
                } else {
                    ui.label(
                        RichText::new(format!("Value: {}", value.value()))
                            .color(theme.colors.label)
                    );
                }

                // Expiration details
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Created: {} | Expires: {}",
                        value.created_at().format("%H:%M:%S"),
                        value.expires_at().format("%H:%M:%S")
                    ))
                    .color(theme.colors.description)
                    .font(theme.fonts.description.clone())
                );
            } else {
                ui.label(
                    RichText::new("No value set")
                        .color(theme.colors.placeholder)
                        .italic()
                );
            }

            // Description
            if let Some(description) = &self.parameter.metadata.description {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(description)
                        .color(theme.colors.description)
                        .font(theme.fonts.description.clone())
                );
            }

            // TTL information
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!("TTL: {} seconds", self.get_ttl_seconds()))
                    .color(theme.colors.hint)
                    .font(theme.fonts.hint.clone())
            );

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

/// Helper function to create an expirable widget
pub fn expirable_widget(parameter: ExpirableParameter) -> ExpirableWidget<'static> {
    ExpirableWidget::new(parameter)
}
