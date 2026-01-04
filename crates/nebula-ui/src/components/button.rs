//! Button component with variants and sizes.

use crate::theme::current_theme;
use egui::{Color32, Response, RichText, Ui, Vec2, Widget};

/// Button variant determines the visual style
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    /// Primary action button (solid primary color)
    #[default]
    Primary,
    /// Secondary button (subtle background)
    Secondary,
    /// Outline button (border only, transparent background)
    Outline,
    /// Ghost button (no border, no background)
    Ghost,
    /// Destructive action (red)
    Destructive,
    /// Link-style button (looks like a link)
    Link,
    /// Success action (green)
    Success,
}

/// Button size
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    /// Small button (compact)
    Sm,
    /// Medium button (default)
    #[default]
    Md,
    /// Large button
    Lg,
    /// Icon-only button (square)
    Icon,
}

/// A themed button component
///
/// # Example
///
/// ```rust,ignore
/// use nebula_ui::components::Button;
///
/// if Button::new("Save")
///     .primary()
///     .show(ui)
///     .clicked()
/// {
///     save_data();
/// }
/// ```
pub struct Button<'a> {
    text: &'a str,
    variant: ButtonVariant,
    size: ButtonSize,
    icon_left: Option<&'a str>,
    icon_right: Option<&'a str>,
    disabled: bool,
    loading: bool,
    selected: bool,
    min_width: Option<f32>,
    full_width: bool,
}

impl<'a> Button<'a> {
    /// Create a new button with the given text
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            variant: ButtonVariant::Primary,
            size: ButtonSize::Md,
            icon_left: None,
            icon_right: None,
            disabled: false,
            loading: false,
            selected: false,
            min_width: None,
            full_width: false,
        }
    }

    /// Set the button variant
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Set as primary variant
    pub fn primary(mut self) -> Self {
        self.variant = ButtonVariant::Primary;
        self
    }

    /// Set as secondary variant
    pub fn secondary(mut self) -> Self {
        self.variant = ButtonVariant::Secondary;
        self
    }

    /// Set as outline variant
    pub fn outline(mut self) -> Self {
        self.variant = ButtonVariant::Outline;
        self
    }

    /// Set as ghost variant
    pub fn ghost(mut self) -> Self {
        self.variant = ButtonVariant::Ghost;
        self
    }

    /// Set as destructive variant
    pub fn destructive(mut self) -> Self {
        self.variant = ButtonVariant::Destructive;
        self
    }

    /// Set as link variant
    pub fn link(mut self) -> Self {
        self.variant = ButtonVariant::Link;
        self
    }

    /// Set as success variant
    pub fn success(mut self) -> Self {
        self.variant = ButtonVariant::Success;
        self
    }

    /// Set the button size
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// Set as small size
    pub fn small(mut self) -> Self {
        self.size = ButtonSize::Sm;
        self
    }

    /// Set as large size
    pub fn large(mut self) -> Self {
        self.size = ButtonSize::Lg;
        self
    }

    /// Add an icon to the left of the text
    pub fn icon(mut self, icon: &'a str) -> Self {
        self.icon_left = Some(icon);
        self
    }

    /// Add an icon to the left of the text
    pub fn icon_left(mut self, icon: &'a str) -> Self {
        self.icon_left = Some(icon);
        self
    }

    /// Add an icon to the right of the text
    pub fn icon_right(mut self, icon: &'a str) -> Self {
        self.icon_right = Some(icon);
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Set loading state
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// Set selected state (for toggle buttons)
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Set minimum width
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = Some(width);
        self
    }

    /// Make button full width
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    /// Show the button and return the response
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for Button<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let (bg, fg, border) = self.colors(&theme);
        let (padding, font_size, min_size) = self.sizing(&theme);

        let is_enabled = !self.disabled && !self.loading;

        // Build button text
        let text = if self.loading {
            RichText::new("⟳").size(font_size).color(fg)
        } else {
            RichText::new(self.text).size(font_size).color(fg)
        };

        // Calculate size
        let mut desired_size = min_size;
        if self.full_width {
            desired_size.x = ui.available_width();
        } else if let Some(min_w) = self.min_width {
            desired_size.x = desired_size.x.max(min_w);
        }

        // Create the button
        let button = egui::Button::new(text)
            .fill(if is_enabled || self.selected {
                bg
            } else {
                tokens.muted
            })
            .stroke(egui::Stroke::new(
                1.0,
                if is_enabled { border } else { tokens.border },
            ))
            .corner_radius(tokens.rounding_md())
            .min_size(desired_size);

        let response = ui.add_enabled(is_enabled, button);

        // Cursor feedback
        if response.hovered() && is_enabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        response
    }
}

