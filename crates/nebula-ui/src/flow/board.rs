//! Board editor - the main flow graph editor widget.
//!
//! Combines canvas, nodes, connections, and interactions into a complete editor.

use std::collections::HashMap;

use egui::{CentralPanel, Context, Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::theme;

use super::{
    canvas::{Canvas, CanvasState},
    connection::{ConnectionRenderer, ConnectionState, ConnectionStyle, point_near_connection},
    node::{NodeStyle, NodeVisualState, NodeWidget},
    selection::{SelectionState, move_selection},
    types::{Connection, ConnectionId, DataType, Node, NodeId, Pin, PinId, PinKind},
};

/// State for a connection being created.
#[derive(Debug, Clone)]
pub struct PendingConnection {
    /// Source node ID.
    pub from_node: NodeId,
    /// Source pin ID.
    pub from_pin: PinId,
    /// Source pin kind.
    pub from_kind: PinKind,
    /// Source pin data type.
    pub data_type: DataType,
    /// Current cursor position.
    pub to_pos: Pos2,
}

/// Events emitted by the board editor.
#[derive(Debug, Clone)]
pub enum BoardEvent {
    /// A node was selected.
    NodeSelected(NodeId),
    /// A node was deselected.
    NodeDeselected(NodeId),
    /// A node was moved.
    NodeMoved { id: NodeId, position: Pos2 },
    /// A node deletion was requested.
    NodeDeleteRequested(NodeId),
    /// A connection was created.
    ConnectionCreated {
        from_node: NodeId,
        from_pin: PinId,
        to_node: NodeId,
        to_pin: PinId,
    },
    /// A connection was deleted.
    ConnectionDeleted(ConnectionId),
    /// Canvas was panned.
    CanvasPanned(Vec2),
    /// Canvas was zoomed.
    CanvasZoomed(f32),
    /// Empty space was clicked.
    CanvasClicked(Pos2),
    /// Empty space was double-clicked.
    CanvasDoubleClicked(Pos2),
    /// Context menu was requested.
    ContextMenuRequested(Pos2),
}

/// Configuration for the board editor.
#[derive(Debug, Clone)]
pub struct BoardConfig {
    /// Node rendering style.
    pub node_style: NodeStyle,
    /// Connection rendering style.
    pub connection_style: ConnectionStyle,
    /// Whether to show grid.
    pub show_grid: bool,
    /// Whether to enable snapping to grid.
    pub snap_to_grid: bool,
    /// Grid snap size.
    pub grid_snap_size: f32,
    /// Whether to show minimap.
    pub show_minimap: bool,
    /// Connection hit threshold (for selection).
    pub connection_hit_threshold: f32,
}

impl Default for BoardConfig {
    fn default() -> Self {
        Self {
            node_style: NodeStyle::default(),
            connection_style: ConnectionStyle::default(),
            show_grid: true,
            snap_to_grid: true,
            grid_snap_size: 20.0,
            show_minimap: false,
            connection_hit_threshold: 8.0,
        }
    }
}

/// The main board editor state.
pub struct BoardState {
    /// Canvas pan/zoom state.
    pub canvas: CanvasState,
    /// Selection state.
    pub selection: SelectionState,
    /// Pending connection being created.
    pub pending_connection: Option<PendingConnection>,
    /// Configuration.
    pub config: BoardConfig,
    /// Cached pin positions for connection rendering.
    pin_positions: HashMap<PinId, Pos2>,
    /// Cached node rects (in screen space).
    node_rects: HashMap<NodeId, Rect>,
    /// Pin to node mapping.
    pin_to_node: HashMap<PinId, NodeId>,
    /// Node currently being dragged.
    dragging_node: Option<NodeId>,
}

impl BoardState {
    /// Creates a new board state.
    pub fn new() -> Self {
        Self {
            canvas: CanvasState::new(),
            selection: SelectionState::new(),
            pending_connection: None,
            config: BoardConfig::default(),
            pin_positions: HashMap::new(),
            node_rects: HashMap::new(),
            pin_to_node: HashMap::new(),
            dragging_node: None,
        }
    }

    /// Creates a new board state with config.
    pub fn with_config(config: BoardConfig) -> Self {
        Self {
            config,
            ..Self::new()
        }
    }

    /// Resets the view to center.
    pub fn reset_view(&mut self) {
        self.canvas = CanvasState::new();
    }

    /// Fits the view to show all nodes.
    /// Note: This uses cached node_rects which may be from previous frame.
    /// For immediate fit, use `fit_to_nodes` instead.
    pub fn fit_to_content(&mut self, viewport: Rect) {
        if self.node_rects.is_empty() {
            return;
        }

        let mut bounds: Option<Rect> = None;
        for rect in self.node_rects.values() {
            bounds = Some(match bounds {
                Some(b) => b.union(*rect),
                None => *rect,
            });
        }

        if let Some(content_bounds) = bounds {
            let padding = 50.0;
            let padded = content_bounds.expand(padding);

            let scale_x = viewport.width() / padded.width();
            let scale_y = viewport.height() / padded.height();
            let zoom = scale_x.min(scale_y).min(1.0).max(0.1);

            self.canvas.zoom = zoom;
            self.canvas.pan = Vec2::new(
                viewport.center().x - padded.center().x * zoom,
                viewport.center().y - padded.center().y * zoom,
            );
        }
    }

    /// Fits the view to show all provided nodes.
    pub fn fit_to_nodes(&mut self, nodes: &[Node], viewport: Rect) {
        if nodes.is_empty() {
            return;
        }

        // Calculate bounds in canvas space
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;

        for node in nodes {
            min_x = min_x.min(node.position.x);
            max_x = max_x.max(node.position.x + node.size.x);
            min_y = min_y.min(node.position.y);
            max_y = max_y.max(node.position.y + node.size.y);
        }

        let content_bounds = Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y));

        let padding = 50.0;
        let content_with_padding = content_bounds.expand(padding);

        // Calculate zoom to fit
        let zoom_x = viewport.width() / content_with_padding.width();
        let zoom_y = viewport.height() / content_with_padding.height();
        let zoom = zoom_x.min(zoom_y).clamp(0.1, 2.0);

        self.canvas.zoom = zoom;

        // To center: we want content_center to appear at viewport_center
        // screen_pos = canvas_pos * zoom + pan
        // viewport_center = content_center * zoom + pan
        // pan = viewport_center - content_center * zoom
        // But viewport_center should be relative to canvas origin (0,0), so it's viewport.size()/2
        let content_center = content_with_padding.center();
        let viewport_center = viewport.size() / 2.0;

        self.canvas.pan = Vec2::new(
            viewport_center.x - content_center.x * zoom,
            viewport_center.y - content_center.y * zoom,
        );

        eprintln!("fit_to_nodes debug:");
        eprintln!("  content_bounds: {:?}", content_bounds);
        eprintln!("  content_with_padding: {:?}", content_with_padding);
        eprintln!("  content_center: {:?}", content_center);
        eprintln!("  viewport: {:?}", viewport);
        eprintln!("  viewport.size(): {:?}", viewport.size());
        eprintln!("  viewport_center: {:?}", viewport_center);
        eprintln!("  zoom: {}", zoom);
        eprintln!("  pan: {:?}", self.canvas.pan);
    }

    /// Clears cached data.
    pub fn clear_cache(&mut self) {
        self.pin_positions.clear();
        self.node_rects.clear();
        self.pin_to_node.clear();
    }

    /// Returns currently dragging node (for debug).
    pub fn dragging_node(&self) -> Option<NodeId> {
        self.dragging_node
    }

    /// Returns count of cached node rects (for debug).
    pub fn node_rects_count(&self) -> usize {
        self.node_rects.len()
    }
}

