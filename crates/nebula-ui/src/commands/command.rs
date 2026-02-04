//! Command trait and error types.

use std::any::Any;
use std::fmt;

/// Error type for command execution.
#[derive(Debug, Clone)]
pub enum CommandError {
    /// The command failed to execute.
    ExecutionFailed(String),
    /// The command cannot be undone.
    UndoFailed(String),
    /// The command cannot be redone.
    RedoFailed(String),
    /// The state type is invalid.
    InvalidState(String),
    /// A precondition was not met.
    PreconditionFailed(String),
    /// A custom error.
    Custom(String),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExecutionFailed(msg) => write!(f, "Execution failed: {msg}"),
            Self::UndoFailed(msg) => write!(f, "Undo failed: {msg}"),
            Self::RedoFailed(msg) => write!(f, "Redo failed: {msg}"),
            Self::InvalidState(msg) => write!(f, "Invalid state: {msg}"),
            Self::PreconditionFailed(msg) => write!(f, "Precondition failed: {msg}"),
            Self::Custom(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for CommandError {}

/// Result type for command operations.
pub type CommandResult<T = ()> = Result<T, CommandError>;

/// A command that can be executed, undone, and redone.
///
/// Commands encapsulate a state change that can be reversed.
/// Each command stores enough information to both apply and revert the change.
pub trait Command: Send + Sync {
    /// Executes the command, applying the change to the state.
    ///
    /// # Arguments
    /// * `state` - The application state to modify
    ///
    /// # Returns
    /// * `Ok(())` if the command executed successfully
    /// * `Err(CommandError)` if the command failed
    fn execute(&mut self, state: &mut dyn Any) -> CommandResult;

    /// Undoes the command, reverting the state to before execution.
    ///
    /// # Arguments
    /// * `state` - The application state to revert
    ///
    /// # Returns
    /// * `Ok(())` if the undo was successful
    /// * `Err(CommandError)` if the undo failed
    fn undo(&mut self, state: &mut dyn Any) -> CommandResult;

    /// Redoes the command after it has been undone.
    ///
    /// By default, this calls `execute` again. Override if redo
    /// requires different logic than the initial execution.
    fn redo(&mut self, state: &mut dyn Any) -> CommandResult {
        self.execute(state)
    }

    /// Returns a human-readable description of this command.
    ///
    /// Used for displaying in undo/redo menus and history views.
    fn description(&self) -> &str;

    /// Returns a unique identifier for this command type.
    ///
    /// Used for command merging and grouping.
    fn id(&self) -> Option<&str> {
        None
    }

    /// Returns whether this command can be merged with another.
    ///
    /// Merging combines multiple similar commands into one,
    /// useful for things like continuous dragging or typing.
    fn can_merge(&self, _other: &dyn Command) -> bool {
        false
    }

    /// Merges another command into this one.
    ///
    /// Returns true if the merge was successful.
    fn merge(&mut self, _other: Box<dyn Command>) -> bool {
        false
    }

    /// Returns the approximate memory size of this command.
    ///
    /// Used for memory management in the command history.
    fn memory_size(&self) -> usize {
        std::mem::size_of_val(self)
    }

    /// Returns whether this command is significant.
    ///
    /// Non-significant commands (like selection changes) may be
    /// skipped when determining save state or grouped differently.
    fn is_significant(&self) -> bool {
        true
    }
}

/// A no-op command that does nothing.
///
/// Useful as a placeholder or for testing.
#[derive(Debug, Default)]
pub struct NoOpCommand {
    description: String,
}

impl NoOpCommand {
    /// Creates a new no-op command.
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
        }
    }
}

impl Command for NoOpCommand {
    fn execute(&mut self, _state: &mut dyn Any) -> CommandResult {
        Ok(())
    }

    fn undo(&mut self, _state: &mut dyn Any) -> CommandResult {
        Ok(())
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn is_significant(&self) -> bool {
        false
    }
}

/// A command that wraps a closure for simple one-off commands.
pub struct FnCommand<F, U>
where
    F: FnMut(&mut dyn Any) -> CommandResult + Send + Sync,
    U: FnMut(&mut dyn Any) -> CommandResult + Send + Sync,
{
    execute_fn: F,
    undo_fn: U,
    description: String,
}

impl<F, U> FnCommand<F, U>
where
    F: FnMut(&mut dyn Any) -> CommandResult + Send + Sync,
    U: FnMut(&mut dyn Any) -> CommandResult + Send + Sync,
{
    /// Creates a new function command.
    pub fn new(description: impl Into<String>, execute_fn: F, undo_fn: U) -> Self {
        Self {
            execute_fn,
            undo_fn,
            description: description.into(),
        }
    }
}

impl<F, U> Command for FnCommand<F, U>
where
    F: FnMut(&mut dyn Any) -> CommandResult + Send + Sync,
    U: FnMut(&mut dyn Any) -> CommandResult + Send + Sync,
{
    fn execute(&mut self, state: &mut dyn Any) -> CommandResult {
        (self.execute_fn)(state)
    }

    fn undo(&mut self, state: &mut dyn Any) -> CommandResult {
        (self.undo_fn)(state)
    }

    fn description(&self) -> &str {
        &self.description
    }
}

/// Helper macro for creating simple commands.
#[macro_export]
macro_rules! command {
    ($desc:expr, execute: $exec:expr, undo: $undo:expr) => {
        $crate::commands::FnCommand::new($desc, $exec, $undo)
    };
}
