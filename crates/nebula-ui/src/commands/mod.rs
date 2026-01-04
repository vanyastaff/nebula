//! Command system with undo/redo support.
//!
//! This module provides a command pattern implementation for managing
//! state changes with full undo/redo capability.
//!
//! # Example
//!
//! ```ignore
//! use nebula_ui::commands::{Command, CommandHistory};
//!
//! struct MoveNodeCommand {
//!     node_id: NodeId,
//!     old_pos: Pos2,
//!     new_pos: Pos2,
//! }
//!
//! impl Command for MoveNodeCommand {
//!     fn execute(&mut self, state: &mut dyn Any) -> Result<(), CommandError> {
//!         // Apply the move
//!         Ok(())
//!     }
//!
//!     fn undo(&mut self, state: &mut dyn Any) -> Result<(), CommandError> {
//!         // Revert the move
//!         Ok(())
//!     }
//!
//!     fn description(&self) -> &str {
//!         "Move node"
//!     }
//! }
//!
//! let mut history = CommandHistory::new();
//! history.execute(Box::new(cmd), &mut state)?;
//! history.undo(&mut state)?;
//! history.redo(&mut state)?;
//! ```

mod command;
mod history;
mod transaction;

pub use command::{Command, CommandError, CommandResult};
pub use history::{CommandHistory, HistoryConfig, HistoryEvent};
pub use transaction::{Transaction, TransactionBuilder};

/// Prelude for command system
pub mod prelude {
    pub use super::{
        Command, CommandError, CommandHistory, CommandResult, HistoryConfig, HistoryEvent,
        Transaction, TransactionBuilder,
    };
}
