//! Pan/zoom canvas for the flow editor.

use egui::{Pos2, Rect, Response, Sense, Ui, Vec2};

/// Canvas state (pan and zoom)
#[derive(Clone, Debug)]
pub struct CanvasState {
    /// Current pan offset
    pub pan: Vec2,
    /// Current zoom level
    pub zoom: f32,
    /// Minimum zoom
    pub min_zoom: f32,
    /// Maximum zoom
    pub max_zoom: f32,
    /// Current viewport rect (updated each frame)
    pub viewport: Rect,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            min_zoom: 0.1,
            max_zoom: 4.0,
            viewport: Rect::NOTHING,
        }
    }
}

impl CanvasState {
    /// Create a new canvas state
    pub fn new() -> Self {
        Self::default()
    }

    /// Set zoom limits
    pub fn with_zoom_limits(mut self, min: f32, max: f32) -> Self {
        self.min_zoom = min;
        self.max_zoom = max;
        self
    }

    /// Convert screen position to canvas position
    pub fn screen_to_canvas(&self, screen_pos: Pos2, canvas_rect: Rect) -> Pos2 {
        let relative = screen_pos - canvas_rect.min.to_vec2();
        Pos2::new(
            (relative.x - self.pan.x) / self.zoom,
            (relative.y - self.pan.y) / self.zoom,
        )
    }

    /// Convert canvas position to screen position
    pub fn canvas_to_screen(&self, canvas_pos: Pos2, canvas_rect: Rect) -> Pos2 {
        Pos2::new(
            canvas_pos.x * self.zoom + self.pan.x + canvas_rect.min.x,
            canvas_pos.y * self.zoom + self.pan.y + canvas_rect.min.y,
        )
    }

    /// Zoom at a specific screen position
    pub fn zoom_at(&mut self, screen_pos: Pos2, canvas_rect: Rect, delta: f32) {
        let old_zoom = self.zoom;
        self.zoom = (self.zoom * (1.0 + delta)).clamp(self.min_zoom, self.max_zoom);

        if (self.zoom - old_zoom).abs() > f32::EPSILON {
            // Adjust pan to zoom towards cursor position
            let relative = screen_pos - canvas_rect.min.to_vec2();
            let scale_change = self.zoom / old_zoom;
            self.pan = relative - (relative - self.pan) * scale_change;
        }
    }

    /// Reset to default view
    pub fn reset(&mut self) {
        self.pan = Vec2::ZERO;
        self.zoom = 1.0;
    }

    /// Fit content to view
    pub fn fit_to_content(&mut self, content_bounds: Rect, canvas_rect: Rect, padding: f32) {
        if content_bounds.is_positive() {
            let content_size = content_bounds.size() + Vec2::splat(padding * 2.0);
            let canvas_size = canvas_rect.size();

            // Calculate zoom to fit
            let zoom_x = canvas_size.x / content_size.x;
            let zoom_y = canvas_size.y / content_size.y;
            self.zoom = zoom_x.min(zoom_y).clamp(self.min_zoom, self.max_zoom);

            // Center content
            let scaled_content = content_bounds.size() * self.zoom;
            self.pan = Vec2::new(
                (canvas_size.x - scaled_content.x) / 2.0 - content_bounds.min.x * self.zoom,
                (canvas_size.y - scaled_content.y) / 2.0 - content_bounds.min.y * self.zoom,
            );
        }
    }
}

/// A pan/zoom canvas widget
pub struct Canvas<'a> {
    state: &'a mut CanvasState,
    show_grid: bool,
    grid_size: f32,
}

impl<'a> Canvas<'a> {
    /// Create a new canvas
    pub fn new(state: &'a mut CanvasState) -> Self {
        Self {
            state,
            show_grid: true,
            grid_size: 20.0,
        }
    }

    /// Set grid visibility
    pub fn grid(mut self, show: bool) -> Self {
        self.show_grid = show;
        self
    }