impl Default for BoardState {
    fn default() -> Self {
        Self::new()
    }
}

/// The board editor widget.
pub struct BoardEditor<'a> {
    nodes: &'a [Node],
    pins: &'a HashMap<NodeId, (Vec<Pin>, Vec<Pin>)>,
    connections: &'a [Connection],
    state: &'a mut BoardState,
    events: Vec<BoardEvent>,
}

impl<'a> BoardEditor<'a> {
    /// Creates a new board editor.
    pub fn new(
        nodes: &'a [Node],
        pins: &'a HashMap<NodeId, (Vec<Pin>, Vec<Pin>)>,
        connections: &'a [Connection],
        state: &'a mut BoardState,
    ) -> Self {
        Self {
            nodes,
            pins,
            connections,
            state,
            events: Vec::new(),
        }
    }

    /// Shows the board editor in a central panel.
    pub fn show(mut self, ctx: &Context) -> Vec<BoardEvent> {
        CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                self.show_in_ui(ui);
            });

        self.events
    }

    /// Take accumulated events (drains the events vec).
    pub fn take_events(&mut self) -> Vec<BoardEvent> {
        std::mem::take(&mut self.events)
    }

    /// Shows the board editor in a specific UI area.
    pub fn show_in_ui(&mut self, ui: &mut Ui) -> Response {
        // Use Sense::hover() for canvas to avoid stealing drag from nodes
        // Pan/zoom handled via raw input, node dragging via ui.interact() in draw_nodes
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::hover());
        let rect = response.rect;

        // Set clip rect to prevent drawing outside canvas area
        let painter = painter.with_clip_rect(rect);
        ui.set_clip_rect(rect);

        // Store canvas rect for draw_nodes
        self.state.canvas.viewport = rect;

        // Handle input BEFORE clearing caches (uses node_rects from previous frame)
        self.handle_input(ui, &response, rect);

        // Clear pin caches (node_rects kept for next frame's input handling)
        self.state.pin_positions.clear();
        self.state.pin_to_node.clear();

        // Draw canvas background and grid
        let theme = theme::current_theme();
        let tokens = &theme.tokens;
        painter.rect_filled(rect, 0.0, tokens.background);

        if self.state.config.show_grid {
            // Draw grid manually instead of using Canvas::show
            let grid_color = tokens.border.gamma_multiply(0.3);
            let major_grid_color = tokens.border.gamma_multiply(0.6);
            let grid_size = 20.0 * self.state.canvas.zoom;

            if grid_size >= 5.0 {
                let offset = Vec2::new(
                    self.state.canvas.pan.x % grid_size,
                    self.state.canvas.pan.y % grid_size,
                );

                // Vertical lines
                let mut x = rect.min.x + offset.x;
                let mut i = 0;
                while x < rect.max.x {
                    let color = if i % 5 == 0 {
                        major_grid_color
                    } else {
                        grid_color
                    };
                    painter.line_segment(
                        [Pos2::new(x, rect.min.y), Pos2::new(x, rect.max.y)],
                        egui::Stroke::new(1.0, color),
                    );
                    x += grid_size;
                    i += 1;
                }

                // Horizontal lines
                let mut y = rect.min.y + offset.y;
                let mut j = 0;
                while y < rect.max.y {
                    let color = if j % 5 == 0 {
                        major_grid_color
                    } else {
                        grid_color
                    };
                    painter.line_segment(
                        [Pos2::new(rect.min.x, y), Pos2::new(rect.max.x, y)],
                        egui::Stroke::new(1.0, color),
                    );
                    y += grid_size;
                    j += 1;
                }
            }
        }

        // Draw connections first (uses cached pin_positions from previous frame)
        self.draw_connections(ui);

        // Draw nodes second (on top) and update pin_positions for next frame
        self.draw_nodes(ui);

        // Draw pending connection
        if let Some(ref pending) = self.state.pending_connection {
            self.draw_pending_connection(ui, pending);
        }

        // Draw box selection
        if let Some(box_sel) = self.state.selection.box_selection() {
            let theme = theme::current_theme();
            painter.rect_filled(box_sel.rect(), 0.0, theme.tokens.accent.gamma_multiply(0.1));
            painter.rect_stroke(
                box_sel.rect(),
                0,
                egui::Stroke::new(1.0, theme.tokens.accent),
                egui::epaint::StrokeKind::Middle,
            );
        }

        response
    }

    fn handle_input(&mut self, ui: &mut Ui, _response: &Response, rect: Rect) {
        let input = ui.input(|i| i.clone());

        // Update multi-select mode
        self.state
            .selection
            .set_multi_select(input.modifiers.shift || input.modifiers.ctrl);

        // Handle panning with middle mouse or alt+primary drag (using raw input)
        let dominated =
            input.pointer.middle_down() || (input.pointer.primary_down() && input.modifiers.alt);
        if dominated && rect.contains(input.pointer.hover_pos().unwrap_or_default()) {
            let delta = input.pointer.delta();
            if delta != Vec2::ZERO {
                self.state.canvas.pan += delta;
                self.events.push(BoardEvent::CanvasPanned(delta));
            }
        }

        // Handle zooming with scroll
        let scroll_delta = input.raw_scroll_delta.y;
        if scroll_delta != 0.0 && rect.contains(input.pointer.hover_pos().unwrap_or_default()) {
            let zoom_factor = 1.0 + scroll_delta * 0.001;
            let new_zoom = (self.state.canvas.zoom * zoom_factor).clamp(0.1, 3.0);

            if let Some(pointer_pos) = input.pointer.hover_pos() {
                // Zoom towards cursor
                let old_canvas_pos = self.state.canvas.screen_to_canvas(pointer_pos, rect);
                self.state.canvas.zoom = new_zoom;
                let new_screen_pos = self.state.canvas.canvas_to_screen(old_canvas_pos, rect);
                self.state.canvas.pan += pointer_pos - new_screen_pos;
            } else {
                self.state.canvas.zoom = new_zoom;
            }

            self.events.push(BoardEvent::CanvasZoomed(new_zoom));
        }

        // Handle node dragging using raw input
        let dominated_by_pan =
            input.pointer.middle_down() || (input.pointer.primary_down() && input.modifiers.alt);

        // Check if mouse is over a pin (to avoid starting drag when clicking pins)
        let over_pin = if let Some(pos) = input.pointer.hover_pos() {
            self.state.pin_positions.values().any(|&pin_pos| {
                let pin_hit_radius = 12.0 * self.state.canvas.zoom;
                (pos - pin_pos).length() < pin_hit_radius
            })
        } else {
            false
        };

        // Start drag when mouse pressed on a node (but not on a pin)
        if input.pointer.primary_pressed() && !dominated_by_pan && !over_pin {
            if let Some(pos) = input.pointer.hover_pos() {
                if rect.contains(pos) {
                    // Check if clicking on a node (using cached rects from previous frame)
                    for (&node_id, &node_rect) in &self.state.node_rects {
                        if node_rect.contains(pos) {
                            self.state.dragging_node = Some(node_id);
                            // Select the node if not already selected
                            if !self.state.selection.is_node_selected(node_id) {
                                if !self.state.selection.is_multi_select() {
                                    self.state.selection.clear();
                                }
                                self.state.selection.select_node(node_id);
                                self.events.push(BoardEvent::NodeSelected(node_id));
                            }
                            break;
                        }
                    }
                }
            }
        }

        // Handle ongoing node drag
        if let Some(dragging_id) = self.state.dragging_node {
            let pointer_delta = input.pointer.delta();
            if input.pointer.primary_down() && pointer_delta != Vec2::ZERO {
                let zoom = self.state.canvas.zoom;
                let canvas_delta = pointer_delta / zoom;

                // Find current position of the node
                if let Some(node) = self.nodes.iter().find(|n| n.id == dragging_id) {
                    let new_pos = node.position + canvas_delta;

                    self.events.push(BoardEvent::NodeMoved {
                        id: dragging_id,
                        position: new_pos,
                    });
                }
            }
        }

        // Stop dragging when mouse released
        if input.pointer.primary_released() {
            self.state.dragging_node = None;
        }

        // Handle box selection - only if not dragging a node (using raw input)
        if input.pointer.primary_pressed()
            && !dominated_by_pan
            && self.state.pending_connection.is_none()
            && self.state.dragging_node.is_none()
        {
            if let Some(pos) = input.pointer.hover_pos() {
                if rect.contains(pos) {
                    // Check if we're clicking on empty space
                    let on_node = self.state.node_rects.values().any(|r| r.contains(pos));
                    let on_connection = self.connection_at_position(pos).is_some();

                    if !on_node && !on_connection {
                        self.state
                            .selection
                            .start_box_selection(pos, input.modifiers.shift);
                    }
                }
            }
        }

        if self.state.selection.is_box_selecting() {
            if let Some(pos) = input.pointer.hover_pos() {
                self.state.selection.update_box_selection(pos);
            }
        }

        if input.pointer.primary_released() && self.state.selection.is_box_selecting() {
            let box_sel = self.state.selection.finish_box_selection();
            if let Some(sel) = box_sel {
                let sel_rect = sel.rect();
                let canvas = &self.state.canvas;

                // Find nodes in the box
                let nodes_in_box: Vec<NodeId> = self
                    .nodes
                    .iter()
                    .filter(|n| {
                        let screen_pos = canvas.canvas_to_screen(n.position, rect);
                        sel_rect.contains(screen_pos)
                    })
                    .map(|n| n.id)
                    .collect();

                if !sel.additive {
                    self.state.selection.clear();
                }
                for id in nodes_in_box {
                    self.state.selection.select_node(id);
                    self.events.push(BoardEvent::NodeSelected(id));
                }
            }
        }

        // Handle click on empty space (using raw input)
        if input.pointer.primary_clicked() && self.state.pending_connection.is_none() {
            if let Some(pos) = input.pointer.hover_pos() {
                if rect.contains(pos) {
                    let canvas_pos = self.state.canvas.screen_to_canvas(pos, rect);
                    if self.node_at_position(canvas_pos).is_none()
                        && self.connection_at_position(pos).is_none()
                    {
                        if !self.state.selection.is_multi_select() {
                            self.state.selection.clear();
                        }
                        self.events.push(BoardEvent::CanvasClicked(canvas_pos));
                    }
                }
            }
        }

        // Handle double-click (using raw input)
        if input
            .pointer
            .button_double_clicked(egui::PointerButton::Primary)
        {
            if let Some(pos) = input.pointer.hover_pos() {
                if rect.contains(pos) {
                    let canvas_pos = self.state.canvas.screen_to_canvas(pos, rect);
                    self.events
                        .push(BoardEvent::CanvasDoubleClicked(canvas_pos));
                }
            }
        }

        // Handle context menu (using raw input)
        if input.pointer.secondary_clicked() {
            if let Some(pos) = input.pointer.hover_pos() {
                if rect.contains(pos) {
                    let canvas_pos = self.state.canvas.screen_to_canvas(pos, rect);
                    self.events
                        .push(BoardEvent::ContextMenuRequested(canvas_pos));
                }
            }
        }

        // Handle keyboard shortcuts
        if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
            // Delete selected nodes
            for &node_id in self.state.selection.selected_nodes().clone().iter() {
                self.events.push(BoardEvent::NodeDeleteRequested(node_id));
            }
            // Delete selected connections
            for &conn_id in self.state.selection.selected_connections().clone().iter() {
                self.events.push(BoardEvent::ConnectionDeleted(conn_id));
            }
        }

        // Handle escape to cancel pending connection
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.state.pending_connection = None;
            self.state.selection.clear();
        }
    }

    fn draw_nodes(&mut self, ui: &mut Ui) {
        let viewport = self.state.canvas.viewport;
        let pan = self.state.canvas.pan;
        let zoom = self.state.canvas.zoom;

        for node in self.nodes {
            let (input_pins, output_pins) = self
                .pins
                .get(&node.id)
                .map(|(i, o)| (i.as_slice(), o.as_slice()))
                .unwrap_or((&[], &[]));

            let visual_state = if self.state.selection.is_node_selected(node.id) {
                NodeVisualState::Selected
            } else {
                NodeVisualState::Normal
            };

            let node_response = NodeWidget::new(node, input_pins, output_pins)
                .style(self.state.config.node_style.clone())
                .visual_state(visual_state)
                .show(ui, viewport, pan, zoom);

            // Cache node rect (screen space) and pin positions
            self.state
                .node_rects
                .insert(node.id, node_response.screen_rect);

            for pin_resp in &node_response.pin_responses {
                self.state
                    .pin_positions
                    .insert(pin_resp.pin_id, pin_resp.center);
                self.state.pin_to_node.insert(pin_resp.pin_id, node.id);
            }

            // Node dragging and selection handled in handle_input via raw pointer

            // Handle delete button click
            if node_response.delete_requested {
                self.events.push(BoardEvent::NodeDeleteRequested(node.id));
            }

            // Handle pin interactions
            for pin_resp in node_response.pin_responses {
                if pin_resp.clicked {
                    if let Some(pending) = self.state.pending_connection.take() {
                        // Complete connection - only if connecting different pin kinds
                        if pending.from_kind != pin_resp.kind {
                            let (from_node, from_pin, to_node, to_pin) = match pending.from_kind {
                                PinKind::Output => (
                                    pending.from_node,
                                    pending.from_pin,
                                    node.id,
                                    pin_resp.pin_id,
                                ),
                                PinKind::Input => (
                                    node.id,
                                    pin_resp.pin_id,
                                    pending.from_node,
                                    pending.from_pin,
                                ),
                            };

                            self.events.push(BoardEvent::ConnectionCreated {
                                from_node,
                                from_pin,
                                to_node,
                                to_pin,
                            });
                        }
                    } else {
                        // Start new connection
                        let pin = self.find_pin(pin_resp.pin_id);
                        if let Some(pin) = pin {
                            self.state.pending_connection = Some(PendingConnection {
                                from_node: node.id,
                                from_pin: pin_resp.pin_id,
                                from_kind: pin_resp.kind,
                                data_type: pin.data_type.clone(),
                                to_pos: pin_resp.center,
                            });
                        }
                    }
                }
            }
        }
    }

    fn draw_connections(&mut self, ui: &mut Ui) {
        let zoom = self.state.canvas.zoom;

        for connection in self.connections {
            let from_pos = self.state.pin_positions.get(&connection.source).copied();
            let to_pos = self.state.pin_positions.get(&connection.target).copied();

            if let (Some(from), Some(to)) = (from_pos, to_pos) {
                let state = if self.state.selection.is_connection_selected(connection.id) {
                    ConnectionState::Selected
                } else {
                    ConnectionState::Normal
                };

                // Create renderer with connection's edge type
                let mut style = self.state.config.connection_style.clone();
                style.edge_type = connection.edge_type;

                let renderer = ConnectionRenderer::new().style(style);

                renderer.draw_connection(ui, connection, from, to, state, zoom);
            }
        }
    }

    fn draw_pending_connection(&self, ui: &mut Ui, pending: &PendingConnection) {
        let renderer = ConnectionRenderer::new().style(self.state.config.connection_style.clone());
        let zoom = self.state.canvas.zoom;

        if let Some(&from_pos) = self.state.pin_positions.get(&pending.from_pin) {
            let cursor_pos = ui.input(|i| i.pointer.hover_pos().unwrap_or(pending.to_pos));

            // Check if hovering over a valid target
            let valid = true; // Simplified - would check pin compatibility

            renderer.draw_preview(
                ui,
                from_pos,
                cursor_pos,
                pending.from_kind,
                &pending.data_type,
                valid,
                zoom,
            );
        }
    }

    fn node_at_position(&self, canvas_pos: Pos2) -> Option<NodeId> {
        for node in self.nodes.iter().rev() {
            if let Some(rect) = self.state.node_rects.get(&node.id) {
                if rect.contains(canvas_pos) {
                    return Some(node.id);
                }
            }
        }
        None
    }

    fn connection_at_position(&self, screen_pos: Pos2) -> Option<ConnectionId> {
        let threshold = self.state.config.connection_hit_threshold;
        let curve_factor = self.state.config.connection_style.curve_factor;

        for connection in self.connections {
            let from_pos = self.state.pin_positions.get(&connection.source).copied();
            let to_pos = self.state.pin_positions.get(&connection.target).copied();

            if let (Some(from), Some(to)) = (from_pos, to_pos) {
                if point_near_connection(screen_pos, from, to, curve_factor, threshold) {
                    return Some(connection.id);
                }
            }
        }
        None
    }

    fn find_pin(&self, pin_id: PinId) -> Option<&Pin> {
        for (_, (inputs, outputs)) in self.pins {
            for pin in inputs.iter().chain(outputs.iter()) {
                if pin.id == pin_id {
                    return Some(pin);
                }
            }
        }
        None
    }
}
