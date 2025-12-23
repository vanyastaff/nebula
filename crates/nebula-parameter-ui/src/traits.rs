//! Common traits for parameter widgets.

use crate::ParameterTheme;
use egui::{Frame, Rounding, Stroke, Ui};

/// Response from a parameter widget.
#[derive(Debug, Clone, Default)]
pub struct WidgetResponse {
    /// Whether the value changed.
    pub changed: bool,
    /// Whether the widget lost focus (finished editing).
    pub lost_focus: bool,
    /// Validation error message, if any.
    pub error: Option<String>,
}

impl WidgetResponse {
    /// Create a response indicating the value changed.
    #[must_use]
    pub fn changed() -> Self {
        Self {
            changed: true,
            ..Default::default()
        }
    }

    /// Create a response indicating focus was lost.
    #[must_use]
    pub fn lost_focus() -> Self {
        Self {
            lost_focus: true,
            ..Default::default()
        }
    }

    /// Create a response with an error.
    #[must_use]
    pub fn with_error(error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Default::default()
        }
    }
}

/// Trait for parameter widgets.
///
/// All parameter widgets implement this trait to provide a consistent interface.
pub trait ParameterWidget {
    /// The parameter type this widget handles.
    type Parameter;

    /// Create a new widget for the given parameter.
    fn new(parameter: Self::Parameter) -> Self;

    /// Get a reference to the underlying parameter.
    fn parameter(&self) -> &Self::Parameter;

    /// Get a mutable reference to the underlying parameter.
    fn parameter_mut(&mut self) -> &mut Self::Parameter;

    /// Render the widget and return a response.
    fn show(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> WidgetResponse;

    /// Render the widget with a label.
    fn show_with_label(&mut self, ui: &mut Ui, theme: &ParameterTheme) -> WidgetResponse
    where
        Self: Sized,
    {
        self.show(ui, theme)
    }
}

/// Extension methods for egui Ui with themed styling.
pub trait UiExt {
    /// Add vertical spacing based on theme.
    fn theme_spacing(&mut self, theme: &ParameterTheme);

    /// Show a label with theme styling.
    fn themed_label(&mut self, theme: &ParameterTheme, text: &str);

    /// Show a hint/description with theme styling.
    fn themed_hint(&mut self, theme: &ParameterTheme, text: &str);

    /// Show an error message with theme styling.
    fn themed_error(&mut self, theme: &ParameterTheme, text: &str);

    /// Show a styled input container with optional icon.
    fn styled_input_frame<R>(
        &mut self,
        theme: &ParameterTheme,
        icon: Option<&str>,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> (egui::InnerResponse<R>, bool);

    /// Show a parameter header with label and optional badge.
    fn param_header(&mut self, theme: &ParameterTheme, label: &str, badge: Option<&str>);

    /// Show description text below input.
    fn param_description(&mut self, theme: &ParameterTheme, text: &str);

    /// Show a styled error box.
    fn error_box(&mut self, theme: &ParameterTheme, error: &str);

    /// Show a success indicator.
    fn success_indicator(&mut self, theme: &ParameterTheme, text: &str);
}

impl UiExt for Ui {
    fn theme_spacing(&mut self, theme: &ParameterTheme) {
        self.add_space(theme.spacing);
    }

    fn themed_label(&mut self, theme: &ParameterTheme, text: &str) {
        self.label(egui::RichText::new(text).color(theme.label_color).strong());
    }

    fn themed_hint(&mut self, theme: &ParameterTheme, text: &str) {
        self.label(egui::RichText::new(text).small().color(theme.hint_color));
    }

    fn themed_error(&mut self, theme: &ParameterTheme, text: &str) {
        self.horizontal(|ui| {
            ui.label(egui::RichText::new("⚠").color(theme.error));
            ui.label(egui::RichText::new(text).small().color(theme.error));
        });
    }

    fn styled_input_frame<R>(
        &mut self,
        theme: &ParameterTheme,
        icon: Option<&str>,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> (egui::InnerResponse<R>, bool) {
        let frame_response = Frame::none()
            .fill(theme.input_bg)
            .stroke(Stroke::new(1.0, theme.input_border))
            .rounding(Rounding::same(theme.border_radius as u8))
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(self, |ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(24.0);

                    // Optional icon
                    if let Some(icon_str) = icon {
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(icon_str)
                                .size(14.0)
                                .color(theme.primary),
                        );
                        ui.add_space(4.0);
                    }

                    add_contents(ui)
                })
                .inner
            });

        let is_hovered = frame_response.response.hovered();

        // Draw hover effect
        if is_hovered {
            self.painter().rect_stroke(
                frame_response.response.rect,
                Rounding::same(theme.border_radius as u8),
                Stroke::new(1.5, theme.input_border_focused),
                egui::StrokeKind::Outside,
            );
        }

        (frame_response, is_hovered)
    }

    fn param_header(&mut self, theme: &ParameterTheme, label: &str, badge: Option<&str>) {
        self.horizontal(|ui| {
            ui.label(egui::RichText::new(label).color(theme.label_color).strong());

            if let Some(badge_text) = badge {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    Frame::none()
                        .fill(theme.primary.gamma_multiply(0.15))
                        .rounding(Rounding::same(8))
                        .inner_margin(egui::Margin::symmetric(6, 1))
                        .show(ui, |ui| {
                            ui.label(egui::RichText::new(badge_text).small().color(theme.primary));
                        });
                });
            }
        });
    }

    fn param_description(&mut self, theme: &ParameterTheme, text: &str) {
        if !text.is_empty() {
            self.add_space(2.0);
            self.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(text).small().color(theme.hint_color));
            });
        }
    }

    fn error_box(&mut self, theme: &ParameterTheme, error: &str) {
        self.add_space(4.0);
        Frame::none()
            .fill(theme.error.gamma_multiply(0.1))
            .rounding(Rounding::same(4))
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(self, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("⚠").color(theme.error));
                    ui.label(egui::RichText::new(error).small().color(theme.error));
                });
            });
    }

    fn success_indicator(&mut self, theme: &ParameterTheme, text: &str) {
        Frame::none()
            .fill(theme.success.gamma_multiply(0.1))
            .rounding(Rounding::same(4))
            .inner_margin(egui::Margin::symmetric(6, 2))
            .show(self, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("✓").color(theme.success));
                    ui.label(egui::RichText::new(text).small().color(theme.success));
                });
            });
    }
}