    /// Set grid size
    pub fn grid_size(mut self, size: f32) -> Self {
        self.grid_size = size;
        self
    }

    /// Show the canvas
    pub fn show<R>(
        self,
        ui: &mut Ui,
        add_contents: impl FnOnce(&mut Ui, &CanvasState, Rect) -> R,
    ) -> CanvasResponse<R> {
        let theme = crate::theme::current_theme();
        let tokens = &theme.tokens;

        // Allocate full available space
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());

        // Handle pan (middle mouse or space+drag)
        if response.dragged_by(egui::PointerButton::Middle) {
            self.state.pan += response.drag_delta();
        }

        // Handle zoom (scroll wheel)
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta != 0.0 && response.hovered() {
            if let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos()) {
                self.state.zoom_at(pointer_pos, rect, scroll_delta * 0.001);
            }
        }

        // Draw background
        ui.painter().rect_filled(rect, 0.0, tokens.background);

        // Draw grid
        if self.show_grid {
            self.draw_grid(ui, rect);
        }

        // Clip to canvas bounds
        ui.set_clip_rect(rect);

        // Create a transform for content
        let inner = add_contents(ui, self.state, rect);

        CanvasResponse {
            inner,
            response,
            rect,
        }
    }

    fn draw_grid(&self, ui: &mut Ui, rect: Rect) {
        let theme = crate::theme::current_theme();
        let tokens = &theme.tokens;

        let grid_color = tokens.border.gamma_multiply(0.5);
        let major_grid_color = tokens.border;

        let painter = ui.painter();

        let scaled_grid = self.grid_size * self.state.zoom;
        if scaled_grid < 5.0 {
            return; // Grid too small to draw
        }

        // Calculate grid offset
        let offset = Vec2::new(
            self.state.pan.x % scaled_grid,
            self.state.pan.y % scaled_grid,
        );

        let major_every = 5;

        // Vertical lines
        let mut x = rect.min.x + offset.x;
        let mut i = ((-self.state.pan.x / scaled_grid) as i32).max(0);
        while x < rect.max.x {
            let color = if i % major_every == 0 {
                major_grid_color
            } else {
                grid_color
            };

            painter.line_segment(
                [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
                egui::Stroke::new(1.0, color),
            );

            x += scaled_grid;
            i += 1;
        }

        // Horizontal lines
        let mut y = rect.min.y + offset.y;
        let mut j = ((-self.state.pan.y / scaled_grid) as i32).max(0);
        while y < rect.max.y {
            let color = if j % major_every == 0 {
                major_grid_color
            } else {
                grid_color
            };

            painter.line_segment(
                [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
                egui::Stroke::new(1.0, color),
            );

            y += scaled_grid;
            j += 1;
        }
    }
}

/// Response from showing a canvas
pub struct CanvasResponse<R> {
    /// Inner content's return value
    pub inner: R,
    /// Canvas response
    pub response: Response,
    /// Canvas rectangle
    pub rect: Rect,
}

impl<R> CanvasResponse<R> {
    /// Check if canvas was clicked
    pub fn clicked(&self) -> bool {
        self.response.clicked()
    }

    /// Check if right-clicked (context menu)
    pub fn context_clicked(&self) -> bool {
        self.response.secondary_clicked()
    }

    /// Check if hovered
    pub fn hovered(&self) -> bool {
        self.response.hovered()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canvas_state_default() {
        let state = CanvasState::default();
        assert_eq!(state.pan, Vec2::ZERO);
        assert_eq!(state.zoom, 1.0);
    }

    #[test]
    fn test_screen_to_canvas() {
        let state = CanvasState {
            pan: Vec2::new(100.0, 50.0),
            zoom: 2.0,
            ..Default::default()
        };

        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let screen = Pos2::new(200.0, 150.0);

        let canvas = state.screen_to_canvas(screen, rect);

        // (200 - 100) / 2 = 50
        // (150 - 50) / 2 = 50
        assert_eq!(canvas, Pos2::new(50.0, 50.0));
    }
}
