//! Resizable panel components.

use crate::theme::current_theme;
use egui::{CursorIcon, Id, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

/// Direction for resizable panels
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResizeDirection {
    /// Horizontal resize (left-right)
    #[default]
    Horizontal,
    /// Vertical resize (top-bottom)
    Vertical,
}

/// Resizable panel handle
pub struct ResizeHandle {
    id: Id,
    direction: ResizeDirection,
    position: f32,
    min_position: f32,
    max_position: f32,
    handle_width: f32,
}

impl ResizeHandle {
    /// Create a new resize handle
    pub fn new(id: impl std::hash::Hash, direction: ResizeDirection) -> Self {
        Self {
            id: Id::new(id),
            direction,
            position: 0.5,
            min_position: 0.1,
            max_position: 0.9,
            handle_width: 8.0,
        }
    }

    /// Set initial position (0.0 - 1.0)
    pub fn position(mut self, pos: f32) -> Self {
        self.position = pos.clamp(0.0, 1.0);
        self
    }

    /// Set minimum position
    pub fn min(mut self, min: f32) -> Self {
        self.min_position = min.clamp(0.0, 1.0);
        self
    }

    /// Set maximum position
    pub fn max(mut self, max: f32) -> Self {
        self.max_position = max.clamp(0.0, 1.0);
        self
    }

    /// Set handle width
    pub fn handle_width(mut self, width: f32) -> Self {
        self.handle_width = width;
        self
    }

    /// Show the resize handle and return new position
    pub fn show(self, ui: &mut Ui, rect: Rect) -> ResizeHandleResponse {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let handle_rect = match self.direction {
            ResizeDirection::Horizontal => {
                let x = rect.min.x + rect.width() * self.position - self.handle_width / 2.0;
                Rect::from_min_size(
                    Pos2::new(x, rect.min.y),
                    Vec2::new(self.handle_width, rect.height()),
                )
            }
            ResizeDirection::Vertical => {
                let y = rect.min.y + rect.height() * self.position - self.handle_width / 2.0;
                Rect::from_min_size(
                    Pos2::new(rect.min.x, y),
                    Vec2::new(rect.width(), self.handle_width),
                )
            }
        };

        let response = ui.interact(handle_rect, self.id, Sense::drag());

        // Update cursor
        if response.hovered() || response.dragged() {
            let cursor = match self.direction {
                ResizeDirection::Horizontal => CursorIcon::ResizeHorizontal,
                ResizeDirection::Vertical => CursorIcon::ResizeVertical,
            };
            ui.ctx().set_cursor_icon(cursor);
        }

        // Calculate new position from drag
        let mut new_position = self.position;
        if response.dragged() {
            let delta = response.drag_delta();
            let delta_fraction = match self.direction {
                ResizeDirection::Horizontal => delta.x / rect.width(),
                ResizeDirection::Vertical => delta.y / rect.height(),
            };
            new_position =
                (self.position + delta_fraction).clamp(self.min_position, self.max_position);
        }

        // Draw handle
        let handle_color = if response.dragged() {
            tokens.primary
        } else if response.hovered() {
            tokens.border
        } else {
            tokens.border.gamma_multiply(0.5)
        };

        ui.painter().rect_filled(handle_rect, 0.0, handle_color);

        // Draw grip dots
        let center = handle_rect.center();
        let dot_color = tokens.muted_foreground;
        let dot_radius = 1.5;
        let dot_spacing = 4.0;

        match self.direction {
            ResizeDirection::Horizontal => {
                for i in -1..=1 {
                    ui.painter().circle_filled(
                        Pos2::new(center.x, center.y + i as f32 * dot_spacing),
                        dot_radius,
                        dot_color,
                    );
                }
            }
            ResizeDirection::Vertical => {
                for i in -1..=1 {
                    ui.painter().circle_filled(
                        Pos2::new(center.x + i as f32 * dot_spacing, center.y),
                        dot_radius,
                        dot_color,
                    );
                }
            }
        }

        ResizeHandleResponse {
            position: new_position,
            dragging: response.dragged(),
            hovered: response.hovered(),
        }
    }
}

/// Response from resize handle
#[derive(Clone, Debug)]
pub struct ResizeHandleResponse {
    /// New position (0.0 - 1.0)
    pub position: f32,
    /// Whether handle is being dragged
    pub dragging: bool,
    /// Whether handle is hovered
    pub hovered: bool,
}

/// Two-panel resizable layout
///
/// # Example
///
/// ```rust,ignore
/// let mut split = 0.3;
/// ResizablePanels::horizontal(&mut split)
///     .show(ui, |ui, panel| {
///         match panel {
///             Panel::First => ui.label("Left panel"),
///             Panel::Second => ui.label("Right panel"),
///         }
///     });
/// ```
pub struct ResizablePanels<'a> {
    split: &'a mut f32,
    direction: ResizeDirection,
    min_size: f32,
    handle_width: f32,
    show_handle: bool,
}

/// Panel identifier
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    /// First panel (left or top)
    First,
    /// Second panel (right or bottom)
    Second,
}

