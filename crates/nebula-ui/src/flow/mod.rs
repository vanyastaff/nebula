//! Flow editor components for visual node-based programming.
//!
//! This module provides components for building a visual workflow editor:
//!
//! - [`BoardEditor`]: Main canvas with pan/zoom
//! - [`NodeWidget`]: Individual node rendering
//! - [`PinWidget`]: Input/output pin rendering
//! - [`Connection`]: Bezier curve connections
//! - [`Layer`]: Hierarchical node grouping

mod board;
mod canvas;
mod connection;
mod node;
mod pin;
mod selection;
mod types;

pub use board::{BoardConfig, BoardEditor, BoardEvent, BoardState, PendingConnection};
pub use canvas::{Canvas, CanvasState};
pub use connection::{
    ConnectionRenderer, ConnectionState, ConnectionStyle, connection_id, point_near_connection,
};
pub use node::{NodeResponse, NodeStyle, NodeVisualState, NodeWidget, PinResponse};
pub use pin::{PinInteraction, PinState, PinStyle, PinWidget, can_connect, connection_offset};
pub use selection::{
    BoxSelection, SelectionState, move_selection, selection_bounds, selection_center,
};
pub use types::*;

/// Prelude for flow components
pub mod prelude {
    pub use super::{
        // Board
        BoardConfig,
        BoardEditor,
        BoardEvent,
        BoardState,
        // Selection
        BoxSelection,
        // Canvas
        Canvas,
        CanvasState,
        // Connections
        Connection,
        ConnectionId,
        ConnectionRenderer,
        ConnectionState,
        ConnectionStyle,
        // Types
        DataType,
        Layer,
        LayerId,
        Node,
        NodeId,
        NodeResponse,
        NodeStyle,
        NodeVisualState,
        NodeWidget,
        Pin,
        PinId,
        PinInteraction,
        PinKind,
        PinResponse,
        PinState,
        PinStyle,
        PinWidget,
        SelectionState,
        // Value types
        ValueType,
        Variable,
    };
}
