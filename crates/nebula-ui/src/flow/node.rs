//! Node widget for flow editor.
//!
//! Renders individual nodes with header, pins, and content area.

use egui::{Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

use crate::theme;

use super::types::{Node, NodeId, Pin, PinKind};

/// Visual state of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodeVisualState {
    /// Normal state.
    #[default]
    Normal,
    /// Node is hovered.
    Hovered,
    /// Node is selected.
    Selected,
    /// Node is being dragged.
    Dragging,
    /// Node is executing (for runtime visualization).
    Executing,
    /// Node has an error.
    Error,
}

/// Style configuration for node rendering.
#[derive(Debug, Clone)]
pub struct NodeStyle {
    /// Minimum width of the node.
    pub min_width: f32,
    /// Maximum width of the node.
    pub max_width: f32,
    /// Header height.
    pub header_height: f32,
    /// Pin row height.
    pub pin_row_height: f32,
    /// Padding inside the node.
    pub padding: f32,
    /// Corner rounding.
    pub rounding: f32,
    /// Border width for normal state.
    pub border_width: f32,
    /// Border width for selected state.
    pub selected_border_width: f32,
    /// Shadow offset.
    pub shadow_offset: Vec2,
    /// Shadow blur radius.
    pub shadow_blur: f32,
}

impl Default for NodeStyle {
    fn default() -> Self {
        Self {
            min_width: 180.0,
            max_width: 300.0,
            header_height: 28.0,
            pin_row_height: 24.0,
            padding: 8.0,
            rounding: 6.0,
            border_width: 1.0,
            selected_border_width: 2.0,
            shadow_offset: Vec2::new(2.0, 4.0),
            shadow_blur: 8.0,
        }
    }
}

/// Result of rendering a node.
#[derive(Debug)]
pub struct NodeResponse {
    /// The egui response for the node.
    pub response: Response,
    /// The node's bounding rectangle in canvas space.
    pub rect: Rect,
    /// The node's bounding rectangle in screen space.
    pub screen_rect: Rect,
    /// Pin responses with their positions.
    pub pin_responses: Vec<PinResponse>,
    /// Whether the header was clicked.
    pub header_clicked: bool,
    /// Whether the node should be deleted (close button clicked).
    pub delete_requested: bool,
    /// Whether the node is being dragged.
    pub is_dragged: bool,
    /// Drag delta in screen space.
    pub drag_delta: Vec2,
}

/// Response for a single pin.
#[derive(Debug)]
pub struct PinResponse {
    /// The pin ID.
    pub pin_id: super::types::PinId,
    /// The pin kind.
    pub kind: PinKind,
    /// The pin's center position in screen space.
    pub center: Pos2,
    /// Whether the pin is hovered.
    pub hovered: bool,
    /// Whether the pin was clicked.
    pub clicked: bool,
}

/// Widget for rendering a node.
pub struct NodeWidget<'a> {
    node: &'a Node,
    input_pins: &'a [Pin],
    output_pins: &'a [Pin],
    style: NodeStyle,
    visual_state: NodeVisualState,
    show_close_button: bool,
}

impl<'a> NodeWidget<'a> {
    /// Creates a new node widget.
    pub fn new(node: &'a Node, input_pins: &'a [Pin], output_pins: &'a [Pin]) -> Self {
        Self {
            node,
            input_pins,
            output_pins,
            style: NodeStyle::default(),
            visual_state: NodeVisualState::Normal,
            show_close_button: true,
        }
    }

    /// Sets the node style.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Sets the visual state.
    pub fn visual_state(mut self, state: NodeVisualState) -> Self {
        self.visual_state = state;
        self
    }

    /// Sets whether to show the close button.
    pub fn show_close_button(mut self, show: bool) -> Self {
        self.show_close_button = show;
        self
    }

