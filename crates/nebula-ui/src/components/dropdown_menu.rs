//! Dropdown menu component.

use crate::components::ButtonVariant;
use crate::theme::current_theme;
use egui::{Response, RichText, Ui};

/// A dropdown menu triggered by a button
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::DropdownMenu;
///
/// DropdownMenu::new("Options")
///     .icon("⚙")
///     .show(ui, |ui| {
///         if ui.button("Option 1").clicked() {
///             // Handle option 1
///         }
///         if ui.button("Option 2").clicked() {
///             // Handle option 2
///         }
///     });
/// ```
pub struct DropdownMenu<'a> {
    label: &'a str,
    icon: Option<&'a str>,
    variant: ButtonVariant,
    disabled: bool,
}

impl<'a> DropdownMenu<'a> {
    /// Create a new dropdown menu
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            variant: ButtonVariant::Secondary,
            disabled: false,
        }
    }

    /// Create an icon-only dropdown
    pub fn icon_only(icon: &'a str) -> Self {
        Self {
            label: "",
            icon: Some(icon),
            variant: ButtonVariant::Ghost,
            disabled: false,
        }
    }

    /// Set an icon
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set the button variant
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Show the dropdown menu
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let button_text = if let Some(icon) = self.icon {
            if self.label.is_empty() {
                format!("{} ▼", icon)
            } else {
                format!("{} {} ▼", icon, self.label)
            }
        } else {
            format!("{} ▼", self.label)
        };

        let response = ui.menu_button(RichText::new(button_text).size(tokens.font_size_sm), |ui| {
            ui.spacing_mut().item_spacing.y = tokens.spacing_xs;
            add_contents(ui);
        });

        response.response
    }
}

/// A dropdown menu item
pub struct DropdownMenuItem<'a> {
    label: &'a str,
    icon: Option<&'a str>,
    selected: bool,
    disabled: bool,
}

impl<'a> DropdownMenuItem<'a> {
    /// Create a new menu item
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            selected: false,
            disabled: false,
        }
    }

    /// Add an icon
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Mark as selected
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Show the menu item, returns true if clicked
    pub fn show(self, ui: &mut Ui) -> bool {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let text_color = if self.disabled {
            tokens.muted_foreground
        } else {
            tokens.foreground
        };

        let bg_color = if self.selected {
            tokens.accent
        } else {
            egui::Color32::TRANSPARENT
        };

        let response = ui
            .horizontal(|ui| {
                // Selection indicator
                if self.selected {
                    ui.label(
                        RichText::new("✓")
                            .size(tokens.font_size_sm)
                            .color(tokens.primary),
                    );
                } else {
                    ui.add_space(tokens.font_size_sm + 4.0);
                }

                // Icon
                if let Some(icon) = self.icon {
                    ui.label(
                        RichText::new(icon)
                            .size(tokens.font_size_sm)
                            .color(text_color),
                    );
                }

                // Label
                ui.label(
                    RichText::new(self.label)
                        .size(tokens.font_size_sm)
                        .color(text_color),
                );
            })
            .response;

        let clicked = response.interact(egui::Sense::click()).clicked() && !self.disabled;

        if response.hovered() && !self.disabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        clicked
    }
}
