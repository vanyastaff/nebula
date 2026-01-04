//! Card container component.

use crate::theme::current_theme;
use egui::{Color32, Margin, Response, Ui};

/// A card container with optional header and footer
///
/// # Example
///
/// ```rust,ignore
/// Card::new()
///     .hoverable()
///     .show(ui, |ui| {
///         ui.label("Card content");
///     });
/// ```
pub struct Card {
    padding: Option<f32>,
    margin: Option<f32>,
    hoverable: bool,
    selected: bool,
    clickable: bool,
    border_color: Option<Color32>,
    background: Option<Color32>,
    rounding: Option<f32>,
    shadow: bool,
    min_width: Option<f32>,
    min_height: Option<f32>,
    full_width: bool,
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

impl Card {
    /// Create a new card
    pub fn new() -> Self {
        Self {
            padding: None,
            margin: None,
            hoverable: false,
            selected: false,
            clickable: false,
            border_color: None,
            background: None,
            rounding: None,
            shadow: false,
            min_width: None,
            min_height: None,
            full_width: false,
        }
    }

    /// Set padding inside the card
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = Some(padding);
        self
    }

    /// Set margin around the card
    pub fn margin(mut self, margin: f32) -> Self {
        self.margin = Some(margin);
        self
    }

    /// Enable hover effect
    pub fn hoverable(mut self) -> Self {
        self.hoverable = true;
        self
    }

    /// Set selected state (highlighted border)
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Make the card clickable
    pub fn clickable(mut self) -> Self {
        self.clickable = true;
        self.hoverable = true;
        self
    }

    /// Set custom border color
    pub fn border_color(mut self, color: Color32) -> Self {
        self.border_color = Some(color);
        self
    }

    /// Set custom background color
    pub fn background(mut self, color: Color32) -> Self {
        self.background = Some(color);
        self
    }

    /// Set custom rounding
    pub fn rounding(mut self, rounding: f32) -> Self {
        self.rounding = Some(rounding);
        self
    }

    /// Enable shadow
    pub fn shadow(mut self) -> Self {
        self.shadow = true;
        self
    }

    /// Set minimum width
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = Some(width);
        self
    }

    /// Set minimum height
    pub fn min_height(mut self, height: f32) -> Self {
        self.min_height = Some(height);
        self
    }

    /// Make full width
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Show the card with content
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> CardResponse<R> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let padding = self.padding.unwrap_or(tokens.spacing_lg);
        let rounding = self.rounding.unwrap_or(tokens.radius_lg);

        let base_bg = self.background.unwrap_or(tokens.card);

        // Allocate space for the card
        let available_width = if self.full_width {
            ui.available_width()
        } else {
            self.min_width.unwrap_or(0.0)
        };

        // Create a sense for hover/click detection
        let sense = if self.clickable {
            egui::Sense::click()
        } else if self.hoverable {
            egui::Sense::hover()
        } else {
            egui::Sense::hover()
        };

        // Determine colors based on state
        let (bg_color, border_color, border_width) = {
            let is_hovered = false; // Will be updated after response

            let bg = if is_hovered && self.hoverable {
                crate::theme::color_mix(base_bg, tokens.accent, 0.1)
            } else {
                base_bg
            };

            let border = if self.selected {
                tokens.primary
            } else {
                self.border_color.unwrap_or(tokens.border)
            };

            let width = if self.selected { 2.0 } else { 1.0 };

            (bg, border, width)
        };

        let mut frame = egui::Frame::NONE
            .fill(bg_color)
            .stroke(egui::Stroke::new(border_width, border_color))
            .corner_radius(rounding)
            .inner_margin(Margin::same(padding as i8));

        if self.shadow {
            frame = frame.shadow(egui::Shadow {
                offset: [0, 2],
                blur: tokens.shadow_md as u8,
                spread: 0,
                color: tokens.shadow_color,
            });
        }

        if let Some(margin) = self.margin {
            frame = frame.outer_margin(Margin::same(margin as i8));
        }

        let frame_response = frame.show(ui, |ui| {
            if self.full_width {
                ui.set_min_width(available_width - padding * 2.0);
            }
            if let Some(w) = self.min_width {
                ui.set_min_width(w);
            }
            if let Some(h) = self.min_height {
                ui.set_min_height(h);
            }
            add_contents(ui)
        });

        let response = ui.interact(
            frame_response.response.rect,
            ui.id().with("card_interact"),
            sense,
        );

        // Update cursor for clickable cards
        if self.clickable && response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        CardResponse {
            inner: frame_response.inner,
            response,
        }
    }
}

/// Response from showing a card
pub struct CardResponse<R> {
    /// The inner content's return value
    pub inner: R,
    /// The card's interaction response
    pub response: Response,
}

impl<R> CardResponse<R> {
    /// Check if the card was clicked
    pub fn clicked(&self) -> bool {
        self.response.clicked()
    }

    /// Check if the card is hovered
    pub fn hovered(&self) -> bool {
        self.response.hovered()
    }

    /// Check if the card has focus
    pub fn has_focus(&self) -> bool {
        self.response.has_focus()
    }
}

/// Card with a header section
pub struct HeaderCard<'a> {
    title: &'a str,
    subtitle: Option<&'a str>,
    card: Card,
    collapsible: bool,
    collapsed: Option<&'a mut bool>,
    action: Option<Box<dyn FnOnce(&mut Ui) + 'a>>,
}

impl<'a> HeaderCard<'a> {
    /// Create a new header card
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            subtitle: None,
            card: Card::new(),
            collapsible: false,
            collapsed: None,
            action: None,
        }
    }

    /// Set subtitle
    pub fn subtitle(mut self, subtitle: &'a str) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    /// Make collapsible
    pub fn collapsible(mut self, collapsed: &'a mut bool) -> Self {
        self.collapsible = true;
        self.collapsed = Some(collapsed);
        self
    }

    /// Add action widget to header
    pub fn action(mut self, action: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.action = Some(Box::new(action));
        self
    }

    /// Apply card settings
    pub fn card(mut self, f: impl FnOnce(Card) -> Card) -> Self {
        self.card = f(self.card);
        self
    }

    /// Show the card
    pub fn show<R>(
        mut self,
        ui: &mut Ui,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> CardResponse<Option<R>> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        // Extract values before closure to avoid borrow issues
        let collapsed_state = self.collapsed.as_ref().map_or(false, |c| **c);
        let title = self.title;
        let subtitle = self.subtitle;
        let collapsible = self.collapsible;
        let action = self.action.take();
        let collapsed_ref = self.collapsed.take();

        self.card.show(ui, |ui| {
            let mut collapsed_ref = collapsed_ref;

            // Header
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new(title)
                            .size(tokens.font_size_lg)
                            .color(tokens.foreground)
                            .strong(),
                    );

                    if let Some(subtitle) = subtitle {
                        ui.label(
                            egui::RichText::new(subtitle)
                                .size(tokens.font_size_sm)
                                .color(tokens.muted_foreground),
                        );
                    }
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(action) = action {
                        action(ui);
                    }

                    if collapsible {
                        if let Some(ref mut collapsed) = collapsed_ref {
                            let icon = if **collapsed { "▶" } else { "▼" };
                            if ui.small_button(icon).clicked() {
                                **collapsed = !**collapsed;
                            }
                        }
                    }
                });
            });

            // Content
            let show_content = if collapsible { !collapsed_state } else { true };

            if show_content {
                ui.add_space(tokens.spacing_md);
                Some(add_contents(ui))
            } else {
                None
            }
        })
    }
}
