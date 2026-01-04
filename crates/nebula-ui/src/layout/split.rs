//! Split view component for resizable panes.

use crate::theme::current_theme;
use egui::{Ui, Vec2};

/// Split direction
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SplitDirection {
    /// Horizontal split (left/right panes)
    #[default]
    Horizontal,
    /// Vertical split (top/bottom panes)
    Vertical,
}

/// A resizable split view
pub struct SplitView<'a> {
    direction: SplitDirection,
    ratio: &'a mut f32,
    min_ratio: f32,
    max_ratio: f32,
    handle_width: f32,
    show_handle: bool,
}

impl<'a> SplitView<'a> {
    /// Create a new split view
    pub fn new(ratio: &'a mut f32) -> Self {
        Self {
            direction: SplitDirection::Horizontal,
            ratio,
            min_ratio: 0.1,
            max_ratio: 0.9,
            handle_width: 6.0,
            show_handle: true,
        }
    }

    /// Set direction
    pub fn direction(mut self, direction: SplitDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Horizontal split
    pub fn horizontal(mut self) -> Self {
        self.direction = SplitDirection::Horizontal;
        self
    }

    /// Vertical split
    pub fn vertical(mut self) -> Self {
        self.direction = SplitDirection::Vertical;
        self
    }

    /// Set minimum ratio
    pub fn min_ratio(mut self, ratio: f32) -> Self {
        self.min_ratio = ratio;
        self
    }

    /// Set maximum ratio
    pub fn max_ratio(mut self, ratio: f32) -> Self {
        self.max_ratio = ratio;
        self
    }

    /// Set handle width
    pub fn handle_width(mut self, width: f32) -> Self {
        self.handle_width = width;
        self
    }

    /// Hide handle (still resizable)
    pub fn hide_handle(mut self) -> Self {
        self.show_handle = false;
        self
    }

    /// Show the split view
    pub fn show<R1, R2>(
        self,
        ui: &mut Ui,
        first: impl FnOnce(&mut Ui) -> R1,
        second: impl FnOnce(&mut Ui) -> R2,
    ) -> SplitResponse<R1, R2> {
        let theme = current_theme();
        let tokens = &theme.tokens;

        let available = ui.available_size();
        let handle_width = self.handle_width;

        // Calculate sizes
        let (first_size, second_size, handle_rect) = match self.direction {
            SplitDirection::Horizontal => {
                let first_width = (available.x - handle_width) * *self.ratio;
                let second_width = available.x - first_width - handle_width;

                let first_size = Vec2::new(first_width, available.y);
                let second_size = Vec2::new(second_width, available.y);

                let handle_rect = egui::Rect::from_min_size(
                    ui.cursor().min + Vec2::new(first_width, 0.0),
                    Vec2::new(handle_width, available.y),
                );

                (first_size, second_size, handle_rect)
            }
            SplitDirection::Vertical => {
                let first_height = (available.y - handle_width) * *self.ratio;
                let second_height = available.y - first_height - handle_width;

                let first_size = Vec2::new(available.x, first_height);
                let second_size = Vec2::new(available.x, second_height);

                let handle_rect = egui::Rect::from_min_size(
                    ui.cursor().min + Vec2::new(0.0, first_height),
                    Vec2::new(available.x, handle_width),
                );

                (first_size, second_size, handle_rect)
            }
        };

        // Layout
        let mut first_response = None;
        let mut second_response = None;

        match self.direction {
            SplitDirection::Horizontal => {
                ui.horizontal(|ui| {
                    // First pane
                    ui.allocate_ui(first_size, |ui| {
                        first_response = Some(first(ui));
                    });

                    // Handle
                    let handle_response = ui.allocate_rect(handle_rect, egui::Sense::drag());

                    if self.show_handle {
                        let handle_color = if handle_response.dragged() || handle_response.hovered()
                        {
                            tokens.primary
                        } else {
                            tokens.border
                        };

                        ui.painter().rect_filled(
                            handle_rect.shrink2(Vec2::new(2.0, 0.0)),
                            2.0,
                            handle_color,
                        );
                    }

                    if handle_response.dragged() {
                        let delta = handle_response.drag_delta().x;
                        let total = available.x - handle_width;
                        *self.ratio =
                            (*self.ratio + delta / total).clamp(self.min_ratio, self.max_ratio);
                    }

                    if handle_response.hovered() || handle_response.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }

                    // Second pane
                    ui.allocate_ui(second_size, |ui| {
                        second_response = Some(second(ui));
                    });
                });
            }
            SplitDirection::Vertical => {
                ui.vertical(|ui| {
                    // First pane
                    ui.allocate_ui(first_size, |ui| {
                        first_response = Some(first(ui));
                    });

                    // Handle
                    let handle_response = ui.allocate_rect(handle_rect, egui::Sense::drag());

                    if self.show_handle {
                        let handle_color = if handle_response.dragged() || handle_response.hovered()
                        {
                            tokens.primary
                        } else {
                            tokens.border
                        };

                        ui.painter().rect_filled(
                            handle_rect.shrink2(Vec2::new(0.0, 2.0)),
                            2.0,
                            handle_color,
                        );
                    }

                    if handle_response.dragged() {
                        let delta = handle_response.drag_delta().y;
                        let total = available.y - handle_width;
                        *self.ratio =
                            (*self.ratio + delta / total).clamp(self.min_ratio, self.max_ratio);
                    }

                    if handle_response.hovered() || handle_response.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                    }

                    // Second pane
                    ui.allocate_ui(second_size, |ui| {
                        second_response = Some(second(ui));
                    });
                });
            }
        }

        SplitResponse {
            first: first_response.unwrap(),
            second: second_response.unwrap(),
        }
    }
}

/// Response from a split view
pub struct SplitResponse<R1, R2> {
    /// First pane's return value
    pub first: R1,
    /// Second pane's return value
    pub second: R2,
}

/// Triple split view (three panes)
pub struct TripleSplit<'a> {
    direction: SplitDirection,
    ratio1: &'a mut f32,
    ratio2: &'a mut f32,
}

impl<'a> TripleSplit<'a> {
    /// Create a new triple split
    pub fn new(ratio1: &'a mut f32, ratio2: &'a mut f32) -> Self {
        Self {
            direction: SplitDirection::Horizontal,
            ratio1,
            ratio2,
        }
    }

    /// Set direction
    pub fn direction(mut self, direction: SplitDirection) -> Self {
        self.direction = direction;
        self
    }

    /// Show the triple split
    pub fn show<R1, R2, R3>(
        self,
        ui: &mut Ui,
        first: impl FnOnce(&mut Ui) -> R1,
        second: impl FnOnce(&mut Ui) -> R2,
        third: impl FnOnce(&mut Ui) -> R3,
    ) -> (R1, R2, R3) {
        // Use nested splits
        let mut middle_ratio = *self.ratio2 / (1.0 - *self.ratio1);

        let split = SplitView::new(self.ratio1).direction(self.direction);

        let result = split.show(ui, first, |ui| {
            let inner_split = SplitView::new(&mut middle_ratio).direction(self.direction);

            inner_split.show(ui, second, third)
        });

        // Update ratio2 based on middle_ratio
        *self.ratio2 = middle_ratio * (1.0 - *self.ratio1);

        (result.first, result.second.first, result.second.second)
    }
}