    /// Renders the node and returns the response.
    pub fn show(self, ui: &mut Ui, viewport: Rect, pan: Vec2, zoom: f32) -> NodeResponse {
        let theme = theme::current_theme();
        let tokens = &theme.tokens;

        // Calculate screen position using proper canvas_to_screen formula
        let screen_pos = Pos2::new(
            self.node.position.x * zoom + pan.x + viewport.min.x,
            self.node.position.y * zoom + pan.y + viewport.min.y,
        );

        // Calculate node size
        let pin_count = self.input_pins.len().max(self.output_pins.len());
        let content_height = pin_count as f32 * self.style.pin_row_height * zoom;
        let total_height =
            (self.style.header_height + self.style.padding * 2.0) * zoom + content_height;
        let width = self.style.min_width * zoom;

        let node_rect = Rect::from_min_size(screen_pos, Vec2::new(width, total_height));

        // Use ui.interact() for proper egui integration
        // Create unique ID from node ID to avoid conflicts
        let response = ui.interact(
            node_rect,
            egui::Id::new(("node", self.node.id)),
            Sense::click_and_drag(),
        );

        let is_hovered = response.hovered();
        let is_clicked = response.clicked();
        let is_dragged = response.dragged();
        let drag_delta = response.drag_delta();

        let painter = ui.painter().clone();

        // Get colors based on state and category
        let category_color = theme.color_for_category(&self.node.category);
        let (bg_color, border_color, border_width) = match self.visual_state {
            NodeVisualState::Normal => (tokens.card, tokens.border, self.style.border_width),
            NodeVisualState::Hovered => (tokens.secondary, category_color, self.style.border_width),
            NodeVisualState::Selected => (
                tokens.card,
                category_color,
                self.style.selected_border_width,
            ),
            NodeVisualState::Dragging => (
                tokens.secondary,
                category_color,
                self.style.selected_border_width,
            ),
            NodeVisualState::Executing => (
                tokens.card,
                tokens.warning,
                self.style.selected_border_width,
            ),
            NodeVisualState::Error => (
                tokens.card,
                tokens.destructive,
                self.style.selected_border_width,
            ),
        };

        let rounding = (self.style.rounding * zoom) as u8;

        // Draw shadow
        if self.visual_state == NodeVisualState::Selected
            || self.visual_state == NodeVisualState::Dragging
        {
            let shadow_rect = node_rect.translate(self.style.shadow_offset * zoom);
            painter.rect_filled(
                shadow_rect,
                CornerRadius::same(rounding),
                Color32::from_black_alpha(40),
            );
        }

        // Draw node background
        painter.rect_filled(node_rect, CornerRadius::same(rounding), bg_color);
        painter.rect_stroke(
            node_rect,
            CornerRadius::same(rounding),
            Stroke::new(border_width * zoom, border_color),
            egui::epaint::StrokeKind::Middle,
        );

        // Draw header
        let header_rect = Rect::from_min_size(
            screen_pos,
            Vec2::new(width, self.style.header_height * zoom),
        );
        painter.rect_filled(
            header_rect,
            CornerRadius {
                nw: rounding,
                ne: rounding,
                sw: 0,
                se: 0,
            },
            category_color,
        );

        // Draw header text
        let text_pos = header_rect.left_center() + Vec2::new(self.style.padding * zoom, 0.0);
        painter.text(
            text_pos,
            egui::Align2::LEFT_CENTER,
            &self.node.name,
            egui::FontId::proportional(12.0 * zoom),
            tokens.primary_foreground,
        );

        // Draw close button if enabled
        let mut delete_requested = false;
        if self.show_close_button {
            let close_btn_center =
                header_rect.right_center() - Vec2::new(self.style.padding * zoom + 6.0 * zoom, 0.0);
            let close_btn_rect = Rect::from_center_size(close_btn_center, Vec2::splat(12.0 * zoom));
            let close_response = ui.allocate_rect(close_btn_rect, Sense::click());

            if close_response.hovered() {
                painter.circle_filled(close_btn_center, 6.0 * zoom, Color32::from_white_alpha(30));
            }
            painter.text(
                close_btn_center,
                egui::Align2::CENTER_CENTER,
                "×",
                egui::FontId::proportional(14.0 * zoom),
                tokens.primary_foreground,
            );

            if close_response.clicked() {
                delete_requested = true;
            }
        }

        // Draw pins
        let mut pin_responses = Vec::new();
        let content_top =
            screen_pos.y + self.style.header_height * zoom + self.style.padding * zoom;

        // Input pins (left side)
        for (i, pin) in self.input_pins.iter().enumerate() {
            let pin_y = content_top + (i as f32 + 0.5) * self.style.pin_row_height * zoom;
            let pin_center = Pos2::new(screen_pos.x, pin_y);

            let pin_response = self.draw_pin(ui, pin, pin_center, zoom, &theme);
            pin_responses.push(pin_response);

            // Draw pin label
            let label_pos = pin_center + Vec2::new(self.style.padding * zoom + 6.0 * zoom, 0.0);
            painter.text(
                label_pos,
                egui::Align2::LEFT_CENTER,
                &pin.name,
                egui::FontId::proportional(11.0 * zoom),
                tokens.foreground,
            );
        }

        // Output pins (right side)
        for (i, pin) in self.output_pins.iter().enumerate() {
            let pin_y = content_top + (i as f32 + 0.5) * self.style.pin_row_height * zoom;
            let pin_center = Pos2::new(screen_pos.x + width, pin_y);

            let pin_response = self.draw_pin(ui, pin, pin_center, zoom, &theme);
            pin_responses.push(pin_response);

            // Draw pin label
            let label_pos = pin_center - Vec2::new(self.style.padding * zoom + 6.0 * zoom, 0.0);
            painter.text(
                label_pos,
                egui::Align2::RIGHT_CENTER,
                &pin.name,
                egui::FontId::proportional(11.0 * zoom),
                tokens.foreground,
            );
        }

        // Calculate canvas-space rect
        let canvas_rect = Rect::from_min_size(
            self.node.position,
            Vec2::new(self.style.min_width, total_height / zoom),
        );

        let header_clicked = is_clicked;

        NodeResponse {
            response,
            rect: canvas_rect,
            screen_rect: node_rect,
            pin_responses,
            header_clicked,
            delete_requested,
            is_dragged,
            drag_delta,
        }
    }