impl<'a> Button<'a> {
    fn colors(&self, theme: &crate::theme::Theme) -> (Color32, Color32, Color32) {
        let tokens = &theme.tokens;

        match self.variant {
            ButtonVariant::Primary => (tokens.primary, tokens.primary_foreground, tokens.primary),
            ButtonVariant::Secondary => {
                (tokens.secondary, tokens.secondary_foreground, tokens.border)
            }
            ButtonVariant::Outline => (Color32::TRANSPARENT, tokens.foreground, tokens.border),
            ButtonVariant::Ghost => (
                tokens.accent,
                tokens.accent_foreground,
                Color32::TRANSPARENT,
            ),
            ButtonVariant::Destructive => (
                tokens.destructive,
                tokens.destructive_foreground,
                tokens.destructive,
            ),
            ButtonVariant::Link => (Color32::TRANSPARENT, tokens.primary, Color32::TRANSPARENT),
            ButtonVariant::Success => (tokens.success, tokens.success_foreground, tokens.success),
        }
    }

    fn sizing(&self, theme: &crate::theme::Theme) -> (Vec2, f32, Vec2) {
        let tokens = &theme.tokens;

        match self.size {
            ButtonSize::Sm => (
                Vec2::new(tokens.spacing_sm, tokens.spacing_xs),
                tokens.font_size_sm,
                Vec2::new(0.0, 28.0),
            ),
            ButtonSize::Md => (
                Vec2::new(tokens.spacing_md, tokens.spacing_sm),
                tokens.font_size_md,
                Vec2::new(0.0, 36.0),
            ),
            ButtonSize::Lg => (
                Vec2::new(tokens.spacing_lg, tokens.spacing_md),
                tokens.font_size_lg,
                Vec2::new(0.0, 44.0),
            ),
            ButtonSize::Icon => (
                Vec2::new(tokens.spacing_sm, tokens.spacing_sm),
                tokens.font_size_md,
                Vec2::new(36.0, 36.0),
            ),
        }
    }
}

/// Icon-only button
pub struct IconButton<'a> {
    icon: &'a str,
    variant: ButtonVariant,
    size: ButtonSize,
    disabled: bool,
    tooltip: Option<&'a str>,
    selected: bool,
}

impl<'a> IconButton<'a> {
    /// Create a new icon button
    pub fn new(icon: &'a str) -> Self {
        Self {
            icon,
            variant: ButtonVariant::Ghost,
            size: ButtonSize::Icon,
            disabled: false,
            tooltip: None,
            selected: false,
        }
    }

    /// Set the variant
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Set as ghost (default for icon buttons)
    pub fn ghost(mut self) -> Self {
        self.variant = ButtonVariant::Ghost;
        self
    }

    /// Set as outline
    pub fn outline(mut self) -> Self {
        self.variant = ButtonVariant::Outline;
        self
    }

    /// Set as primary
    pub fn primary(mut self) -> Self {
        self.variant = ButtonVariant::Primary;
        self
    }

    /// Set size
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// Set small size
    pub fn small(mut self) -> Self {
        self.size = ButtonSize::Sm;
        self
    }

    /// Set disabled state
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Add tooltip on hover
    pub fn tooltip(mut self, text: &'a str) -> Self {
        self.tooltip = Some(text);
        self
    }

    /// Set selected state
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Show the icon button
    pub fn show(self, ui: &mut Ui) -> Response {
        self.ui(ui)
    }
}

impl<'a> Widget for IconButton<'a> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let size = match self.size {
            ButtonSize::Sm => 24.0,
            ButtonSize::Md | ButtonSize::Icon => 32.0,
            ButtonSize::Lg => 40.0,
        };

        let (bg, fg, border) = match self.variant {
            ButtonVariant::Ghost => (
                if self.selected {
                    tokens.accent
                } else {
                    Color32::TRANSPARENT
                },
                tokens.foreground,
                Color32::TRANSPARENT,
            ),
            ButtonVariant::Outline => (
                if self.selected {
                    tokens.accent
                } else {
                    Color32::TRANSPARENT
                },
                tokens.foreground,
                tokens.border,
            ),
            ButtonVariant::Primary => (tokens.primary, tokens.primary_foreground, tokens.primary),
            _ => (tokens.secondary, tokens.foreground, tokens.border),
        };

        let button =
            egui::Button::new(RichText::new(self.icon).size(tokens.font_size_md).color(fg))
                .fill(bg)
                .stroke(egui::Stroke::new(1.0, border))
                .corner_radius(tokens.rounding_md())
                .min_size(Vec2::splat(size));

        let response = ui.add_enabled(!self.disabled, button);

        if let Some(tip) = self.tooltip {
            response.clone().on_hover_text(tip);
        }

        if response.hovered() && !self.disabled {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_button_builder() {
        let button = Button::new("Test").primary().small().disabled(true);

        assert_eq!(button.variant, ButtonVariant::Primary);
        assert_eq!(button.size, ButtonSize::Sm);
        assert!(button.disabled);
    }

    #[test]
    fn test_icon_button_builder() {
        let button = IconButton::new("▶").ghost().tooltip("Play");

        assert_eq!(button.variant, ButtonVariant::Ghost);
        assert_eq!(button.tooltip, Some("Play"));
    }
}
