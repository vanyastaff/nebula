//! Breadcrumb navigation component.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui, Widget};

/// A breadcrumb item
#[derive(Clone)]
pub struct BreadcrumbItem<'a> {
    /// Label text
    pub label: &'a str,
    /// Optional icon
    pub icon: Option<&'a str>,
}

impl<'a> BreadcrumbItem<'a> {
    /// Create a new breadcrumb item
    pub fn new(label: &'a str) -> Self {
        Self { label, icon: None }
    }

    /// Add an icon
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// Breadcrumb navigation component
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::{Breadcrumb, BreadcrumbItem};
///
/// let clicked = ui.add(Breadcrumb::new(vec![
///     BreadcrumbItem::new("Home").icon("🏠"),
///     BreadcrumbItem::new("Products"),
///     BreadcrumbItem::new("Electronics"),
/// ]));
/// ```
pub struct Breadcrumb<'a> {
    items: Vec<BreadcrumbItem<'a>>,
    separator: &'a str,
}

impl<'a> Breadcrumb<'a> {
    /// Create a new breadcrumb
    pub fn new(items: Vec<BreadcrumbItem<'a>>) -> Self {
        Self {
            items,
            separator: "/",
        }
    }

    /// Create from simple string labels
    pub fn from_labels(labels: &[&'a str]) -> Self {
        Self {
            items: labels.iter().map(|l| BreadcrumbItem::new(l)).collect(),
            separator: "/",
        }
    }

    /// Set custom separator
    pub fn separator(mut self, sep: &'a str) -> Self {
        self.separator = sep;
        self
    }

    /// Show the breadcrumb and return clicked index (if any)
    pub fn show(self, ui: &mut Ui) -> Option<usize> {
        let mut clicked = None;

        ui.horizontal(|ui| {
            let theme = current_theme();
            let tokens = &theme.tokens;

            let len = self.items.len();

            for (index, item) in self.items.into_iter().enumerate() {
                let is_last = index == len - 1;

                // Icon
                if let Some(icon) = item.icon {
                    ui.label(
                        RichText::new(icon)
                            .size(tokens.font_size_sm)
                            .color(tokens.muted_foreground),
                    );
                }

                // Label (clickable if not last)
                if is_last {
                    ui.label(
                        RichText::new(item.label)
                            .size(tokens.font_size_sm)
                            .color(tokens.foreground),
                    );
                } else {
                    let response = ui.add(
                        egui::Label::new(
                            RichText::new(item.label)
                                .size(tokens.font_size_sm)
                                .color(tokens.muted_foreground),
                        )
                        .sense(egui::Sense::click()),
                    );

                    if response.clicked() {
                        clicked = Some(index);
                    }

                    if response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }

                    // Separator
                    ui.label(
                        RichText::new(self.separator)
                            .size(tokens.font_size_sm)
                            .color(tokens.muted_foreground),
                    );
                }
            }
        });

        clicked
    }
}

impl<'a> Widget for Breadcrumb<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let response = ui.horizontal(|ui| {
            self.show(ui);
        });
        response.response
    }
}
