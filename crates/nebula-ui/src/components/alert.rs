//! Alert component for displaying important messages.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Vec2, Widget};

/// Alert variant determines the visual style and icon
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlertVariant {
    /// Informational message (blue)
    #[default]
    Info,
    /// Success message (green)
    Success,
    /// Warning message (yellow/orange)
    Warning,
    /// Error/destructive message (red)
    Error,
}

/// An alert component for displaying important messages
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::{Alert, AlertVariant};
///
/// ui.add(Alert::new("Operation completed successfully!")
///     .variant(AlertVariant::Success)
///     .title("Success"));
/// ```
pub struct Alert<'a> {
    message: &'a str,
    variant: AlertVariant,
    title: Option<&'a str>,
    dismissible: bool,
    dismissed: Option<&'a mut bool>,
}

impl<'a> Alert<'a> {
    /// Create a new alert with a message
    pub fn new(message: &'a str) -> Self {
        Self {
            message,
            variant: AlertVariant::Info,
            title: None,
            dismissible: false,
            dismissed: None,
        }
    }

    /// Set the alert variant
    pub fn variant(mut self, variant: AlertVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Set as info variant
    pub fn info(mut self) -> Self {
        self.variant = AlertVariant::Info;
        self
    }

    /// Set as success variant
    pub fn success(mut self) -> Self {
        self.variant = AlertVariant::Success;
        self
    }

    /// Set as warning variant
    pub fn warning(mut self) -> Self {
        self.variant = AlertVariant::Warning;
        self
    }

    /// Set as error variant
    pub fn error(mut self) -> Self {
        self.variant = AlertVariant::Error;
        self
    }

    /// Set a title for the alert
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// Make the alert dismissible
    pub fn dismissible(mut self, dismissed: &'a mut bool) -> Self {
        self.dismissible = true;
        self.dismissed = Some(dismissed);
        self
    }

    /// Show the alert
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }

    fn icon(&self) -> &'static str {
        match self.variant {
            AlertVariant::Info => "ℹ",
            AlertVariant::Success => "✓",
            AlertVariant::Warning => "⚠",
            AlertVariant::Error => "✕",
        }
    }
}

impl<'a> Widget for Alert<'a> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        // Check if dismissed
        if let Some(dismissed) = &self.dismissed {
            if **dismissed {
                return ui.allocate_response(Vec2::ZERO, egui::Sense::hover());
            }
        }

        let theme = current_theme();
        let tokens = &theme.tokens;

        let (bg_color, border_color, icon_color) = match self.variant {
            AlertVariant::Info => (tokens.info.linear_multiply(0.15), tokens.info, tokens.info),
            AlertVariant::Success => (
                tokens.success.linear_multiply(0.15),
                tokens.success,
                tokens.success,
            ),
            AlertVariant::Warning => (
                tokens.warning.linear_multiply(0.15),
                tokens.warning,
                tokens.warning,
            ),
            AlertVariant::Error => (
                tokens.destructive.linear_multiply(0.15),
                tokens.destructive,
                tokens.destructive,
            ),
        };

        let frame = egui::Frame::new()
            .fill(bg_color)
            .stroke(egui::Stroke::new(1.0, border_color))
            .corner_radius(tokens.rounding_md())
            .inner_margin(tokens.spacing_md);

        frame
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Icon
                    ui.label(
                        RichText::new(self.icon())
                            .size(tokens.font_size_lg)
                            .color(icon_color),
                    );

                    ui.add_space(tokens.spacing_sm);

                    // Content
                    ui.vertical(|ui| {
                        if let Some(title) = self.title {
                            ui.label(
                                RichText::new(title)
                                    .size(tokens.font_size_md)
                                    .strong()
                                    .color(tokens.foreground),
                            );
                            ui.add_space(2.0);
                        }

                        ui.label(
                            RichText::new(self.message)
                                .size(tokens.font_size_sm)
                                .color(tokens.muted_foreground),
                        );
                    });

                    // Dismiss button
                    if self.dismissible {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            let dismiss_btn = ui.add(
                                egui::Button::new(RichText::new("✕").size(tokens.font_size_sm))
                                    .fill(egui::Color32::TRANSPARENT)
                                    .stroke(egui::Stroke::NONE),
                            );

                            if dismiss_btn.clicked() {
                                if let Some(dismissed) = &mut self.dismissed {
                                    **dismissed = true;
                                }
                            }

                            if dismiss_btn.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                        });
                    }
                });
            })
            .response
    }
}
