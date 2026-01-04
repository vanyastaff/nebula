//! Design tokens for theming.
//!
//! Design tokens are the smallest pieces of a design system - they define
//! colors, spacing, typography, and other visual properties.

use egui::Color32;

/// Design tokens - all visual properties centralized
#[derive(Clone, Debug)]
pub struct ThemeTokens {
    // ==================== Colors ====================
    /// Primary brand color (buttons, links, focus rings)
    pub primary: Color32,
    /// Text on primary color
    pub primary_foreground: Color32,

    /// Secondary color (less prominent elements)
    pub secondary: Color32,
    /// Text on secondary color
    pub secondary_foreground: Color32,

    /// Muted backgrounds (disabled states, subtle backgrounds)
    pub muted: Color32,
    /// Text on muted backgrounds
    pub muted_foreground: Color32,

    /// Accent color (highlights, selections)
    pub accent: Color32,
    /// Text on accent color
    pub accent_foreground: Color32,

    // ==================== Semantic Colors ====================
    /// Main background color
    pub background: Color32,
    /// Main foreground/text color
    pub foreground: Color32,

    /// Card/panel background
    pub card: Color32,
    /// Card foreground
    pub card_foreground: Color32,

    /// Border color
    pub border: Color32,
    /// Input field background
    pub input: Color32,
    /// Focus ring color
    pub ring: Color32,

    // ==================== Status Colors ====================
    /// Destructive/error actions
    pub destructive: Color32,
    /// Text on destructive
    pub destructive_foreground: Color32,

    /// Success states
    pub success: Color32,
    /// Text on success
    pub success_foreground: Color32,

    /// Warning states
    pub warning: Color32,
    /// Text on warning
    pub warning_foreground: Color32,

    /// Info states
    pub info: Color32,
    /// Text on info
    pub info_foreground: Color32,

    // ==================== Radii ====================
    /// Small radius (buttons, inputs)
    pub radius_sm: f32,
    /// Medium radius (cards, dialogs)
    pub radius_md: f32,
    /// Large radius (panels, modals)
    pub radius_lg: f32,
    /// Full/pill radius
    pub radius_full: f32,

    // ==================== Spacing ====================
    /// Extra small spacing (2px)
    pub spacing_xs: f32,
    /// Small spacing (4px)
    pub spacing_sm: f32,
    /// Medium spacing (8px)
    pub spacing_md: f32,
    /// Large spacing (16px)
    pub spacing_lg: f32,
    /// Extra large spacing (24px)
    pub spacing_xl: f32,
    /// 2x extra large spacing (32px)
    pub spacing_2xl: f32,

    // ==================== Typography ====================
    /// Extra small font size
    pub font_size_xs: f32,
    /// Small font size
    pub font_size_sm: f32,
    /// Base/medium font size
    pub font_size_md: f32,
    /// Large font size
    pub font_size_lg: f32,
    /// Extra large font size
    pub font_size_xl: f32,
    /// 2x extra large font size
    pub font_size_2xl: f32,

    // ==================== Shadows ====================
    /// Shadow color
    pub shadow_color: Color32,
    /// Shadow blur for small elements
    pub shadow_sm: f32,
    /// Shadow blur for medium elements
    pub shadow_md: f32,
    /// Shadow blur for large elements
    pub shadow_lg: f32,

    // ==================== Animations ====================
    /// Fast animation duration (ms)
    pub duration_fast: f32,
    /// Normal animation duration (ms)
    pub duration_normal: f32,
    /// Slow animation duration (ms)
    pub duration_slow: f32,
}

impl Default for ThemeTokens {
    fn default() -> Self {
        Self::dark()
    }
}

impl ThemeTokens {
    /// Create dark theme tokens
    #[must_use]
    pub fn dark() -> Self {
        Self {
            // Primary (Indigo)
            primary: Color32::from_rgb(99, 102, 241),
            primary_foreground: Color32::WHITE,

            // Secondary
            secondary: Color32::from_rgb(39, 39, 42),
            secondary_foreground: Color32::from_rgb(250, 250, 250),

            // Muted
            muted: Color32::from_rgb(39, 39, 42),
            muted_foreground: Color32::from_rgb(161, 161, 170),

            // Accent
            accent: Color32::from_rgb(39, 39, 42),
            accent_foreground: Color32::from_rgb(250, 250, 250),

            // Background/Foreground
            background: Color32::from_rgb(9, 9, 11),
            foreground: Color32::from_rgb(250, 250, 250),

            // Card
            card: Color32::from_rgb(24, 24, 27),
            card_foreground: Color32::from_rgb(250, 250, 250),

            // Borders & Input
            border: Color32::from_rgb(39, 39, 42),
            input: Color32::from_rgb(39, 39, 42),
            ring: Color32::from_rgb(99, 102, 241),

            // Status: Destructive (Red)
            destructive: Color32::from_rgb(239, 68, 68),
            destructive_foreground: Color32::WHITE,

            // Status: Success (Green)
            success: Color32::from_rgb(34, 197, 94),
            success_foreground: Color32::WHITE,

            // Status: Warning (Yellow)
            warning: Color32::from_rgb(234, 179, 8),
            warning_foreground: Color32::from_rgb(24, 24, 27),

            // Status: Info (Blue)
            info: Color32::from_rgb(59, 130, 246),
            info_foreground: Color32::WHITE,

            // Radii
            radius_sm: 4.0,
            radius_md: 6.0,
            radius_lg: 8.0,
            radius_full: 9999.0,

            // Spacing
            spacing_xs: 2.0,
            spacing_sm: 4.0,
            spacing_md: 8.0,
            spacing_lg: 16.0,
            spacing_xl: 24.0,
            spacing_2xl: 32.0,

            // Typography
            font_size_xs: 10.0,
            font_size_sm: 12.0,
            font_size_md: 14.0,
            font_size_lg: 16.0,
            font_size_xl: 20.0,
            font_size_2xl: 24.0,

            // Shadows
            shadow_color: Color32::from_black_alpha(60),
            shadow_sm: 2.0,
            shadow_md: 4.0,
            shadow_lg: 8.0,

            // Animations
            duration_fast: 100.0,
            duration_normal: 200.0,
            duration_slow: 300.0,
        }
    }

