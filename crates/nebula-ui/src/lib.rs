//! # nebula-ui
//!
//! UI component library for Nebula workflow automation.
#![allow(clippy::excessive_nesting)]
#![allow(unused_imports)]
//!
//! ## Overview
//!
//! `nebula-ui` provides a complete UI toolkit built on [egui](https://github.com/emilk/egui),
//! designed specifically for visual workflow programming. It includes:
//!
//! - **Theme System**: Comprehensive theming with design tokens, dark/light modes
//! - **Base Components**: Button, Input, Card, Dialog, Select, etc.
//! - **Layout Components**: Sidebar, Panel, Toolbar, SplitView
//! - **Flow Components**: Node editor, pins, connections, layers
//! - **Command System**: Undo/redo with command pattern
//!
//! ## Architecture
//!
//! The library follows patterns inspired by modern UI frameworks:
//!
//! - **Design Tokens**: All visual properties (colors, spacing, radii) are centralized
//! - **Component Composition**: Small, focused components that compose together
//! - **State Management**: Centralized app state with clear data flow
//! - **Command Pattern**: All mutations go through reversible commands
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use nebula_ui::prelude::*;
//!
//! fn ui(ctx: &egui::Context) {
//!     // Apply theme
//!     let theme = Theme::dark();
//!     theme.apply(ctx);
//!
//!     egui::CentralPanel::default().show(ctx, |ui| {
//!         // Use themed components
//!         if Button::new("Click me").primary().show(ui).clicked() {
//!             println!("Clicked!");
//!         }
//!
//!         let mut text = String::new();
//!         TextInput::new(&mut text)
//!             .placeholder("Enter text...")
//!             .show(ui);
//!     });
//! }
//! ```

#![warn(clippy::all)]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

pub mod commands;
pub mod components;
pub mod flow;
pub mod icons;
pub mod layout;
pub mod state;
pub mod theme;

// Re-exports
pub use commands::{Command, CommandHistory};
pub use components::prelude::*;
pub use flow::prelude::*;
pub use state::AppState;
pub use theme::{DataTypeColors, NodeCategoryColors, Theme, ThemeTokens};

/// Prelude for common imports
pub mod prelude {
    pub use crate::commands::{Command, CommandHistory};
    pub use crate::components::prelude::*;
    pub use crate::flow::prelude::*;
    pub use crate::icons::Icon;
    pub use crate::layout::prelude::*;
    pub use crate::state::AppState;
    pub use crate::theme::{Theme, ThemeTokens, current_theme, set_theme};
}
