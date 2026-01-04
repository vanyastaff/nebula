//! MiniMap component for flow editor navigation.
//!
//! Provides a bird's-eye view of the entire flow graph with viewport indicator.

use egui::{Color32, Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::theme;

use super::types::{Connection, Node, NodeId};

/// Configuration for the minimap.
#[derive(Debug, Clone)]
pub struct MinimapConfig {
    /// Width of the minimap panel.
    pub width: f32,
    /// Height of the minimap panel.
    pub height: f32,
    /// Position on screen (relative to parent).
    pub position: MinimapPosition,
    /// Background opacity (0.0 - 1.0).
    pub background_opacity: f32,
    /// Whether nodes show their category color.
    pub colored_nodes: bool,
    /// Node scale factor in minimap.
    pub node_scale: f32,
    /// Padding around content.
    pub padding: f32,
}

impl Default for MinimapConfig {
    fn default() -> Self {
        Self {
            width: 200.0,
            height: 150.0,
            position: MinimapPosition::BottomRight,
            background_opacity: 0.9,
            colored_nodes: true,
            node_scale: 0.1,
            padding: 20.0,
        }
    }
}

/// Position of the minimap on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinimapPosition {
    /// Top left corner.
    TopLeft,
    /// Top right corner.
    TopRight,
    /// Bottom left corner.
    BottomLeft,
    /// Bottom right corner.
    BottomRight,
}

/// MiniMap widget for flow graph navigation.
pub struct Minimap<'a> {
    nodes: &'a [Node],
    connections: &'a [Connection],
    config: MinimapConfig,
    viewport_rect: Rect,
    canvas_pan: Vec2,
    canvas_zoom: f32,
}

impl<'a> Minimap<'a> {
    /// Creates a new minimap.
    pub fn new(
        nodes: &'a [Node],
        connections: &'a [Connection],
        viewport_rect: Rect,
        canvas_pan: Vec2,
        canvas_zoom: f32,
    ) -> Self {
        Self {
            nodes,
            connections,
            config: MinimapConfig::default(),
            viewport_rect,
            canvas_pan,
            canvas_zoom,
        }
    }

    /// Sets the minimap configuration.
    pub fn config(mut self, config: MinimapConfig) -> Self {
        self.config = config;
        self
    }

