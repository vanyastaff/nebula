//! Command history with undo/redo stack.

use std::any::Any;
use std::collections::VecDeque;

use super::command::{Command, CommandError, CommandResult};

/// Configuration for command history.
#[derive(Debug, Clone)]
pub struct HistoryConfig {
    /// Maximum number of commands to keep in history.
    pub max_history: usize,
    /// Maximum memory usage for history (in bytes).
    pub max_memory: usize,
    /// Whether to merge consecutive similar commands.
    pub enable_merging: bool,
    /// Time window for merging commands (in milliseconds).
    pub merge_window_ms: u64,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_history: 100,
            max_memory: 50 * 1024 * 1024, // 50 MB
            enable_merging: true,
            merge_window_ms: 500,
        }
    }
}

/// Events emitted by the command history.
#[derive(Debug, Clone)]
pub enum HistoryEvent {
    /// A command was executed.
    CommandExecuted { description: String },
    /// A command was undone.
    CommandUndone { description: String },
    /// A command was redone.
    CommandRedone { description: String },
    /// Commands were merged.
    CommandsMerged { count: usize },
    /// History was cleared.
    HistoryCleared,
    /// A save point was set.
    SavePointSet,
}

/// Manages command history with undo/redo capability.
pub struct CommandHistory {
    /// Commands that can be undone.
    undo_stack: VecDeque<Box<dyn Command>>,
    /// Commands that can be redone.
    redo_stack: Vec<Box<dyn Command>>,
    /// Configuration.
    config: HistoryConfig,
    /// Current memory usage estimate.
    memory_usage: usize,
    /// Index of the last saved state (-1 if never saved).
    save_point: Option<usize>,
    /// Timestamp of last command for merging.
    last_command_time: std::time::Instant,
    /// Event listeners.
    listeners: Vec<Box<dyn Fn(&HistoryEvent) + Send + Sync>>,
}

impl CommandHistory {
    /// Creates a new command history with default config.
    pub fn new() -> Self {
        Self::with_config(HistoryConfig::default())
    }

    /// Creates a new command history with the given config.
    pub fn with_config(config: HistoryConfig) -> Self {
        Self {
            undo_stack: VecDeque::with_capacity(config.max_history),
            redo_stack: Vec::new(),
            config,
            memory_usage: 0,
            save_point: None,
            last_command_time: std::time::Instant::now(),
            listeners: Vec::new(),
        }
    }

    /// Executes a command and adds it to the history.
    pub fn execute(&mut self, mut command: Box<dyn Command>, state: &mut dyn Any) -> CommandResult {
        // Execute the command
        command.execute(state)?;

        let description = command.description().to_string();

        // Clear redo stack
        self.redo_stack.clear();

        // Try to merge with previous command
        let merged = if self.config.enable_merging {
            self.try_merge_command(&mut command)
        } else {
            false
        };

        if !merged {
            // Add to undo stack
            self.memory_usage += command.memory_size();
            self.undo_stack.push_back(command);

            // Enforce limits
            self.enforce_limits();
        }

        self.last_command_time = std::time::Instant::now();

        self.emit_event(HistoryEvent::CommandExecuted { description });

        Ok(())
    }

    /// Undoes the last command.
    pub fn undo(&mut self, state: &mut dyn Any) -> CommandResult {
        let mut command = self
            .undo_stack
            .pop_back()
            .ok_or_else(|| CommandError::UndoFailed("Nothing to undo".to_string()))?;

        let description = command.description().to_string();

        command.undo(state)?;

        self.redo_stack.push(command);

        self.emit_event(HistoryEvent::CommandUndone { description });

        Ok(())
    }

    /// Redoes the last undone command.
    pub fn redo(&mut self, state: &mut dyn Any) -> CommandResult {
        let mut command = self
            .redo_stack
            .pop()
            .ok_or_else(|| CommandError::RedoFailed("Nothing to redo".to_string()))?;

        let description = command.description().to_string();

        command.redo(state)?;

        self.undo_stack.push_back(command);

        self.emit_event(HistoryEvent::CommandRedone { description });

        Ok(())
    }

    /// Returns true if there are commands to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Returns true if there are commands to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Returns the description of the next command to undo.
    pub fn undo_description(&self) -> Option<&str> {
        self.undo_stack.back().map(|c| c.description())
    }

    /// Returns the description of the next command to redo.
    pub fn redo_description(&self) -> Option<&str> {
        self.redo_stack.last().map(|c| c.description())
    }

    /// Returns the number of commands in the undo stack.
    pub fn undo_count(&self) -> usize {
        self.undo_stack.len()
    }

