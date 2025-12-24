//! Notice display widget for NoticeParameter.

use crate::{ParameterTheme, ParameterWidget, WidgetResponse};
use egui::{Frame, CornerRadius, Stroke, Ui};
use nebula_parameter::core::Parameter;
use nebula_parameter::types::{NoticeParameter, NoticeType};

/// Widget for displaying notices/alerts.
pub struct NoticeWidget {
    parameter: NoticeParameter,
    /// Whether the notice has been dismissed
    dismissed: bool,
}

impl ParameterWidget for NoticeWidget {
    type Parameter = NoticeParameter;

    fn new(parameter: Self::Parameter) -> Self {
        Self {
            parameter,
            dismissed: false,
        }
    }

    fn parameter(&self) -> &Self::Parameter {
        &self.parameter
    }

    fn parameter_mut(&mut self) -> &mut Self::Parameter {
        &mut self.parameter
    }

    fn show(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> WidgetResponse {
        let response = WidgetResponse::default();

        // Don't show if dismissed
        if self.dismissed {
            return response;
        }

        let notice_type = self
            .parameter
            .options
            .as_ref()
            .and_then(|o| o.notice_type.as_ref())
            .unwrap_or(&NoticeType::Info);

        let is_dismissible = self
            .parameter
            .options
            .as_ref()
            .is_some_and(|o| o.dismissible);

        let accent_color = match notice_type {
            NoticeType::Info => theme.info,
            NoticeType::Warning => theme.warning,
            NoticeType::Error => theme.error,
            NoticeType::Success => theme.success,
        };

        // Notice - flat with left accent border only
        Frame::none()
            .fill(theme.surface)
            .stroke(Stroke::new(1.0, theme.input_border))
            .rounding(CornerRadius::same(theme.border_radius as u8))
            .inner_margin(egui::Margin::same(0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Left accent bar
                    let available_height = ui.available_height().max(32.0);
                    let (accent_rect, _) = ui.allocate_exact_size(
                        egui::vec2(3.0, available_height),
                        egui::Sense::hover(),
                    );
                    ui.painter().rect_filled(
                        accent_rect,
                        CornerRadius {
                            nw: theme.border_radius as u8,
                            sw: theme.border_radius as u8,
                            ne: 0,
                            se: 0,
                        },
                        accent_color,
                    );

                    // Content area
                    ui.add_space(10.0);

                    ui.vertical(|ui| {
                        ui.add_space(6.0);

                        // Title and dismiss button row
                        ui.horizontal(|ui| {
                            // Title (from metadata name)
                            let metadata = self.parameter.metadata();
                            if !metadata.name.is_empty() {
                                ui.label(
                                    egui::RichText::new(&metadata.name)
                                        .strong()
                                        .color(accent_color),
                                );
                            }

                            // Dismiss button (right-aligned)
                            if is_dismissible {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.add_space(6.0);
                                        if ui.small_button("x").clicked() {
                                            self.dismissed = true;
                                        }
                                    },
                                );
                            }
                        });

                        // Content text
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(&self.parameter.content).color(theme.label_color),
                        );

                        // Hint (help text)
                        let metadata = self.parameter.metadata();
                        if let Some(ref hint) = metadata.hint {
                            if !hint.is_empty() {
                                ui.add_space(2.0);
                                ui.label(egui::RichText::new(hint).small().color(theme.hint_color));
                            }
                        }

                        ui.add_space(6.0);
                    });

                    ui.add_space(10.0);
                });
            });

        response
    }
}

impl NoticeWidget {
    /// Check if the notice has been dismissed.
    #[must_use]
    pub fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    /// Reset the dismissed state to show the notice again.
    pub fn reset(&mut self) {
        self.dismissed = false;
    }

    /// Manually dismiss the notice.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// Get the notice type.
    #[must_use]
    pub fn notice_type(&self) -> &NoticeType {
        self.parameter
            .options
            .as_ref()
            .and_then(|o| o.notice_type.as_ref())
            .unwrap_or(&NoticeType::Info)
    }

    /// Get the notice content.
    #[must_use]
    pub fn content(&self) -> &str {
        &self.parameter.content
    }
}
