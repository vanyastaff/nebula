//! Tooltip component.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Widget};

/// A styled tooltip
pub struct Tooltip<'a> {
    text: &'a str,
    shortcut: Option<&'a str>,
    delay: f32,
}

impl<'a> Tooltip<'a> {
    /// Create a new tooltip
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            shortcut: None,
            delay: 0.5,
        }
    }

    /// Add keyboard shortcut hint
    pub fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Set delay before showing (seconds)
    pub fn delay(mut self, delay: f32) -> Self {
        self.delay = delay;
        self
    }

    /// Show tooltip on hover of the given response
    pub fn show(self, response: &Response, ui: &Ui) {
        let theme = current_theme();
        let tokens = &theme.tokens;

        response.clone().on_hover_ui_at_pointer(|ui| {
            ui.set_max_width(300.0);

            let frame = egui::Frame::NONE
                .fill(tokens.card)
                .stroke(egui::Stroke::new(1.0, tokens.border))
                .corner_radius(tokens.rounding_md())
                .inner_margin(tokens.spacing_sm as i8)
                .shadow(egui::Shadow {
                    offset: [0, 2],
                    blur: tokens.shadow_sm as u8,
                    spread: 0,
                    color: tokens.shadow_color,
                });

            frame.show(ui, |ui| {
                if let Some(shortcut) = self.shortcut {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(self.text)
                                .size(tokens.font_size_sm)
                                .color(tokens.foreground),
                        );
                        ui.add_space(tokens.spacing_md);
                        ui.label(
                            RichText::new(shortcut)
                                .size(tokens.font_size_xs)
                                .color(tokens.muted_foreground)
                                .background_color(tokens.muted),
                        );
                    });
                } else {
                    ui.label(
                        RichText::new(self.text)
                            .size(tokens.font_size_sm)
                            .color(tokens.foreground),
                    );
                }
            });
        });
    }
}

/// Extension trait for adding tooltips to responses
pub trait TooltipExt {
    /// Add a simple tooltip
    fn tooltip(self, text: &str) -> Self;

    /// Add a tooltip with keyboard shortcut
    fn tooltip_with_shortcut(self, text: &str, shortcut: &str) -> Self;
}

impl TooltipExt for Response {
    fn tooltip(self, text: &str) -> Self {
        self.on_hover_text(text)
    }

    fn tooltip_with_shortcut(self, text: &str, shortcut: &str) -> Self {
        self.on_hover_ui(|ui| {
            let theme = current_theme();
            let tokens = &theme.tokens;

            ui.horizontal(|ui| {
                ui.label(text);
                ui.add_space(tokens.spacing_md);

                // Keyboard shortcut badge
                egui::Frame::NONE
                    .fill(tokens.muted)
                    .corner_radius(tokens.rounding_sm())
                    .inner_margin(egui::Margin::symmetric(
                        tokens.spacing_xs as i8,
                        (tokens.spacing_xs / 2.0) as i8,
                    ))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(shortcut)
                                .size(tokens.font_size_xs)
                                .color(tokens.muted_foreground)
                                .monospace(),
                        );
                    });
            });
        })
    }
}

/// Info tooltip (question mark icon that shows text on hover)
pub struct InfoTooltip<'a> {
    text: &'a str,
}

impl<'a> InfoTooltip<'a> {
    /// Create a new info tooltip
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }

    /// Show the info icon with tooltip
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for InfoTooltip<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let response = ui.add(
            egui::Label::new(
                RichText::new("ⓘ")
                    .size(tokens.font_size_sm)
                    .color(tokens.muted_foreground),
            )
            .sense(egui::Sense::hover()),
        );

        response.clone().on_hover_ui(|ui| {
            ui.set_max_width(250.0);
            ui.label(
                RichText::new(self.text)
                    .size(tokens.font_size_sm)
                    .color(tokens.foreground),
            );
        });

        response
    }
}
