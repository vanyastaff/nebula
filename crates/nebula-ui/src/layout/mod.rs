//! Layout components for nebula-ui.
//!
//! Provides structural components for building application layouts:
//!
//! - [`Sidebar`]: Collapsible side navigation
//! - [`Panel`]: Content panels with headers
//! - [`Toolbar`]: Horizontal toolbar with items
//! - [`SplitView`]: Resizable split panes

// Layout components have complex nested structure for hierarchical UI
#![allow(clippy::excessive_nesting)]

mod panel;
mod sidebar;
mod split;
mod toolbar;

pub use panel::{Panel, PanelPosition};
pub use sidebar::{Sidebar, SidebarItem, SidebarSection};
pub use split::{SplitDirection, SplitView};
pub use toolbar::{Toolbar, ToolbarItem};

/// Prelude for layout components
pub mod prelude {
    pub use super::{
        Panel, PanelPosition, Sidebar, SidebarItem, SidebarSection, SplitDirection, SplitView,
        Toolbar, ToolbarItem,
    };
}