    /// Shows the minimap and returns interaction response.
    pub fn show(self, ui: &mut Ui) -> MinimapResponse {
        let theme = theme::current_theme();
        let tokens = &theme.tokens;

        // Calculate minimap position
        let parent_rect = ui.max_rect();
        let margin = 10.0;
        let minimap_size = Vec2::new(self.config.width, self.config.height);

        let minimap_pos = match self.config.position {
            MinimapPosition::TopLeft => parent_rect.min + Vec2::splat(margin),
            MinimapPosition::TopRight => Pos2::new(
                parent_rect.max.x - minimap_size.x - margin,
                parent_rect.min.y + margin,
            ),
            MinimapPosition::BottomLeft => Pos2::new(
                parent_rect.min.x + margin,
                parent_rect.max.y - minimap_size.y - margin,
            ),
            MinimapPosition::BottomRight => parent_rect.max - minimap_size - Vec2::splat(margin),
        };

        let minimap_rect = Rect::from_min_size(minimap_pos, minimap_size);

        // Allocate space and get interaction
        let response = ui.allocate_rect(minimap_rect, Sense::click_and_drag());

        let painter = ui.painter();

        // Draw background with opacity
        let bg_color = Color32::from_rgba_unmultiplied(
            tokens.background.r(),
            tokens.background.g(),
            tokens.background.b(),
            (255.0 * self.config.background_opacity) as u8,
        );
        painter.rect_filled(minimap_rect, 4.0, bg_color);
        painter.rect_stroke(
            minimap_rect,
            4.0,
            egui::Stroke::new(1.0, tokens.border),
            egui::StrokeKind::Outside,
        );

        // Calculate bounds of all nodes
        let content_bounds = self.calculate_content_bounds();

        if content_bounds.is_positive() {
            // Calculate transform to fit content in minimap
            let content_size = content_bounds.size();
            let available_size = minimap_size - Vec2::splat(self.config.padding * 2.0);

            let scale_x = available_size.x / content_size.x;
            let scale_y = available_size.y / content_size.y;
            let scale = scale_x.min(scale_y);

            let offset = minimap_rect.min.to_vec2()
                + Vec2::splat(self.config.padding)
                + (available_size - content_size * scale) / 2.0;

            // Transform function: canvas coords -> minimap screen coords
            let to_minimap = |canvas_pos: Pos2| -> Pos2 {
                let relative = canvas_pos - content_bounds.min.to_vec2();
                Pos2::new(offset.x + relative.x * scale, offset.y + relative.y * scale)
            };

            // Draw connections (as thin lines)
            for connection in self.connections {
                if let (Some(from_node), Some(to_node)) = (
                    self.nodes
                        .iter()
                        .find(|n| n.outputs.iter().any(|p| p.id == connection.source)),
                    self.nodes
                        .iter()
                        .find(|n| n.inputs.iter().any(|p| p.id == connection.target)),
                ) {
                    let from = to_minimap(from_node.position);
                    let to = to_minimap(to_node.position);

                    let conn_color = if self.config.colored_nodes {
                        theme
                            .color_for_data_type(&connection.data_type)
                            .gamma_multiply(0.5)
                    } else {
                        tokens.border
                    };

                    painter.line_segment([from, to], egui::Stroke::new(1.0, conn_color));
                }
            }

            // Draw nodes (as small rectangles)
            for node in self.nodes {
                let node_pos = to_minimap(node.position);
                let node_size = node.size * scale * self.config.node_scale;
                let node_rect = Rect::from_min_size(node_pos, node_size);

                let node_color = if self.config.colored_nodes {
                    theme.color_for_category(&node.category)
                } else {
                    tokens.accent
                };

                painter.rect_filled(node_rect, 1.0, node_color);
            }

            // Calculate and draw viewport indicator
            let viewport_canvas_rect = self.calculate_viewport_in_canvas();
            let viewport_min = to_minimap(viewport_canvas_rect.min);
            let viewport_max = to_minimap(viewport_canvas_rect.max);
            let viewport_minimap_rect = Rect::from_two_pos(viewport_min, viewport_max);

            // Draw viewport outline
            painter.rect_stroke(
                viewport_minimap_rect,
                2.0,
                egui::Stroke::new(2.0, tokens.accent),
                egui::StrokeKind::Outside,
            );

            // Draw semi-transparent viewport fill
            painter.rect_filled(
                viewport_minimap_rect,
                2.0,
                tokens.accent.gamma_multiply(0.1),
            );

            // Handle interaction - clicking or dragging to navigate
            let mut clicked_position = None;

            if response.clicked() || response.dragged() {
                if let Some(pointer_pos) = response.interact_pointer_pos() {
                    // Convert minimap click position back to canvas position
                    let relative = pointer_pos - offset.to_pos2();
                    let canvas_pos =
                        content_bounds.min + Vec2::new(relative.x / scale, relative.y / scale);

                    clicked_position = Some(canvas_pos);
                }
            }

            let hovered = response.hovered();

            MinimapResponse {
                response,
                clicked_position,
                hovered,
            }
        } else {
            // No nodes to display
            painter.text(
                minimap_rect.center(),
                egui::Align2::CENTER_CENTER,
                "No nodes",
                egui::FontId::proportional(12.0),
                tokens.muted_foreground,
            );

            MinimapResponse {
                response,
                clicked_position: None,
                hovered: false,
            }
        }
    }

    /// Calculate bounding box of all nodes in canvas space.
    fn calculate_content_bounds(&self) -> Rect {
        if self.nodes.is_empty() {
            return Rect::NOTHING;
        }

        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;

        for node in self.nodes {
            min_x = min_x.min(node.position.x);
            max_x = max_x.max(node.position.x + node.size.x);
            min_y = min_y.min(node.position.y);
            max_y = max_y.max(node.position.y + node.size.y);
        }

        Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y))
    }

    /// Calculate what portion of the canvas is currently visible.
    fn calculate_viewport_in_canvas(&self) -> Rect {
        // Convert viewport screen rect to canvas coordinates
        let top_left = self.screen_to_canvas(self.viewport_rect.min);
        let bottom_right = self.screen_to_canvas(self.viewport_rect.max);

        Rect::from_two_pos(top_left, bottom_right)
    }

    fn screen_to_canvas(&self, screen_pos: Pos2) -> Pos2 {
        let relative = screen_pos - self.viewport_rect.min.to_vec2();
        Pos2::new(
            (relative.x - self.canvas_pan.x) / self.canvas_zoom,
            (relative.y - self.canvas_pan.y) / self.canvas_zoom,
        )
    }
}

/// Response from showing a minimap.
#[derive(Debug)]
pub struct MinimapResponse {
    /// The egui response for the minimap.
    pub response: Response,
    /// Canvas position that was clicked (for navigation).
    pub clicked_position: Option<Pos2>,
    /// Whether the minimap is hovered.
    pub hovered: bool,
}

impl MinimapResponse {
    /// Returns true if the minimap was clicked or dragged.
    pub fn navigated(&self) -> bool {
        self.clicked_position.is_some()
    }
}
