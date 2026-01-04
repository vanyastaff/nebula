//! Empty state component for when no content is available.

use crate::components::Button;
use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Widget};

/// Empty state component for displaying when no content exists
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::EmptyState;
///
/// ui.add(EmptyState::new("No items found")
///     .icon("📭")
///     .description("Try adjusting your search or filters")
///     .action("Add Item", || { /* handle */ }));
/// ```
pub struct EmptyState<'a> {
    title: &'a str,
    description: Option<&'a str>,
    icon: Option<&'a str>,
    action_label: Option<&'a str>,
    compact: bool,
}

impl<'a> EmptyState<'a> {
    /// Create a new empty state
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            description: None,
            icon: None,
            action_label: None,
            compact: false,
        }
    }

    /// Set a description
    pub fn description(mut self, description: &'a str) -> Self {
        self.description = Some(description);
        self
    }

    /// Set an icon
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set an action button label
    pub fn action(mut self, label: &'a str) -> Self {
        self.action_label = Some(label);
        self
    }

    /// Use compact layout
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// Show the empty state, returns true if action button clicked
    pub fn show(self, ui: &mut Ui) -> bool {
        let mut clicked = false;

        ui.vertical_centered(|ui| {
            let theme = current_theme();
            let tokens = &theme.tokens;

            let spacing = if self.compact {
                tokens.spacing_sm
            } else {
                tokens.spacing_lg
            };

            ui.add_space(spacing);

            // Icon
            if let Some(icon) = self.icon {
                ui.label(
                    RichText::new(icon)
                        .size(if self.compact { 32.0 } else { 48.0 })
                        .color(tokens.muted_foreground),
                );
                ui.add_space(tokens.spacing_md);
            }

            // Title
            ui.label(
                RichText::new(self.title)
                    .size(if self.compact {
                        tokens.font_size_md
                    } else {
                        tokens.font_size_lg
                    })
                    .strong()
                    .color(tokens.foreground),
            );

            // Description
            if let Some(desc) = self.description {
                ui.add_space(tokens.spacing_xs);
                ui.label(
                    RichText::new(desc)
                        .size(tokens.font_size_sm)
                        .color(tokens.muted_foreground),
                );
            }

            // Action button
            if let Some(label) = self.action_label {
                ui.add_space(tokens.spacing_md);
                if Button::new(label).primary().show(ui).clicked() {
                    clicked = true;
                }
            }

            ui.add_space(spacing);
        });

        clicked
    }
}

impl<'a> Widget for EmptyState<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let response = ui.vertical_centered(|ui| {
            self.show(ui);
        });
        response.response
    }
}
