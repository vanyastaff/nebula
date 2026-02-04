//! Selection state management for flow editor.
//!
//! Handles node and connection selection, multi-select, and box selection.

use std::collections::HashSet;

use egui::{Pos2, Rect, Vec2};

use super::types::{ConnectionId, NodeId};

/// Selection state for the flow editor.
#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    /// Currently selected nodes.
    selected_nodes: HashSet<NodeId>,
    /// Currently selected connections.
    selected_connections: HashSet<ConnectionId>,
    /// Primary selected node (last clicked).
    primary_node: Option<NodeId>,
    /// Whether multi-select is active (Shift/Ctrl held).
    multi_select: bool,
    /// Box selection state.
    box_selection: Option<BoxSelection>,
}

/// State for box/marquee selection.
#[derive(Debug, Clone)]
pub struct BoxSelection {
    /// Starting point of the selection box.
    pub start: Pos2,
    /// Current end point of the selection box.
    pub end: Pos2,
    /// Whether to add to or replace selection.
    pub additive: bool,
}

impl BoxSelection {
    /// Creates a new box selection.
    pub fn new(start: Pos2, additive: bool) -> Self {
        Self {
            start,
            end: start,
            additive,
        }
    }

    /// Gets the selection rectangle.
    pub fn rect(&self) -> Rect {
        Rect::from_two_pos(self.start, self.end)
    }

    /// Updates the end point.
    pub fn update(&mut self, pos: Pos2) {
        self.end = pos;
    }
}

impl SelectionState {
    /// Creates a new empty selection state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.selected_nodes.is_empty() && self.selected_connections.is_empty()
    }

    /// Returns the number of selected items.
    pub fn count(&self) -> usize {
        self.selected_nodes.len() + self.selected_connections.len()
    }

    // Node selection methods

    /// Returns true if the node is selected.
    pub fn is_node_selected(&self, id: NodeId) -> bool {
        self.selected_nodes.contains(&id)
    }

    /// Returns true if the node is the primary selection.
    pub fn is_primary_node(&self, id: NodeId) -> bool {
        self.primary_node == Some(id)
    }

    /// Gets all selected nodes.
    pub fn selected_nodes(&self) -> &HashSet<NodeId> {
        &self.selected_nodes
    }

    /// Gets the primary selected node.
    pub fn primary_node(&self) -> Option<NodeId> {
        self.primary_node
    }

    /// Selects a single node, clearing other selections.
    pub fn select_node(&mut self, id: NodeId) {
        if !self.multi_select {
            self.clear();
        }
        self.selected_nodes.insert(id);
        self.primary_node = Some(id);
    }

    /// Toggles node selection.
    pub fn toggle_node(&mut self, id: NodeId) {
        if self.selected_nodes.contains(&id) {
            self.selected_nodes.remove(&id);
            if self.primary_node == Some(id) {
                self.primary_node = self.selected_nodes.iter().next().copied();
            }
        } else {
            self.selected_nodes.insert(id);
            self.primary_node = Some(id);
        }
    }

    /// Deselects a node.
    pub fn deselect_node(&mut self, id: NodeId) {
        self.selected_nodes.remove(&id);
        if self.primary_node == Some(id) {
            self.primary_node = self.selected_nodes.iter().next().copied();
        }
    }

    /// Selects multiple nodes.
    pub fn select_nodes(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        if !self.multi_select {
            self.selected_nodes.clear();
        }
        for id in ids {
            self.selected_nodes.insert(id);
            self.primary_node = Some(id);
        }
    }

    // Connection selection methods

    /// Returns true if the connection is selected.
    pub fn is_connection_selected(&self, id: ConnectionId) -> bool {
        self.selected_connections.contains(&id)
    }

    /// Gets all selected connections.
    pub fn selected_connections(&self) -> &HashSet<ConnectionId> {
        &self.selected_connections
    }

    /// Selects a connection.
    pub fn select_connection(&mut self, id: ConnectionId) {
        if !self.multi_select {
            self.clear();
        }
        self.selected_connections.insert(id);
    }

    /// Toggles connection selection.
    pub fn toggle_connection(&mut self, id: ConnectionId) {
        if self.selected_connections.contains(&id) {
            self.selected_connections.remove(&id);
        } else {
            self.selected_connections.insert(id);
        }
    }

    /// Deselects a connection.
    pub fn deselect_connection(&mut self, id: ConnectionId) {
        self.selected_connections.remove(&id);
    }

    // General methods

    /// Clears all selections.
    pub fn clear(&mut self) {
        self.selected_nodes.clear();
        self.selected_connections.clear();
        self.primary_node = None;
    }

    /// Sets multi-select mode.
    pub fn set_multi_select(&mut self, enabled: bool) {
        self.multi_select = enabled;
    }

    /// Returns true if multi-select is enabled.
    pub fn is_multi_select(&self) -> bool {
        self.multi_select
    }

    // Box selection methods

    /// Starts a box selection.
    pub fn start_box_selection(&mut self, pos: Pos2, additive: bool) {
        self.box_selection = Some(BoxSelection::new(pos, additive));
    }

    /// Updates the box selection end point.
    pub fn update_box_selection(&mut self, pos: Pos2) {
        if let Some(ref mut box_sel) = self.box_selection {
            box_sel.update(pos);
        }
    }

    /// Finishes box selection and returns the selection rectangle.
    pub fn finish_box_selection(&mut self) -> Option<BoxSelection> {
        self.box_selection.take()
    }

    /// Gets the current box selection state.
    pub fn box_selection(&self) -> Option<&BoxSelection> {
        self.box_selection.as_ref()
    }

    /// Returns true if box selection is active.
    pub fn is_box_selecting(&self) -> bool {
        self.box_selection.is_some()
    }

    /// Applies box selection to nodes based on their positions.
    pub fn apply_box_selection<F>(&mut self, node_in_rect: F)
    where
        F: Fn(Rect) -> Vec<NodeId>,
    {
        if let Some(box_sel) = self.box_selection.take() {
            let rect = box_sel.rect();
            let nodes_in_rect = node_in_rect(rect);

            if !box_sel.additive {
                self.selected_nodes.clear();
            }

            for id in nodes_in_rect {
                self.selected_nodes.insert(id);
            }

            self.primary_node = self.selected_nodes.iter().next().copied();
        }
    }
}

/// Calculates the bounding box of selected nodes.
pub fn selection_bounds<F>(selection: &SelectionState, get_node_rect: F) -> Option<Rect>
where
    F: Fn(NodeId) -> Option<Rect>,
{
    let mut bounds: Option<Rect> = None;

    for &node_id in selection.selected_nodes() {
        if let Some(rect) = get_node_rect(node_id) {
            bounds = Some(match bounds {
                Some(b) => b.union(rect),
                None => rect,
            });
        }
    }

    bounds
}

/// Calculates the center of selected nodes.
pub fn selection_center<F>(selection: &SelectionState, get_node_rect: F) -> Option<Pos2>
where
    F: Fn(NodeId) -> Option<Rect>,
{
    selection_bounds(selection, get_node_rect).map(|r| r.center())
}

/// Moves selected nodes by a delta.
pub fn move_selection<F>(selection: &SelectionState, delta: Vec2, mut move_node: F)
where
    F: FnMut(NodeId, Vec2),
{
    for &node_id in selection.selected_nodes() {
        move_node(node_id, delta);
    }
}
