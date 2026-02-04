//! Hover card component for rich tooltips.

use crate::theme::current_theme;
use egui::{Response, Ui};

/// A hover card that shows rich content on hover
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::HoverCard;
///
/// let response = ui.label("Hover over me");
/// HoverCard::new(&response)
///     .width(200.0)
///     .show(ui, |ui| {
///         ui.label("Rich content here");
///         ui.label("With multiple elements");
///     });
/// ```
pub struct HoverCard<'a> {
    response: &'a Response,
    width: Option<f32>,
}

impl<'a> HoverCard<'a> {
    /// Create a new hover card
    pub fn new(response: &'a Response) -> Self {
        Self {
            response,
            width: None,
        }
    }

    /// Set the card width
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Show the hover card content
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) {
        if self.response.hovered() {
            let theme = current_theme();
            let tokens = &theme.tokens;

            self.response.clone().on_hover_ui(|ui| {
                if let Some(width) = self.width {
                    ui.set_min_width(width);
                }

                let frame = egui::Frame::new()
                    .fill(tokens.card)
                    .stroke(egui::Stroke::new(1.0, tokens.border))
                    .corner_radius(tokens.rounding_md())
                    .inner_margin(tokens.spacing_md)
                    .shadow(egui::Shadow {
                        offset: [0, 4],
                        blur: 8,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(40),
                    });

                frame.show(ui, |ui| {
                    add_contents(ui);
                });
            });
        }
    }
}
