//! Collapsible component for expandable content.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui};

/// A collapsible section with header and content
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::Collapsible;
///
/// Collapsible::new("Advanced Settings", &mut open)
///     .show(ui, |ui| {
///         ui.label("Hidden content here");
///     });
/// ```
pub struct Collapsible<'a> {
    header: &'a str,
    open: &'a mut bool,
    default_open: bool,
    icon: Option<&'a str>,
}

impl<'a> Collapsible<'a> {
    /// Create a new collapsible section
    pub fn new(header: &'a str, open: &'a mut bool) -> Self {
        Self {
            header,
            open,
            default_open: false,
            icon: None,
        }
    }

    /// Set default open state
    pub fn default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    /// Set an icon for the header
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Show the collapsible with content
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let mut response = ui.horizontal(|ui| {
            // Toggle chevron
            let chevron = if *self.open { "▼" } else { "▶" };
            let toggle = ui.add(
                egui::Label::new(
                    RichText::new(chevron)
                        .size(tokens.font_size_sm)
                        .color(tokens.muted_foreground),
                )
                .sense(egui::Sense::click()),
            );

            if toggle.clicked() {
                *self.open = !*self.open;
            }

            if toggle.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            // Icon
            if let Some(icon) = self.icon {
                ui.label(
                    RichText::new(icon)
                        .size(tokens.font_size_md)
                        .color(tokens.foreground),
                );
            }

            // Header text (also clickable)
            let header_response = ui.add(
                egui::Label::new(
                    RichText::new(self.header)
                        .size(tokens.font_size_md)
                        .color(tokens.foreground),
                )
                .sense(egui::Sense::click()),
            );

            if header_response.clicked() {
                *self.open = !*self.open;
            }

            if header_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            toggle
        });

        // Content
        if *self.open {
            ui.add_space(tokens.spacing_xs);
            ui.indent("collapsible_content", |ui| {
                add_contents(ui);
            });
        }

        response.inner
    }
}
