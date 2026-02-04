//! Controls panel for flow editor (zoom buttons, fit view, etc.).

use egui::{Pos2, Rect, Response, Ui, Vec2};

use crate::theme;

/// Position of the controls panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlsPosition {
    /// Top left corner.
    TopLeft,
    /// Top right corner.
    TopRight,
    /// Bottom left corner.
    BottomLeft,
    /// Bottom right corner.
    BottomRight,
}

/// Configuration for the controls panel.
#[derive(Debug, Clone)]
pub struct ControlsConfig {
    /// Position on screen.
    pub position: ControlsPosition,
    /// Whether to show zoom in/out buttons.
    pub show_zoom: bool,
    /// Whether to show fit view button.
    pub show_fit_view: bool,
    /// Whether to show zoom to 100% button.
    pub show_zoom_reset: bool,
    /// Whether to show fullscreen button.
    pub show_fullscreen: bool,
    /// Whether to show lock button.
    pub show_lock: bool,
    /// Button size.
    pub button_size: f32,
    /// Spacing between buttons.
    pub spacing: f32,
}

impl Default for ControlsConfig {
    fn default() -> Self {
        Self {
            position: ControlsPosition::BottomLeft,
            show_zoom: true,
            show_fit_view: true,
            show_zoom_reset: true,
            show_fullscreen: false,
            show_lock: false,
            button_size: 32.0,
            spacing: 4.0,
        }
    }
}

/// Actions that can be triggered by controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    /// Zoom in.
    ZoomIn,
    /// Zoom out.
    ZoomOut,
    /// Reset zoom to 100%.
    ZoomReset,
    /// Fit view to all nodes.
    FitView,
    /// Toggle fullscreen.
    ToggleFullscreen,
    /// Toggle interaction lock.
    ToggleLock,
}

/// Controls panel widget.
pub struct Controls {
    config: ControlsConfig,
    is_locked: bool,
}

impl Controls {
    /// Creates a new controls panel.
    pub fn new() -> Self {
        Self {
            config: ControlsConfig::default(),
            is_locked: false,
        }
    }

    /// Sets the configuration.
    pub fn config(mut self, config: ControlsConfig) -> Self {
        self.config = config;
        self
    }

    /// Sets the lock state.
    pub fn locked(mut self, locked: bool) -> Self {
        self.is_locked = locked;
        self
    }

    /// Shows the controls panel.
    pub fn show(self, ui: &mut Ui) -> ControlsResponse {
        let theme = theme::current_theme();
        let tokens = &theme.tokens;

        // Calculate panel position
        let parent_rect = ui.max_rect();
        let margin = 10.0;

        // Calculate panel size
        let button_count = [
            self.config.show_zoom.then_some(2), // zoom in + out
            self.config.show_zoom_reset.then_some(1),
            self.config.show_fit_view.then_some(1),
            self.config.show_fullscreen.then_some(1),
            self.config.show_lock.then_some(1),
        ]
        .into_iter()
        .flatten()
        .sum::<usize>();

        let panel_width = self.config.button_size + self.config.spacing * 2.0;
        let panel_height = button_count as f32 * (self.config.button_size + self.config.spacing)
            + self.config.spacing;

        let panel_size = Vec2::new(panel_width, panel_height);

        let panel_pos = match self.config.position {
            ControlsPosition::TopLeft => parent_rect.min + Vec2::splat(margin),
            ControlsPosition::TopRight => Pos2::new(
                parent_rect.max.x - panel_width - margin,
                parent_rect.min.y + margin,
            ),
            ControlsPosition::BottomLeft => Pos2::new(
                parent_rect.min.x + margin,
                parent_rect.max.y - panel_height - margin,
            ),
            ControlsPosition::BottomRight => parent_rect.max - panel_size - Vec2::splat(margin),
        };

        let panel_rect = Rect::from_min_size(panel_pos, panel_size);

        // Draw background
        let painter = ui.painter();
        painter.rect_filled(panel_rect, 6.0, tokens.background);
        painter.rect_stroke(
            panel_rect,
            6.0,
            egui::Stroke::new(1.0, tokens.border),
            egui::epaint::StrokeKind::Middle,
        );

        // Draw buttons in a vertical layout
        let mut cursor_y = panel_pos.y + self.config.spacing;
        let button_x = panel_pos.x + self.config.spacing;

        let mut actions = Vec::new();

        // Helper to draw a button
        let mut draw_button =
            |ui: &mut Ui, y: &mut f32, icon: &str, tooltip: &str, action: ControlAction| {
                let button_rect = Rect::from_min_size(
                    Pos2::new(button_x, *y),
                    Vec2::splat(self.config.button_size),
                );

                ui.allocate_ui_at_rect(button_rect, |ui| {
                    let button = egui::Button::new(icon)
                        .min_size(Vec2::splat(self.config.button_size))
                        .frame(true);

                    let response = ui.add(button).on_hover_text(tooltip);

                    if response.clicked() {
                        actions.push(action);
                    }
                });

                *y += self.config.button_size + self.config.spacing;
            };

        // Zoom buttons
        if self.config.show_zoom {
            draw_button(ui, &mut cursor_y, "+", "Zoom In", ControlAction::ZoomIn);
            draw_button(ui, &mut cursor_y, "−", "Zoom Out", ControlAction::ZoomOut);
        }

        // Zoom reset
        if self.config.show_zoom_reset {
            draw_button(
                ui,
                &mut cursor_y,
                "1:1",
                "Reset Zoom",
                ControlAction::ZoomReset,
            );
        }

        // Fit view
        if self.config.show_fit_view {
            draw_button(ui, &mut cursor_y, "⛶", "Fit View", ControlAction::FitView);
        }

        // Lock
        if self.config.show_lock {
            let icon = if self.is_locked { "🔒" } else { "🔓" };
            let tooltip = if self.is_locked {
                "Unlock (enable editing)"
            } else {
                "Lock (disable editing)"
            };
            draw_button(ui, &mut cursor_y, icon, tooltip, ControlAction::ToggleLock);
        }

        // Fullscreen
        if self.config.show_fullscreen {
            draw_button(
                ui,
                &mut cursor_y,
                "⛶",
                "Toggle Fullscreen",
                ControlAction::ToggleFullscreen,
            );
        }

        ControlsResponse { actions }
    }
}

impl Default for Controls {
    fn default() -> Self {
        Self::new()
    }
}

/// Response from showing the controls panel.
#[derive(Debug)]
pub struct ControlsResponse {
    /// Actions triggered by button clicks.
    pub actions: Vec<ControlAction>,
}

impl ControlsResponse {
    /// Returns true if any action was triggered.
    pub fn has_actions(&self) -> bool {
        !self.actions.is_empty()
    }

    /// Returns the first action, if any.
    pub fn action(&self) -> Option<ControlAction> {
        self.actions.first().copied()
    }
}
