//! Context menu component for right-click menus.

use crate::theme::current_theme;
use egui::{Response, RichText, Ui};

/// A context menu item
pub struct ContextMenuItem<'a> {
    label: &'a str,
    icon: Option<&'a str>,
    shortcut: Option<&'a str>,
    disabled: bool,
    destructive: bool,
}

impl<'a> ContextMenuItem<'a> {
    /// Create a new menu item
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            shortcut: None,
            disabled: false,
            destructive: false,
        }
    }

    /// Add an icon
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Add a keyboard shortcut hint
    pub fn shortcut(mut self, shortcut: &'a str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Mark as destructive action
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// Show the menu item, returns true if clicked
    pub fn show(self, ui: &mut Ui) -> bool {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let text_color = if self.disabled {
            tokens.muted_foreground
        } else if self.destructive {
            tokens.destructive
        } else {
            tokens.foreground
        };

        let response = ui.horizontal(|ui| {
            ui.set_min_width(150.0);

            // Icon
            if let Some(icon) = self.icon {
                ui.label(
                    RichText::new(icon)
                        .size(tokens.font_size_sm)
                        .color(text_color),
                );
            } else {
                ui.add_space(tokens.font_size_sm + 4.0);
            }

            // Label
            ui.label(
                RichText::new(self.label)
                    .size(tokens.font_size_sm)
                    .color(text_color),
            );

            // Shortcut (right-aligned)
            if let Some(shortcut) = self.shortcut {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(shortcut)
                            .size(tokens.font_size_xs)
                            .color(tokens.muted_foreground),
                    );
                });
            }
        });

        let clicked = response.response.interact(egui::Sense::click()).clicked() && !self.disabled;

        if response.response.hovered() && !self.disabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        clicked
    }
}

/// A separator in a context menu
pub fn context_menu_separator(ui: &mut Ui) {
    let theme = current_theme();
    let tokens = &theme.tokens;

    ui.add_space(tokens.spacing_xs);
    ui.separator();
    ui.add_space(tokens.spacing_xs);
}

/// Show a context menu on right-click
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::{context_menu, ContextMenuItem};
///
/// let response = ui.label("Right-click me");
/// context_menu(&response, |ui| {
///     if ContextMenuItem::new("Copy").icon("📋").shortcut("Ctrl+C").show(ui) {
///         // Handle copy
///     }
///     context_menu_separator(ui);
///     if ContextMenuItem::new("Delete").icon("🗑").destructive().show(ui) {
///         // Handle delete
///     }
/// });
/// ```
pub fn context_menu<R>(response: &Response, add_contents: impl FnOnce(&mut Ui) -> R) {
    response.context_menu(|ui| {
        let theme = current_theme();
        let tokens = &theme.tokens;

        ui.spacing_mut().item_spacing.y = tokens.spacing_xs;
        add_contents(ui);
    });
}

/// A submenu in a context menu
pub struct ContextSubMenu<'a> {
    label: &'a str,
    icon: Option<&'a str>,
}

impl<'a> ContextSubMenu<'a> {
    /// Create a new submenu
    pub fn new(label: &'a str) -> Self {
        Self { label, icon: None }
    }

    /// Add an icon
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Show the submenu
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) {
        let theme = current_theme();
        let tokens = &theme.tokens;

        ui.menu_button(
            RichText::new(format!("{} {}", self.icon.unwrap_or(""), self.label))
                .size(tokens.font_size_sm),
            |ui| {
                ui.spacing_mut().item_spacing.y = tokens.spacing_xs;
                add_contents(ui);
            },
        );
    }
}
