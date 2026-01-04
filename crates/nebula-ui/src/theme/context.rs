//! Global theme context and egui integration.

use super::Theme;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::Arc;

/// Global theme storage
static THEME: Lazy<Arc<RwLock<Theme>>> = Lazy::new(|| Arc::new(RwLock::new(Theme::dark())));

/// Get the current theme
#[must_use]
pub fn current_theme() -> Theme {
    THEME.read().clone()
}

/// Set the current theme
pub fn set_theme(theme: Theme) {
    *THEME.write() = theme;
}

/// Toggle between dark and light themes
pub fn toggle_theme() {
    let mut theme = THEME.write();
    *theme = if theme.is_dark {
        Theme::light()
    } else {
        Theme::dark()
    };
}

/// Apply theme to egui context
pub fn apply_theme(ctx: &egui::Context, theme: &Theme) {
    let tokens = &theme.tokens;

    let mut style = (*ctx.style()).clone();

    // Configure visuals
    style.visuals = egui::Visuals {
        dark_mode: theme.is_dark,

        override_text_color: Some(tokens.foreground),

        widgets: egui::style::Widgets {
            noninteractive: widget_visuals(tokens, WidgetState::Noninteractive),
            inactive: widget_visuals(tokens, WidgetState::Inactive),
            hovered: widget_visuals(tokens, WidgetState::Hovered),
            active: widget_visuals(tokens, WidgetState::Active),
            open: widget_visuals(tokens, WidgetState::Open),
        },

        selection: egui::style::Selection {
            bg_fill: with_alpha(tokens.primary, 80),
            stroke: egui::Stroke::new(1.0, tokens.primary),
        },

        hyperlink_color: tokens.primary,

        faint_bg_color: tokens.muted,
        extreme_bg_color: tokens.background,
        code_bg_color: tokens.muted,

        warn_fg_color: tokens.warning,
        error_fg_color: tokens.destructive,

        window_corner_radius: tokens.rounding_lg(),
        window_shadow: egui::Shadow {
            offset: [0, 2],
            blur: tokens.shadow_md as u8,
            spread: 0,
            color: tokens.shadow_color,
        },
        window_fill: tokens.card,
        window_stroke: tokens.border_stroke(),
        window_highlight_topmost: true,

        menu_corner_radius: tokens.rounding_md(),

        panel_fill: tokens.background,

        popup_shadow: egui::Shadow {
            offset: [0, 4],
            blur: tokens.shadow_lg as u8,
            spread: 0,
            color: tokens.shadow_color,
        },

        resize_corner_size: 12.0,

        text_cursor: egui::style::TextCursorStyle {
            stroke: egui::Stroke::new(2.0, tokens.foreground),
            preview: false,
            blink: true,
            on_duration: 0.5,
            off_duration: 0.5,
        },

        clip_rect_margin: 3.0,
        button_frame: true,
        collapsing_header_frame: false,
        indent_has_left_vline: true,

        striped: false,

        slider_trailing_fill: true,

        handle_shape: egui::style::HandleShape::Circle,

        interact_cursor: Some(egui::CursorIcon::PointingHand),

        image_loading_spinners: true,

        numeric_color_space: egui::style::NumericColorSpace::GammaByte,

        // New fields in egui 0.33
        text_alpha_from_coverage: if theme.is_dark {
            epaint::AlphaFromCoverage::DARK_MODE_DEFAULT
        } else {
            epaint::AlphaFromCoverage::LIGHT_MODE_DEFAULT
        },
        text_edit_bg_color: None,
        disabled_alpha: 0.5,
        weak_text_color: Some(tokens.muted_foreground),
        weak_text_alpha: 0.7,
    };

    // Configure spacing
    style.spacing = egui::Spacing {
        item_spacing: egui::vec2(tokens.spacing_md, tokens.spacing_sm),
        window_margin: egui::Margin::same(tokens.spacing_lg as i8),
        button_padding: egui::vec2(tokens.spacing_md, tokens.spacing_sm),
        menu_margin: egui::Margin::same(tokens.spacing_sm as i8),
        indent: tokens.spacing_lg,
        interact_size: egui::vec2(40.0, 20.0),
        slider_width: 100.0,
        slider_rail_height: 8.0,
        combo_width: 100.0,
        text_edit_width: 280.0,
        icon_width: 14.0,
        icon_width_inner: 8.0,
        icon_spacing: tokens.spacing_sm,
        tooltip_width: 600.0,
        menu_width: 180.0,
        menu_spacing: 2.0,
        combo_height: 200.0,
        scroll: egui::style::ScrollStyle::solid(),
        indent_ends_with_horizontal_line: false,
        default_area_size: egui::vec2(300.0, 150.0),
    };

    // Configure interaction
    style.interaction = egui::style::Interaction {
        interact_radius: 5.0,
        resize_grab_radius_side: 5.0,
        resize_grab_radius_corner: 10.0,
        show_tooltips_only_when_still: true,
        tooltip_delay: 0.5,
        tooltip_grace_time: 0.2,
        selectable_labels: true,
        multi_widget_text_select: true,
    };

    // Apply
    ctx.set_style(style);
}