impl<'a> ResizablePanels<'a> {
    /// Create horizontal resizable panels (left | right)
    pub fn horizontal(split: &'a mut f32) -> Self {
        Self {
            split,
            direction: ResizeDirection::Horizontal,
            min_size: 50.0,
            handle_width: 8.0,
            show_handle: true,
        }
    }

    /// Create vertical resizable panels (top | bottom)
    pub fn vertical(split: &'a mut f32) -> Self {
        Self {
            split,
            direction: ResizeDirection::Vertical,
            min_size: 50.0,
            handle_width: 8.0,
            show_handle: true,
        }
    }

    /// Set minimum panel size in pixels
    pub fn min_size(mut self, size: f32) -> Self {
        self.min_size = size;
        self
    }

    /// Set handle width
    pub fn handle_width(mut self, width: f32) -> Self {
        self.handle_width = width;
        self
    }

    /// Hide the handle (still draggable but invisible)
    pub fn hide_handle(mut self) -> Self {
        self.show_handle = false;
        self
    }

    /// Show the resizable panels
    pub fn show<R>(
        self,
        ui: &mut Ui,
        mut add_contents: impl FnMut(&mut Ui, Panel) -> R,
    ) -> ResizablePanelsResponse<R> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let available = ui.available_rect_before_wrap();
        let total_size = match self.direction {
            ResizeDirection::Horizontal => available.width(),
            ResizeDirection::Vertical => available.height(),
        };

        // Calculate min/max as fractions
        let min_fraction = self.min_size / total_size;
        let max_fraction = 1.0 - min_fraction;

        // Clamp split
        *self.split = self.split.clamp(min_fraction, max_fraction);

        let mut first_result = None;
        let mut second_result = None;

        match self.direction {
            ResizeDirection::Horizontal => {
                let first_width = (available.width() - self.handle_width) * *self.split;
                let second_width = available.width() - first_width - self.handle_width;

                ui.horizontal(|ui| {
                    // First panel
                    ui.allocate_ui(Vec2::new(first_width, available.height()), |ui| {
                        first_result = Some(add_contents(ui, Panel::First));
                    });

                    // Handle
                    let handle_rect = Rect::from_min_size(
                        Pos2::new(available.min.x + first_width, available.min.y),
                        Vec2::new(self.handle_width, available.height()),
                    );

                    let handle_response =
                        ui.interact(handle_rect, ui.id().with("resize_handle"), Sense::drag());

                    if handle_response.hovered() || handle_response.dragged() {
                        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    }

                    if handle_response.dragged() {
                        let delta = handle_response.drag_delta().x;
                        let new_first_width = first_width + delta;
                        let usable_width = available.width() - self.handle_width;
                        *self.split =
                            (new_first_width / usable_width).clamp(min_fraction, max_fraction);
                    }

                    // Draw handle
                    if self.show_handle {
                        let handle_color = if handle_response.dragged() {
                            tokens.primary
                        } else if handle_response.hovered() {
                            tokens.border
                        } else {
                            tokens.border.gamma_multiply(0.5)
                        };

                        ui.painter().rect_filled(handle_rect, 0.0, handle_color);
                    }

                    ui.allocate_exact_size(Vec2::new(self.handle_width, 0.0), Sense::hover());

                    // Second panel
                    ui.allocate_ui(Vec2::new(second_width, available.height()), |ui| {
                        second_result = Some(add_contents(ui, Panel::Second));
                    });
                });
            }
            ResizeDirection::Vertical => {
                let first_height = (available.height() - self.handle_width) * *self.split;
                let second_height = available.height() - first_height - self.handle_width;

                ui.vertical(|ui| {
                    // First panel
                    ui.allocate_ui(Vec2::new(available.width(), first_height), |ui| {
                        first_result = Some(add_contents(ui, Panel::First));
                    });

                    // Handle
                    let handle_rect = Rect::from_min_size(
                        Pos2::new(available.min.x, available.min.y + first_height),
                        Vec2::new(available.width(), self.handle_width),
                    );

                    let handle_response =
                        ui.interact(handle_rect, ui.id().with("resize_handle"), Sense::drag());

                    if handle_response.hovered() || handle_response.dragged() {
                        ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
                    }

                    if handle_response.dragged() {
                        let delta = handle_response.drag_delta().y;
                        let new_first_height = first_height + delta;
                        let usable_height = available.height() - self.handle_width;
                        *self.split =
                            (new_first_height / usable_height).clamp(min_fraction, max_fraction);
                    }

                    // Draw handle
                    if self.show_handle {
                        let handle_color = if handle_response.dragged() {
                            tokens.primary
                        } else if handle_response.hovered() {
                            tokens.border
                        } else {
                            tokens.border.gamma_multiply(0.5)
                        };

                        ui.painter().rect_filled(handle_rect, 0.0, handle_color);
                    }

                    ui.allocate_exact_size(Vec2::new(0.0, self.handle_width), Sense::hover());

                    // Second panel
                    ui.allocate_ui(Vec2::new(available.width(), second_height), |ui| {
                        second_result = Some(add_contents(ui, Panel::Second));
                    });
                });
            }
        }

