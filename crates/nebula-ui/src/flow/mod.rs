//! Flow editor components for visual node-based programming.
//!
//! This module provides components for building a visual workflow editor:
//!
//! - [`BoardEditor`]: Main canvas with pan/zoom
//! - [`NodeWidget`]: Individual node rendering
//! - [`PinWidget`]: Input/output pin rendering
//! - [`Connection`]: Bezier curve connections
//! - [`Layer`]: Hierarchical node grouping
//! - [`Minimap`]: Bird's-eye view for navigation
//! - [`Controls`]: Zoom and view controls
//! - [`Background`]: Customizable background patterns

mod background;
mod board;
mod canvas;
mod connection;
mod controls;
mod minimap;
mod node;
mod pin;
mod selection;
mod shortcuts;
mod types;

pub use background::{Background, BackgroundConfig, BackgroundVariant};
pub use board::{BoardConfig, BoardEditor, BoardEvent, BoardState, PendingConnection};
pub use canvas::{Canvas, CanvasState};
pub use connection::{
    ConnectionRenderer, ConnectionState, ConnectionStyle, EdgeType, connection_id,
    point_near_connection,
};
pub use controls::{ControlAction, Controls, ControlsConfig, ControlsPosition, ControlsResponse};
pub use minimap::{Minimap, MinimapConfig, MinimapPosition, MinimapResponse};
pub use node::{NodeResponse, NodeStyle, NodeVisualState, NodeWidget, PinResponse};
pub use pin::{PinInteraction, PinState, PinStyle, PinWidget, can_connect, connection_offset};
pub use selection::{
    BoxSelection, SelectionState, move_selection, selection_bounds, selection_center,
};
pub use shortcuts::{KeyboardShortcuts, ShortcutAction, ShortcutsConfig};
pub use types::*;

/// Prelude for flow components
pub mod prelude {
    pub use super::{
        // Background
        Background,
        BackgroundConfig,
        BackgroundVariant,
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
        // Controls
        ControlAction,
        Controls,
        ControlsConfig,
        ControlsPosition,
        ControlsResponse,
        // Types
        DataType,
        EdgeType,
        // Shortcuts
        KeyboardShortcuts,
        Layer,
        LayerId,
        // Minimap
        Minimap,
        MinimapConfig,
        MinimapPosition,
        MinimapResponse,
        // Types
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
        ShortcutAction,
        ShortcutsConfig,
        // Value types
        ValueType,
        Variable,
    };
}
