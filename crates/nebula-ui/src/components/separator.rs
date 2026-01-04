//! Separator component.

use crate::theme::current_theme;
use egui::{Response, Ui, Widget};

/// A visual separator
pub struct Separator {
    vertical: bool,
    spacing: Option<f32>,
    text: Option<String>,
}

impl Default for Separator {
    fn default() -> Self {
        Self::new()
    }
}

impl Separator {
    /// Create a new horizontal separator
    pub fn new() -> Self {
        Self {
            vertical: false,
            spacing: None,
            text: None,
        }
    }

    /// Make vertical
    pub fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }

    /// Set spacing around separator
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = Some(spacing);
        self
    }

    /// Add text label in the middle
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Show the separator
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl Widget for Separator {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let spacing = self.spacing.unwrap_or(tokens.spacing_md);

        if let Some(text) = self.text {
            // Separator with text
            ui.add_space(spacing);
            let response = ui.horizontal(|ui| {
                let available = ui.available_width();
                let font_id = egui::TextStyle::Body.resolve(ui.style());
                let text_width = ui.fonts_mut(|f| f.glyph_width(&font_id, ' ')) * text.len() as f32;

                let line_width = (available - text_width - tokens.spacing_lg * 2.0) / 2.0;

                // Left line
                ui.add_sized([line_width, 1.0], egui::Separator::default().horizontal());

                // Text
                ui.add_space(tokens.spacing_sm);
                ui.label(
                    egui::RichText::new(&text)
                        .size(tokens.font_size_sm)
                        .color(tokens.muted_foreground),
                );
                ui.add_space(tokens.spacing_sm);

                // Right line
                ui.add_sized([line_width, 1.0], egui::Separator::default().horizontal());
            });
            ui.add_space(spacing);
            response.response
        } else {
            // Simple separator
            ui.add_space(spacing);
            let response = if self.vertical {
                ui.add(egui::Separator::default().vertical())
            } else {
                ui.add(egui::Separator::default().horizontal())
            };
            ui.add_space(spacing);
            response
        }
    }
}

/// Convenience function for horizontal separator
pub fn separator(ui: &mut Ui) -> Response {
    Separator::new().show(ui)
}

/// Convenience function for vertical separator
pub fn separator_vertical(ui: &mut Ui) -> Response {
    Separator::new().vertical().show(ui)
}
