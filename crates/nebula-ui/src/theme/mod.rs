//! Theme system for nebula-ui.
//!
//! This module provides comprehensive theming with design tokens,
//! supporting dark/light modes and consistent styling across components.
//!
//! ## Design Tokens
//!
//! All visual properties are centralized in [`ThemeTokens`]:
//! - Colors (primary, secondary, semantic colors)
//! - Spacing (xs, sm, md, lg, xl)
//! - Radii (sm, md, lg for border radius)
//!
//! ## Usage
//!
//! ```rust,ignore
//! use nebula_ui::theme::{Theme, current_theme, set_theme};
//!
//! // Get current theme
//! let theme = current_theme();
//!
//! // Switch themes
//! set_theme(Theme::light());
//!
//! // Apply to egui context
//! theme.apply(ctx);
//! ```

mod colors;
mod context;
mod tokens;

pub use colors::{DataTypeColors, NodeCategoryColors};
pub use context::{apply_theme, current_theme, set_theme, toggle_theme};
pub use tokens::ThemeTokens;

use egui::Color32;

/// Complete theme definition
#[derive(Clone, Debug)]
pub struct Theme {
    /// Theme name
    pub name: String,
    /// Whether this is a dark theme
    pub is_dark: bool,
    /// Design tokens (colors, spacing, radii)
    pub tokens: ThemeTokens,
    /// Colors for data types (pins, connections)
    pub data_types: DataTypeColors,
    /// Colors for node categories
    pub node_categories: NodeCategoryColors,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// Create the default dark theme
    #[must_use]
    pub fn dark() -> Self {
        Self {
            name: "Dark".into(),
            is_dark: true,
            tokens: ThemeTokens::dark(),
            data_types: DataTypeColors::default(),
            node_categories: NodeCategoryColors::default(),
        }
    }

    /// Create a light theme
    #[must_use]
    pub fn light() -> Self {
        Self {
            name: "Light".into(),
            is_dark: false,
            tokens: ThemeTokens::light(),
            data_types: DataTypeColors::default(),
            node_categories: NodeCategoryColors::default(),
        }
    }

    /// Get color for a data type
    #[must_use]
    pub fn color_for_data_type(&self, data_type: &crate::flow::DataType) -> Color32 {
        use crate::flow::DataType;
        match data_type {
            DataType::Execution => self.data_types.execution,
            DataType::String => self.data_types.string,
            DataType::Number => self.data_types.number,
            DataType::Boolean => self.data_types.boolean,
            DataType::Array(_) => self.data_types.array,
            DataType::Object => self.data_types.object,
            DataType::Generic => self.data_types.generic,
            DataType::Struct(_) => self.data_types.structure,
            DataType::Bytes => self.data_types.bytes,
        }
    }

    /// Get color for a node category
    #[must_use]
    pub fn color_for_category(&self, category: &str) -> Color32 {
        match category.to_lowercase().as_str() {
            "control" | "flow" | "logic" => self.node_categories.control_flow,
            "data" | "transform" | "variable" => self.node_categories.data,
            "io" | "file" | "http" | "network" => self.node_categories.io,
            "ai" | "llm" | "ml" | "embedding" => self.node_categories.ai,
            "utility" | "debug" | "log" => self.node_categories.utility,
            "event" | "trigger" | "webhook" => self.node_categories.event,
            _ => self.node_categories.custom,
        }
    }

    /// Apply this theme to an egui context
    pub fn apply(&self, ctx: &egui::Context) {
        set_theme(self.clone());
        apply_theme(ctx, self);
    }
}

/// Helper to mix two colors
#[must_use]
pub fn color_mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let inv_t = 1.0 - t;
    Color32::from_rgba_unmultiplied(
        (f32::from(a.r()) * inv_t + f32::from(b.r()) * t) as u8,
        (f32::from(a.g()) * inv_t + f32::from(b.g()) * t) as u8,
        (f32::from(a.b()) * inv_t + f32::from(b.b()) * t) as u8,
        (f32::from(a.a()) * inv_t + f32::from(b.a()) * t) as u8,
    )
}

/// Helper to adjust color brightness
#[must_use]
pub fn color_brightness(color: Color32, factor: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        (f32::from(color.r()) * factor).min(255.0) as u8,
        (f32::from(color.g()) * factor).min(255.0) as u8,
        (f32::from(color.b()) * factor).min(255.0) as u8,
        color.a(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dark_theme() {
        let theme = Theme::dark();
        assert!(theme.is_dark);
        assert_eq!(theme.name, "Dark");
    }

    #[test]
    fn test_light_theme() {
        let theme = Theme::light();
        assert!(!theme.is_dark);
        assert_eq!(theme.name, "Light");
    }

    #[test]
    fn test_color_mix() {
        let black = Color32::BLACK;
        let white = Color32::WHITE;

        let mid = color_mix(black, white, 0.5);
        assert_eq!(mid.r(), 127);
        assert_eq!(mid.g(), 127);
        assert_eq!(mid.b(), 127);
    }
}