        ResizablePanelsResponse {
            first: first_result.unwrap(),
            second: second_result.unwrap(),
            split: *self.split,
        }
    }
}

/// Response from resizable panels
pub struct ResizablePanelsResponse<R> {
    /// First panel result
    pub first: R,
    /// Second panel result
    pub second: R,
    /// Current split position
    pub split: f32,
}

/// Resizable box that can be resized from edges/corners
pub struct ResizableBox {
    id: Id,
    rect: Rect,
    min_size: Vec2,
    max_size: Option<Vec2>,
    edges: ResizeEdges,
}

/// Which edges can be resized
#[derive(Clone, Copy, Debug, Default)]
pub struct ResizeEdges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

impl ResizeEdges {
    /// All edges resizable
    pub fn all() -> Self {
        Self {
            left: true,
            right: true,
            top: true,
            bottom: true,
        }
    }

    /// Only horizontal edges
    pub fn horizontal() -> Self {
        Self {
            left: true,
            right: true,
            top: false,
            bottom: false,
        }
    }

    /// Only vertical edges
    pub fn vertical() -> Self {
        Self {
            left: false,
            right: false,
            top: true,
            bottom: true,
        }
    }

    /// Only right edge
    pub fn right_only() -> Self {
        Self {
            left: false,
            right: true,
            top: false,
            bottom: false,
        }
    }

    /// Only bottom edge
    pub fn bottom_only() -> Self {
        Self {
            left: false,
            right: false,
            top: false,
            bottom: true,
        }
    }
}

impl ResizableBox {
    /// Create a new resizable box
    pub fn new(id: impl std::hash::Hash, rect: Rect) -> Self {
        Self {
            id: Id::new(id),
            rect,
            min_size: Vec2::new(50.0, 50.0),
            max_size: None,
            edges: ResizeEdges::all(),
        }
    }

    /// Set minimum size
    pub fn min_size(mut self, size: Vec2) -> Self {
        self.min_size = size;
        self
    }

    /// Set maximum size
    pub fn max_size(mut self, size: Vec2) -> Self {
        self.max_size = Some(size);
        self
    }

    /// Set which edges are resizable
    pub fn edges(mut self, edges: ResizeEdges) -> Self {
        self.edges = edges;
        self
    }

    /// Show the resizable box
    pub fn show(self, ui: &mut Ui) -> ResizableBoxResponse {
        let handle_size = 6.0;
        let mut new_rect = self.rect;

        // Check each edge
        if self.edges.right {
            let edge_rect = Rect::from_min_size(
                Pos2::new(self.rect.max.x - handle_size / 2.0, self.rect.min.y),
                Vec2::new(handle_size, self.rect.height()),
            );
            let response = ui.interact(edge_rect, self.id.with("right"), Sense::drag());

            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }

            if response.dragged() {
                new_rect.max.x += response.drag_delta().x;
            }
        }

        if self.edges.bottom {
            let edge_rect = Rect::from_min_size(
                Pos2::new(self.rect.min.x, self.rect.max.y - handle_size / 2.0),
                Vec2::new(self.rect.width(), handle_size),
            );
            let response = ui.interact(edge_rect, self.id.with("bottom"), Sense::drag());

            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
            }

            if response.dragged() {
                new_rect.max.y += response.drag_delta().y;
            }
        }

        if self.edges.left {
            let edge_rect = Rect::from_min_size(
                Pos2::new(self.rect.min.x - handle_size / 2.0, self.rect.min.y),
                Vec2::new(handle_size, self.rect.height()),
            );
            let response = ui.interact(edge_rect, self.id.with("left"), Sense::drag());

            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }

            if response.dragged() {
                new_rect.min.x += response.drag_delta().x;
            }
        }

        if self.edges.top {
            let edge_rect = Rect::from_min_size(
                Pos2::new(self.rect.min.x, self.rect.min.y - handle_size / 2.0),
                Vec2::new(self.rect.width(), handle_size),
            );
            let response = ui.interact(edge_rect, self.id.with("top"), Sense::drag());

            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
            }

            if response.dragged() {
                new_rect.min.y += response.drag_delta().y;
            }
        }

        // Enforce min/max size
        let size = new_rect.size();
        let clamped_width = size.x.max(self.min_size.x);
        let clamped_height = size.y.max(self.min_size.y);

        let (clamped_width, clamped_height) = if let Some(max) = self.max_size {
            (clamped_width.min(max.x), clamped_height.min(max.y))
        } else {
            (clamped_width, clamped_height)
        };

        // Adjust rect to maintain position when size changes
        if size.x != clamped_width {
            if self.edges.left {
                new_rect.min.x = new_rect.max.x - clamped_width;
            } else {
                new_rect.max.x = new_rect.min.x + clamped_width;
            }
        }

        if size.y != clamped_height {
            if self.edges.top {
                new_rect.min.y = new_rect.max.y - clamped_height;
            } else {
                new_rect.max.y = new_rect.min.y + clamped_height;
            }
        }

        ResizableBoxResponse { rect: new_rect }
    }
}

/// Response from resizable box
pub struct ResizableBoxResponse {
    /// New rectangle after resize
    pub rect: Rect,
}