    fn draw_pin(
        &self,
        ui: &mut Ui,
        pin: &Pin,
        center: Pos2,
        zoom: f32,
        theme: &theme::Theme,
    ) -> PinResponse {
        let pin_radius = 5.0 * zoom;
        let hit_radius = 12.0 * zoom; // Larger hit area for easier clicking

        let hit_rect = Rect::from_center_size(center, Vec2::splat(hit_radius * 2.0));
        let pin_response = ui.interact(hit_rect, egui::Id::new(("pin", pin.id)), Sense::click());

        let painter = ui.painter().clone();
        let pin_color = theme.color_for_data_type(&pin.data_type);

        // Draw pin based on value type
        match pin.value_type {
            super::types::ValueType::Normal => {
                // Filled circle for normal values
                if pin_response.hovered() {
                    painter.circle_filled(center, pin_radius * 1.3, pin_color.gamma_multiply(0.3));
                }
                painter.circle_filled(center, pin_radius, pin_color);
                painter.circle_stroke(
                    center,
                    pin_radius,
                    Stroke::new(1.0 * zoom, theme.tokens.border),
                );
            }
            super::types::ValueType::Array => {
                // Double circle for arrays
                painter.circle_stroke(
                    center,
                    pin_radius + 2.0 * zoom,
                    Stroke::new(1.5 * zoom, pin_color),
                );
                painter.circle_filled(center, pin_radius, pin_color);
            }
            super::types::ValueType::HashMap | super::types::ValueType::HashSet => {
                // Square for maps/sets
                let half_size = pin_radius * 0.8;
                let rect = Rect::from_center_size(center, Vec2::splat(half_size * 2.0));
                let pin_rounding = (2.0 * zoom) as u8;
                painter.rect_filled(rect, CornerRadius::same(pin_rounding), pin_color);
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(pin_rounding),
                    Stroke::new(1.0 * zoom, theme.tokens.border),
                    egui::epaint::StrokeKind::Middle,
                );
            }
        }

        PinResponse {
            pin_id: pin.id,
            kind: pin.id.kind,
            center,
            hovered: pin_response.hovered(),
            clicked: pin_response.clicked(),
        }
    }
}

/// Gets the node ID.
pub fn node_id(node: &Node) -> NodeId {
    node.id
}