#[derive(Clone, Copy)]
enum WidgetState {
    Noninteractive,
    Inactive,
    Hovered,
    Active,
    Open,
}

fn widget_visuals(tokens: &super::ThemeTokens, state: WidgetState) -> egui::style::WidgetVisuals {
    match state {
        WidgetState::Noninteractive => egui::style::WidgetVisuals {
            bg_fill: tokens.muted,
            weak_bg_fill: tokens.muted,
            bg_stroke: egui::Stroke::new(1.0, tokens.border),
            corner_radius: tokens.rounding_md(),
            fg_stroke: egui::Stroke::new(1.0, tokens.foreground),
            expansion: 0.0,
        },
        WidgetState::Inactive => egui::style::WidgetVisuals {
            bg_fill: tokens.secondary,
            weak_bg_fill: with_alpha(tokens.secondary, 180),
            bg_stroke: egui::Stroke::new(1.0, tokens.border),
            corner_radius: tokens.rounding_md(),
            fg_stroke: egui::Stroke::new(1.0, tokens.foreground),
            expansion: 0.0,
        },
        WidgetState::Hovered => egui::style::WidgetVisuals {
            bg_fill: tokens.accent,
            weak_bg_fill: tokens.accent,
            bg_stroke: egui::Stroke::new(1.0, tokens.border),
            corner_radius: tokens.rounding_md(),
            fg_stroke: egui::Stroke::new(1.5, tokens.foreground),
            expansion: 1.0,
        },
        WidgetState::Active => egui::style::WidgetVisuals {
            bg_fill: tokens.primary,
            weak_bg_fill: tokens.primary,
            bg_stroke: egui::Stroke::new(1.0, tokens.primary),
            corner_radius: tokens.rounding_md(),
            fg_stroke: egui::Stroke::new(2.0, tokens.primary_foreground),
            expansion: 1.0,
        },
        WidgetState::Open => egui::style::WidgetVisuals {
            bg_fill: tokens.accent,
            weak_bg_fill: tokens.accent,
            bg_stroke: egui::Stroke::new(1.0, tokens.ring),
            corner_radius: tokens.rounding_md(),
            fg_stroke: egui::Stroke::new(1.0, tokens.foreground),
            expansion: 0.0,
        },
    }
}

fn with_alpha(color: egui::Color32, alpha: u8) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_theme_default() {
        let theme = current_theme();
        assert!(theme.is_dark);
    }

    #[test]
    fn test_toggle_theme() {
        // Reset to dark
        set_theme(Theme::dark());
        assert!(current_theme().is_dark);

        toggle_theme();
        assert!(!current_theme().is_dark);

        toggle_theme();
        assert!(current_theme().is_dark);
    }
}