    /// Returns the number of commands in the redo stack.
    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }

    /// Clears all history.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.memory_usage = 0;
        self.save_point = None;

        self.emit_event(HistoryEvent::HistoryCleared);
    }

    /// Marks the current state as saved.
    pub fn set_save_point(&mut self) {
        self.save_point = Some(self.undo_stack.len());

        self.emit_event(HistoryEvent::SavePointSet);
    }

    /// Returns true if the current state matches the save point.
    pub fn is_at_save_point(&self) -> bool {
        self.save_point == Some(self.undo_stack.len())
    }

    /// Returns true if there are unsaved changes.
    pub fn has_unsaved_changes(&self) -> bool {
        !self.is_at_save_point()
    }

    /// Returns the estimated memory usage.
    pub fn memory_usage(&self) -> usize {
        self.memory_usage
    }

    /// Adds an event listener.
    pub fn add_listener(&mut self, listener: impl Fn(&HistoryEvent) + Send + Sync + 'static) {
        self.listeners.push(Box::new(listener));
    }

    /// Returns an iterator over undo command descriptions.
    pub fn undo_history(&self) -> impl Iterator<Item = &str> {
        self.undo_stack.iter().rev().map(|c| c.description())
    }

    /// Returns an iterator over redo command descriptions.
    pub fn redo_history(&self) -> impl Iterator<Item = &str> {
        self.redo_stack.iter().rev().map(|c| c.description())
    }

    fn try_merge_command(&mut self, command: &mut Box<dyn Command>) -> bool {
        if self.undo_stack.is_empty() {
            return false;
        }

        let elapsed = self.last_command_time.elapsed().as_millis() as u64;
        if elapsed > self.config.merge_window_ms {
            return false;
        }

        let last = self.undo_stack.back_mut().unwrap();
        if last.can_merge(command.as_ref()) {
            // We need to take ownership for merging
            // This is a limitation - we'd need to restructure for proper merging
            false
        } else {
            false
        }
    }

    fn enforce_limits(&mut self) {
        // Enforce max history count
        while self.undo_stack.len() > self.config.max_history {
            if let Some(cmd) = self.undo_stack.pop_front() {
                self.memory_usage = self.memory_usage.saturating_sub(cmd.memory_size());

                // Adjust save point
                if let Some(ref mut sp) = self.save_point {
                    if *sp > 0 {
                        *sp -= 1;
                    } else {
                        self.save_point = None;
                    }
                }
            }
        }

        // Enforce memory limit
        while self.memory_usage > self.config.max_memory && !self.undo_stack.is_empty() {
            if let Some(cmd) = self.undo_stack.pop_front() {
                self.memory_usage = self.memory_usage.saturating_sub(cmd.memory_size());

                if let Some(ref mut sp) = self.save_point {
                    if *sp > 0 {
                        *sp -= 1;
                    } else {
                        self.save_point = None;
                    }
                }
            }
        }
    }

    fn emit_event(&self, event: HistoryEvent) {
        for listener in &self.listeners {
            listener(&event);
        }
    }
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestState {
        value: i32,
    }

    struct SetValueCommand {
        old_value: i32,
        new_value: i32,
    }

    impl Command for SetValueCommand {
        fn execute(&mut self, state: &mut dyn Any) -> CommandResult {
            let state = state
                .downcast_mut::<TestState>()
                .ok_or_else(|| CommandError::InvalidState("Expected TestState".to_string()))?;
            self.old_value = state.value;
            state.value = self.new_value;
            Ok(())
        }

        fn undo(&mut self, state: &mut dyn Any) -> CommandResult {
            let state = state
                .downcast_mut::<TestState>()
                .ok_or_else(|| CommandError::InvalidState("Expected TestState".to_string()))?;
            state.value = self.old_value;
            Ok(())
        }

        fn description(&self) -> &str {
            "Set value"
        }
    }

    #[test]
    fn test_execute_and_undo() {
        let mut history = CommandHistory::new();
        let mut state = TestState { value: 0 };

        history
            .execute(
                Box::new(SetValueCommand {
                    old_value: 0,
                    new_value: 42,
                }),
                &mut state,
            )
            .unwrap();

        assert_eq!(state.value, 42);
        assert!(history.can_undo());
        assert!(!history.can_redo());

        history.undo(&mut state).unwrap();

        assert_eq!(state.value, 0);
        assert!(!history.can_undo());
        assert!(history.can_redo());
    }

    #[test]
    fn test_redo() {
        let mut history = CommandHistory::new();
        let mut state = TestState { value: 0 };

        history
            .execute(
                Box::new(SetValueCommand {
                    old_value: 0,
                    new_value: 42,
                }),
                &mut state,
            )
            .unwrap();

        history.undo(&mut state).unwrap();
        history.redo(&mut state).unwrap();

        assert_eq!(state.value, 42);
    }

    #[test]
    fn test_save_point() {
        let mut history = CommandHistory::new();
        let mut state = TestState { value: 0 };

        history.set_save_point();
        assert!(history.is_at_save_point());

        history
            .execute(
                Box::new(SetValueCommand {
                    old_value: 0,
                    new_value: 42,
                }),
                &mut state,
            )
            .unwrap();

        assert!(!history.is_at_save_point());
        assert!(history.has_unsaved_changes());

        history.undo(&mut state).unwrap();
        assert!(history.is_at_save_point());
    }
}
