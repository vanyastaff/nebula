//! Background patterns for flow editor (dots, lines, cross).

use egui::{Color32, Pos2, Rect, Ui, Vec2};

use crate::theme;

/// Type of background pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackgroundVariant {
    /// Dotted pattern (like ReactFlow default).
    #[default]
    Dots,
    /// Line grid pattern.
    Lines,
    /// Cross pattern (dots + lines).
    Cross,
}

/// Configuration for background rendering.
#[derive(Debug, Clone)]
pub struct BackgroundConfig {
    /// Pattern variant.
    pub variant: BackgroundVariant,
    /// Gap between dots/lines.
    pub gap: f32,
    /// Size of dots (for Dots variant).
    pub dot_size: f32,
    /// Color opacity (0.0 - 1.0).
    pub opacity: f32,
    /// Whether to show major lines/dots (every N gaps).
    pub show_major: bool,
    /// Major line frequency (every N gaps).
    pub major_frequency: u32,
    /// Major line/dot opacity multiplier.
    pub major_opacity_multiplier: f32,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            variant: BackgroundVariant::Dots,
            gap: 20.0,
            dot_size: 1.5,
            opacity: 0.3,
            show_major: true,
            major_frequency: 5,
            major_opacity_multiplier: 2.0,
        }
    }
}

/// Background pattern renderer.
pub struct Background {
    config: BackgroundConfig,
}

impl Background {
    /// Creates a new background renderer.
    pub fn new() -> Self {
        Self {
            config: BackgroundConfig::default(),
        }
    }

    /// Sets the configuration.
    pub fn config(mut self, config: BackgroundConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the variant.
    pub fn variant(mut self, variant: BackgroundVariant) -> Self {
        self.config.variant = variant;
        self
    }

    /// Sets the gap.
    pub fn gap(mut self, gap: f32) -> Self {
        self.config.gap = gap;
        self
    }

    /// Draws the background pattern.
    pub fn draw(&self, ui: &mut Ui, rect: Rect, pan: Vec2, zoom: f32) {
        let theme = theme::current_theme();
        let tokens = &theme.tokens;

        // Draw base background color
        ui.painter().rect_filled(rect, 0.0, tokens.background);

        // Calculate colors
        let base_alpha = (255.0 * self.config.opacity) as u8;
        let major_alpha =
            (base_alpha as f32 * self.config.major_opacity_multiplier).min(255.0) as u8;

        let base_color = Color32::from_rgba_unmultiplied(
            tokens.border.r(),
            tokens.border.g(),
            tokens.border.b(),
            base_alpha,
        );

        let major_color = Color32::from_rgba_unmultiplied(
            tokens.border.r(),
            tokens.border.g(),
            tokens.border.b(),
            major_alpha,
        );

        match self.config.variant {
            BackgroundVariant::Dots => self.draw_dots(ui, rect, pan, zoom, base_color, major_color),
            BackgroundVariant::Lines => {
                self.draw_lines(ui, rect, pan, zoom, base_color, major_color)
            }
            BackgroundVariant::Cross => {
                self.draw_lines(ui, rect, pan, zoom, base_color, major_color);
                self.draw_dots(ui, rect, pan, zoom, base_color, major_color);
            }
        }
    }

    fn draw_dots(
        &self,
        ui: &mut Ui,
        rect: Rect,
        pan: Vec2,
        zoom: f32,
        base_color: Color32,
        major_color: Color32,
    ) {
        let scaled_gap = self.config.gap * zoom;

        // Don't draw if gap is too small
        if scaled_gap < 5.0 {
            return;
        }

        let painter = ui.painter();
        let dot_radius = self.config.dot_size * zoom;

        // Calculate grid offset
        let offset = Vec2::new(pan.x % scaled_gap, pan.y % scaled_gap);

        let start_x = rect.min.x + offset.x;
        let start_y = rect.min.y + offset.y;

        // Calculate grid indices
        let grid_start_x = ((-pan.x / scaled_gap).floor() as i32).max(0);
        let grid_start_y = ((-pan.y / scaled_gap).floor() as i32).max(0);

        let mut y = start_y;
        let mut grid_y = grid_start_y;

        while y < rect.max.y {
            let mut x = start_x;
            let mut grid_x = grid_start_x;

            while x < rect.max.x {
                let is_major = self.config.show_major
                    && grid_x % self.config.major_frequency as i32 == 0
                    && grid_y % self.config.major_frequency as i32 == 0;

                let color = if is_major { major_color } else { base_color };

                let dot_size = if is_major {
                    dot_radius * 1.5
                } else {
                    dot_radius
                };

                painter.circle_filled(Pos2::new(x, y), dot_size, color);

                x += scaled_gap;
                grid_x += 1;
            }

            y += scaled_gap;
            grid_y += 1;
        }
    }

    fn draw_lines(
        &self,
        ui: &mut Ui,
        rect: Rect,
        pan: Vec2,
        zoom: f32,
        base_color: Color32,
        major_color: Color32,
    ) {
        let scaled_gap = self.config.gap * zoom;

        // Don't draw if gap is too small
        if scaled_gap < 5.0 {
            return;
        }

        let painter = ui.painter();

        // Calculate grid offset
        let offset = Vec2::new(pan.x % scaled_gap, pan.y % scaled_gap);

        // Draw vertical lines
        let mut x = rect.min.x + offset.x;
        let mut grid_x = ((-pan.x / scaled_gap).floor() as i32).max(0);

        while x < rect.max.x {
            let is_major =
                self.config.show_major && grid_x % self.config.major_frequency as i32 == 0;

            let color = if is_major { major_color } else { base_color };
            let width = if is_major { 1.0 } else { 0.5 };

            painter.line_segment(
                [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
                egui::Stroke::new(width, color),
            );

            x += scaled_gap;
            grid_x += 1;
        }

        // Draw horizontal lines
        let mut y = rect.min.y + offset.y;
        let mut grid_y = ((-pan.y / scaled_gap).floor() as i32).max(0);

        while y < rect.max.y {
            let is_major =
                self.config.show_major && grid_y % self.config.major_frequency as i32 == 0;

            let color = if is_major { major_color } else { base_color };
            let width = if is_major { 1.0 } else { 0.5 };

            painter.line_segment(
                [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
                egui::Stroke::new(width, color),
            );

            y += scaled_gap;
            grid_y += 1;
        }
    }
}

impl Default for Background {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_background_config_default() {
        let config = BackgroundConfig::default();
        assert_eq!(config.variant, BackgroundVariant::Dots);
        assert_eq!(config.gap, 20.0);
        assert!(config.show_major);
    }

    #[test]
    fn test_background_builder() {
        let bg = Background::new()
            .variant(BackgroundVariant::Lines)
            .gap(30.0);
        assert_eq!(bg.config.variant, BackgroundVariant::Lines);
        assert_eq!(bg.config.gap, 30.0);
    }
}
