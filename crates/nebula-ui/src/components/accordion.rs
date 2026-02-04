//! Accordion component for collapsible content sections.

use crate::icons::Icon;
use crate::theme::current_theme;
use egui::{Response, Ui, Vec2};

/// A single accordion item
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::AccordionItem;
///
/// AccordionItem::new("Section 1", &mut open)
///     .show(ui, |ui| {
///         ui.label("Content here");
///     });
/// ```
pub struct AccordionItem<'a> {
    title: &'a str,
    open: &'a mut bool,
    icon: Option<Icon>,
    disabled: bool,
}

impl<'a> AccordionItem<'a> {
    /// Create a new accordion item
    pub fn new(title: &'a str, open: &'a mut bool) -> Self {
        Self {
            title,
            open,
            icon: None,
            disabled: false,
        }
    }

    /// Set an icon for the header
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Show the accordion item with content
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let header_height = 40.0;

        // Header
        let (header_rect, header_response) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), header_height),
            if self.disabled {
                egui::Sense::hover()
            } else {
                egui::Sense::click()
            },
        );

        if header_response.clicked() && !self.disabled {
            *self.open = !*self.open;
        }

        // Draw header background
        if ui.is_rect_visible(header_rect) {
            let painter = ui.painter();

            let bg_color = if header_response.hovered() && !self.disabled {
                tokens.accent
            } else {
                tokens.muted
            };

            painter.rect_filled(header_rect, tokens.rounding_sm(), bg_color);

            // Chevron icon
            let chevron = if *self.open { "▼" } else { "▶" };
            let chevron_pos = egui::Pos2::new(
                header_rect.left() + tokens.spacing_md,
                header_rect.center().y,
            );
            painter.text(
                chevron_pos,
                egui::Align2::LEFT_CENTER,
                chevron,
                egui::FontId::proportional(tokens.font_size_sm),
                if self.disabled {
                    tokens.muted_foreground
                } else {
                    tokens.foreground
                },
            );

            // Optional icon
            let text_start = if self.icon.is_some() {
                let icon_pos = egui::Pos2::new(
                    header_rect.left() + tokens.spacing_md + 20.0,
                    header_rect.center().y,
                );
                painter.text(
                    icon_pos,
                    egui::Align2::LEFT_CENTER,
                    self.icon.map(|i| i.as_str()).unwrap_or(""),
                    egui::FontId::proportional(tokens.font_size_md),
                    tokens.foreground,
                );
                header_rect.left() + tokens.spacing_md + 44.0
            } else {
                header_rect.left() + tokens.spacing_md + 20.0
            };

            // Title
            painter.text(
                egui::Pos2::new(text_start, header_rect.center().y),
                egui::Align2::LEFT_CENTER,
                self.title,
                egui::FontId::proportional(tokens.font_size_md),
                if self.disabled {
                    tokens.muted_foreground
                } else {
                    tokens.foreground
                },
            );
        }

        // Cursor
        if header_response.hovered() && !self.disabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        // Content (when open)
        if *self.open {
            ui.add_space(tokens.spacing_xs);
            ui.indent("accordion_content", |ui| {
                add_contents(ui);
            });
            ui.add_space(tokens.spacing_sm);
        }

        header_response
    }
}

/// A group of accordion items (only one open at a time)
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::Accordion;
///
/// Accordion::new(&mut selected_index)
///     .item("Section 1", |ui| { ui.label("Content 1"); })
///     .item("Section 2", |ui| { ui.label("Content 2"); })
///     .show(ui);
/// ```
pub struct Accordion<'a> {
    selected: &'a mut Option<usize>,
    items: Vec<(&'a str, Box<dyn FnOnce(&mut Ui) + 'a>)>,
    allow_multiple: bool,
}

impl<'a> Accordion<'a> {
    /// Create a new accordion group
    pub fn new(selected: &'a mut Option<usize>) -> Self {
        Self {
            selected,
            items: Vec::new(),
            allow_multiple: false,
        }
    }

    /// Add an item to the accordion
    pub fn item(mut self, title: &'a str, content: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.items.push((title, Box::new(content)));
        self
    }

    /// Show the accordion
    pub fn show(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let response = ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = tokens.spacing_xs;

            for (index, (title, content)) in self.items.into_iter().enumerate() {
                let is_open = *self.selected == Some(index);
                let mut open = is_open;

                let item_response = AccordionItem::new(title, &mut open).show(ui, content);

                if item_response.clicked() {
                    if is_open {
                        *self.selected = None;
                    } else {
                        *self.selected = Some(index);
                    }
                }
            }
        });

        response.response
    }
}
