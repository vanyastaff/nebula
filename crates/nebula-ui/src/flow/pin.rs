//! Pin widget and utilities for flow editor.
//!
//! Provides standalone pin rendering and connection point utilities.

use egui::{Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

use crate::theme;

use super::types::{DataType, Pin, PinId, PinKind, ValueType};

/// Style configuration for pin rendering.
#[derive(Debug, Clone)]
pub struct PinStyle {
    /// Radius of the pin circle.
    pub radius: f32,
    /// Hit area radius (larger than visual for easier clicking).
    pub hit_radius: f32,
    /// Border width.
    pub border_width: f32,
    /// Whether to show the glow effect on hover.
    pub show_hover_glow: bool,
}

impl Default for PinStyle {
    fn default() -> Self {
        Self {
            radius: 5.0,
            hit_radius: 10.0,
            border_width: 1.0,
            show_hover_glow: true,
        }
    }
}

/// State of a pin for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PinState {
    /// Normal state.
    #[default]
    Normal,
    /// Pin is hovered.
    Hovered,
    /// Pin is connected.
    Connected,
    /// Pin is being dragged for connection.
    Connecting,
    /// Pin is a valid drop target.
    ValidTarget,
    /// Pin is an invalid drop target.
    InvalidTarget,
}

/// Standalone pin widget for custom layouts.
pub struct PinWidget {
    id: PinId,
    kind: PinKind,
    data_type: DataType,
    value_type: ValueType,
    style: PinStyle,
    state: PinState,
}

impl PinWidget {
    /// Creates a new pin widget.
    pub fn new(id: PinId, kind: PinKind, data_type: DataType) -> Self {
        Self {
            id,
            kind,
            data_type,
            value_type: ValueType::Normal,
            style: PinStyle::default(),
            state: PinState::Normal,
        }
    }

    /// Creates a pin widget from a Pin struct.
    pub fn from_pin(pin: &Pin) -> Self {
        Self {
            id: pin.id,
            kind: pin.id.kind,
            data_type: pin.data_type.clone(),
            value_type: pin.value_type,
            style: PinStyle::default(),
            state: if pin.connected {
                PinState::Connected
            } else {
                PinState::Normal
            },
        }
    }

    /// Sets the value type.
    pub fn value_type(mut self, value_type: ValueType) -> Self {
        self.value_type = value_type;
        self
    }

    /// Sets the style.
    pub fn style(mut self, style: PinStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the state.
    pub fn state(mut self, state: PinState) -> Self {
        self.state = state;
        self
    }

    /// Renders the pin at the given position.
    pub fn show(self, ui: &mut Ui, center: Pos2, zoom: f32) -> PinInteraction {
        let theme = theme::current_theme();

        let radius = self.style.radius * zoom;
        let hit_radius = self.style.hit_radius * zoom;
        let border_width = self.style.border_width * zoom;

        // Allocate hit area
        let hit_rect = Rect::from_center_size(center, Vec2::splat(hit_radius * 2.0));
        let response = ui.allocate_rect(hit_rect, Sense::click_and_drag());

        let painter = ui.painter().clone();

        // Determine colors
        let base_color = theme.color_for_data_type(&self.data_type);
        let (fill_color, border_color, glow) = match self.state {
            PinState::Normal => (base_color, theme.tokens.border, false),
            PinState::Hovered => (base_color, base_color, self.style.show_hover_glow),
            PinState::Connected => (base_color, base_color.gamma_multiply(0.8), false),
            PinState::Connecting => (base_color, theme.tokens.accent, true),
            PinState::ValidTarget => (theme.tokens.success, theme.tokens.success, true),
            PinState::InvalidTarget => (
                theme.tokens.destructive.gamma_multiply(0.5),
                theme.tokens.destructive,
                false,
            ),
        };

        // Draw glow if needed
        if glow || response.hovered() {
            painter.circle_filled(center, radius * 1.5, fill_color.gamma_multiply(0.3));
        }

        // Draw pin based on value type
        self.draw_pin_shape(
            &painter,
            center,
            radius,
            border_width,
            fill_color,
            border_color,
        );

        // Draw connection indicator for connected pins
        if self.state == PinState::Connected {
            let indicator_offset = match self.kind {
                PinKind::Input => Vec2::new(radius * 0.5, 0.0), // Input on right
                PinKind::Output => Vec2::new(-radius * 0.5, 0.0), // Output on left
            };
            painter.circle_filled(center + indicator_offset, 2.0 * zoom, base_color);
        }

        let hovered = response.hovered();
        let clicked = response.clicked();
        let drag_started = response.drag_started();
        let drag_stopped = response.drag_stopped();

        PinInteraction {
            id: self.id,
            kind: self.kind,
            response,
            center,
            hovered,
            clicked,
            drag_started,
            drag_stopped,
        }
    }

    fn draw_pin_shape(
        &self,
        painter: &egui::Painter,
        center: Pos2,
        radius: f32,
        border_width: f32,
        fill: Color32,
        border: Color32,
    ) {
        match self.value_type {
            ValueType::Normal => {
                // Simple filled circle
                painter.circle_filled(center, radius, fill);
                painter.circle_stroke(center, radius, Stroke::new(border_width, border));
            }
            ValueType::Array => {
                // Double circle indicating array
                painter.circle_stroke(center, radius + 3.0, Stroke::new(border_width * 1.5, fill));
                painter.circle_filled(center, radius, fill);
                painter.circle_stroke(center, radius, Stroke::new(border_width, border));
            }
            ValueType::HashMap => {
                // Square shape for maps
                let half = radius * 0.85;
                let rect = Rect::from_center_size(center, Vec2::splat(half * 2.0));
                painter.rect_filled(rect, CornerRadius::same(2), fill);
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(2),
                    Stroke::new(border_width, border),
                    egui::epaint::StrokeKind::Middle,
                );
            }
            ValueType::HashSet => {
                // Diamond shape for sets
                let half = radius * 0.9;
                let points = [
                    center + Vec2::new(0.0, -half),
                    center + Vec2::new(half, 0.0),
                    center + Vec2::new(0.0, half),
                    center + Vec2::new(-half, 0.0),
                ];
                painter.add(egui::Shape::convex_polygon(
                    points.to_vec(),
                    fill,
                    Stroke::new(border_width, border),
                ));
            }
        }
    }
}

/// Result of pin interaction.
#[derive(Debug)]
pub struct PinInteraction {
    /// The pin ID.
    pub id: PinId,
    /// The pin kind.
    pub kind: PinKind,
    /// The egui response.
    pub response: Response,
    /// Center position in screen space.
    pub center: Pos2,
    /// Whether the pin is hovered.
    pub hovered: bool,
    /// Whether the pin was clicked.
    pub clicked: bool,
    /// Whether a drag started on this pin.
    pub drag_started: bool,
    /// Whether a drag stopped on this pin.
    pub drag_stopped: bool,
}

/// Determines if two pins can be connected.
pub fn can_connect(source: &Pin, target: &Pin) -> bool {
    // Can't connect to self
    if source.id == target.id {
        return false;
    }

    // Can't connect same kinds (input to input, output to output)
    if source.id.kind == target.id.kind {
        return false;
    }

    // Check data type compatibility
    source.data_type.is_compatible(&target.data_type)
}

/// Gets the connection point offset for a pin based on its kind.
pub fn connection_offset(kind: PinKind, radius: f32) -> Vec2 {
    match kind {
        PinKind::Input => Vec2::new(radius, 0.0), // Input pins on the right
        PinKind::Output => Vec2::new(-radius, 0.0), // Output pins on the left
    }
}