    /// Create light theme tokens
    #[must_use]
    pub fn light() -> Self {
        Self {
            // Primary (Indigo - slightly darker for contrast)
            primary: Color32::from_rgb(79, 70, 229),
            primary_foreground: Color32::WHITE,

            // Secondary
            secondary: Color32::from_rgb(244, 244, 245),
            secondary_foreground: Color32::from_rgb(24, 24, 27),

            // Muted
            muted: Color32::from_rgb(244, 244, 245),
            muted_foreground: Color32::from_rgb(113, 113, 122),

            // Accent
            accent: Color32::from_rgb(244, 244, 245),
            accent_foreground: Color32::from_rgb(24, 24, 27),

            // Background/Foreground
            background: Color32::WHITE,
            foreground: Color32::from_rgb(9, 9, 11),

            // Card
            card: Color32::WHITE,
            card_foreground: Color32::from_rgb(9, 9, 11),

            // Borders & Input
            border: Color32::from_rgb(228, 228, 231),
            input: Color32::from_rgb(228, 228, 231),
            ring: Color32::from_rgb(79, 70, 229),

            // Status: Destructive (Red - darker)
            destructive: Color32::from_rgb(220, 38, 38),
            destructive_foreground: Color32::WHITE,

            // Status: Success (Green - darker)
            success: Color32::from_rgb(22, 163, 74),
            success_foreground: Color32::WHITE,

            // Status: Warning (Yellow - darker)
            warning: Color32::from_rgb(202, 138, 4),
            warning_foreground: Color32::from_rgb(24, 24, 27),

            // Status: Info (Blue - darker)
            info: Color32::from_rgb(37, 99, 235),
            info_foreground: Color32::WHITE,

            // Radii (same as dark)
            radius_sm: 4.0,
            radius_md: 6.0,
            radius_lg: 8.0,
            radius_full: 9999.0,

            // Spacing (same as dark)
            spacing_xs: 2.0,
            spacing_sm: 4.0,
            spacing_md: 8.0,
            spacing_lg: 16.0,
            spacing_xl: 24.0,
            spacing_2xl: 32.0,

            // Typography (same as dark)
            font_size_xs: 10.0,
            font_size_sm: 12.0,
            font_size_md: 14.0,
            font_size_lg: 16.0,
            font_size_xl: 20.0,
            font_size_2xl: 24.0,

            // Shadows (lighter)
            shadow_color: Color32::from_black_alpha(25),
            shadow_sm: 2.0,
            shadow_md: 4.0,
            shadow_lg: 8.0,

            // Animations (same as dark)
            duration_fast: 100.0,
            duration_normal: 200.0,
            duration_slow: 300.0,
        }
    }

    /// Get egui CornerRadius from radius
    #[must_use]
    pub fn rounding_sm(&self) -> egui::CornerRadius {
        egui::CornerRadius::same(self.radius_sm as u8)
    }

    /// Get egui CornerRadius from radius
    #[must_use]
    pub fn rounding_md(&self) -> egui::CornerRadius {
        egui::CornerRadius::same(self.radius_md as u8)
    }

    /// Get egui CornerRadius from radius
    #[must_use]
    pub fn rounding_lg(&self) -> egui::CornerRadius {
        egui::CornerRadius::same(self.radius_lg as u8)
    }

    /// Get egui Stroke for borders
    #[must_use]
    pub fn border_stroke(&self) -> egui::Stroke {
        egui::Stroke::new(1.0, self.border)
    }

    /// Get egui Stroke for focus rings
    #[must_use]
    pub fn ring_stroke(&self) -> egui::Stroke {
        egui::Stroke::new(2.0, self.ring)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dark_tokens() {
        let tokens = ThemeTokens::dark();
        // Dark theme should have dark background
        assert!(tokens.background.r() < 50);
    }

    #[test]
    fn test_light_tokens() {
        let tokens = ThemeTokens::light();
        // Light theme should have light background
        assert!(tokens.background.r() > 200);
    }
}
