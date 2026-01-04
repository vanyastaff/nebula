//! Application state management.
//!
//! Provides a centralized state container following the IBackendState pattern.

use std::collections::HashMap;

use crate::commands::CommandHistory;
use crate::flow::{Connection, ConnectionId, Layer, LayerId, Node, NodeId, Pin, PinId, Variable};

/// Central application state for the flow editor.
///
/// This follows the IBackendState pattern, providing a single source of truth
/// for all application data with clear domain slices.
#[derive(Default)]
pub struct AppState {
    /// Board state (nodes, connections, etc.).
    pub board: BoardState,
    /// UI state (selection, view settings).
    pub ui: UiState,
    /// Command history for undo/redo.
    pub history: CommandHistory,
}

impl AppState {
    /// Creates a new empty application state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets the state to initial values.
    pub fn reset(&mut self) {
        self.board = BoardState::default();
        self.ui = UiState::default();
        self.history.clear();
    }
}

/// State for the flow board (nodes, connections, layers).
#[derive(Default)]
pub struct BoardState {
    /// All nodes in the board.
    pub nodes: HashMap<NodeId, Node>,
    /// All pins, grouped by node.
    pub pins: HashMap<NodeId, (Vec<Pin>, Vec<Pin>)>,
    /// All connections.
    pub connections: HashMap<ConnectionId, Connection>,
    /// All layers.
    pub layers: HashMap<LayerId, Layer>,
    /// All variables.
    pub variables: Vec<Variable>,
    /// Root layer ID.
    pub root_layer: Option<LayerId>,
    /// Node ordering for rendering (back to front).
    pub node_order: Vec<NodeId>,
}

impl BoardState {
    /// Creates a new empty board state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a node to the board.
    pub fn add_node(&mut self, node: Node) {
        let id = node.id;
        self.node_order.push(id);
        self.nodes.insert(id, node);
    }

    /// Removes a node from the board.
    pub fn remove_node(&mut self, id: NodeId) -> Option<Node> {
        self.node_order.retain(|&n| n != id);
        self.pins.remove(&id);

        // Remove connections to/from this node
        let pin_ids: Vec<PinId> = self
            .pins
            .get(&id)
            .map(|(inputs, outputs)| inputs.iter().chain(outputs.iter()).map(|p| p.id).collect())
            .unwrap_or_default();

        self.connections
            .retain(|_, conn| !pin_ids.contains(&conn.source) && !pin_ids.contains(&conn.target));

        self.nodes.remove(&id)
    }

    /// Gets a node by ID.
    pub fn get_node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// Gets a mutable node by ID.
    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&id)
    }

    /// Adds a connection.
    pub fn add_connection(&mut self, connection: Connection) {
        self.connections.insert(connection.id, connection);
    }

    /// Removes a connection.
    pub fn remove_connection(&mut self, id: ConnectionId) -> Option<Connection> {
        self.connections.remove(&id)
    }

    /// Gets all nodes as a slice (in render order).
    pub fn nodes_ordered(&self) -> Vec<&Node> {
        self.node_order
            .iter()
            .filter_map(|id| self.nodes.get(id))
            .collect()
    }

    /// Gets all connections as a vec.
    pub fn connections_vec(&self) -> Vec<&Connection> {
        self.connections.values().collect()
    }

    /// Brings a node to the front (top of render order).
    pub fn bring_to_front(&mut self, id: NodeId) {
        self.node_order.retain(|&n| n != id);
        self.node_order.push(id);
    }

    /// Sends a node to the back (bottom of render order).
    pub fn send_to_back(&mut self, id: NodeId) {
        self.node_order.retain(|&n| n != id);
        self.node_order.insert(0, id);
    }

    /// Clears all board content.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.pins.clear();
        self.connections.clear();
        self.layers.clear();
        self.variables.clear();
        self.node_order.clear();
        self.root_layer = None;
    }
}

/// UI-related state (not part of document data).
#[derive(Default)]
pub struct UiState {
    /// Whether the sidebar is visible.
    pub sidebar_visible: bool,
    /// Whether dark mode is enabled.
    pub dark_mode: bool,
    /// Current zoom level.
    pub zoom: f32,
    /// Whether the minimap is visible.
    pub minimap_visible: bool,
    /// Whether grid snapping is enabled.
    pub snap_to_grid: bool,
    /// Grid size for snapping.
    pub grid_size: f32,
    /// Currently focused node (for keyboard navigation).
    pub focused_node: Option<NodeId>,
    /// Search/filter text.
    pub search_text: String,
    /// Whether the command palette is open.
    pub command_palette_open: bool,
}

impl UiState {
    /// Creates a new UI state with default values.
    pub fn new() -> Self {
        Self {
            sidebar_visible: true,
            dark_mode: true,
            zoom: 1.0,
            minimap_visible: false,
            snap_to_grid: true,
            grid_size: 20.0,
            focused_node: None,
            search_text: String::new(),
            command_palette_open: false,
        }
    }
}
